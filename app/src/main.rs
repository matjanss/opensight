// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent;
mod ble;
mod calwizard;
mod card;
mod count;
mod eventlog;
mod photo;
mod protocol;
mod resolve;
mod upload;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Color32, CornerRadius, FontFamily, FontId, Layout, RichText, Stroke, TextStyle, Theme,
    ThemePreference, Ui, Vec2,
};
use tokio::sync::mpsc::UnboundedSender;

use agent::PasskeyReply;
use ble::{Command, Event, SensorInfo};
use calwizard::{Action, FieldCheck, Phase, Wizard};
use card::{Card, CardTexture, Higher, Score};
use eventlog::EventLog;
use resolve::{How, Key};

const CONTENT_MAX_WIDTH: f32 = 460.0;
/// Width of the column next to the target card; below this window width they stack.
const SIDE_WIDTH: f32 = 280.0;
/// The side column with a card photo: the series table's six columns.
const SIDE_WIDTH_PHOTO: f32 = 440.0;
const STACK_BELOW: f32 = 640.0;
/// Least height of the last shot and series cards below the target card when stacked.
const STACKED_SERIES_MIN: f32 = 360.0;
/// The newest shot on the card. The only color in the UI: it has to stand out on both the
/// black and the white of the card.
const LAST_SHOT_COLOR: Color32 = Color32::from_rgb(232, 96, 52);
/// After a failed automatic reconnect, wait this long before trying again.
const AUTO_RETRY: Duration = Duration::from_secs(10);
const KEY_REMEMBERED: &str = "remembered_sensor";
const KEY_DARK: &str = "dark_mode";
const KEY_DECIMAL: &str = "decimal_scoring";
const KEY_ZOOM_CONTROLS: &str = "zoom_controls";
const KEY_REPLAY_CONTROLS: &str = "replay_controls";
const KEY_CLOSE_CALLS: &str = "close_calls";
const KEY_CLOSE_CALL_MM: &str = "close_call_mm";
const KEY_REPLAY_DELAY: &str = "replay_delay_s";
const KEY_SAVE_EVENTS: &str = "save_events";

/// The window and taskbar icon (assets/icon/). None if the PNG can't be read: the
/// window then gets the system's default icon.
fn app_icon() -> Option<Arc<egui::IconData>> {
    static ICON: OnceLock<Option<Arc<egui::IconData>>> = OnceLock::new();
    ICON.get_or_init(|| {
        eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon/png/opensights-icon-256.png"))
            .ok()
            .map(Arc::new)
    })
    .clone()
}

/// The app's name for the system: eframe's storage directory, and on Linux the window's
/// app id (Wayland) and class (X11). Wayland ignores window icons: the dock takes the icon
/// from the .desktop file of the same name (linux/opensights-app.desktop).
const APP_ID: &str = "opensights-app";

/// A viewport with the app id and the app icon, if there is one.
fn with_app_icon(builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
    let builder = builder.with_app_id(APP_ID);
    match app_icon() {
        Some(icon) => builder.with_icon(icon),
        None => builder,
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: with_app_icon(
            egui::ViewportBuilder::default()
                .with_inner_size([900.0, 640.0])
                .with_min_inner_size([360.0, 420.0])
                .with_title("OpenSights"),
        ),
        ..Default::default()
    };
    eframe::run_native(
        APP_ID,
        options,
        Box::new(|cc| {
            install_fonts(&cc.egui_ctx);
            install_style(&cc.egui_ctx);
            let app = SensorApp::new(cc);
            cc.egui_ctx.set_theme(theme_pref(app.dark_mode));
            Ok(Box::new(app))
        }),
    )
}

/// The PIN popup while pairing: BlueZ waits for `reply`.
struct PasskeyPrompt {
    reply: PasskeyReply,
    input: String,
    focus: bool,
}

const ZOOM_MIN: f32 = 1.0;
const ZOOM_MAX: f32 = 16.0;
/// Where the zoom buttons step to; the wheel zooms smoothly in between.
const ZOOM_STEPS: [f32; 9] = [1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0];

/// How the target card is zoomed and moved.
#[derive(Clone, Copy, PartialEq, Default)]
struct CardView {
    /// 1 = the whole card fits.
    zoom: f32,
    /// The card point (mm from the top left) at the middle of the view.
    centre_mm: Vec2,
}

impl CardView {
    /// Keeps the zoom in range and the card covering the whole view.
    fn clamp(&mut self, card_mm: Vec2) {
        self.zoom = self.zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        let half = card_mm / (2.0 * self.zoom);
        self.centre_mm = self.centre_mm.clamp(half, card_mm - half);
    }

    /// Zooms by `factor`, keeping the card point at `anchor` (points from the view's
    /// middle) in place. `base`: points per mm at 100%.
    fn zoom_by(&mut self, factor: f32, anchor: Vec2, base: f32, card_mm: Vec2) {
        let at = self.centre_mm + anchor / (base * self.zoom);
        self.zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        self.centre_mm = at - anchor / (base * self.zoom);
        self.clamp(card_mm);
    }

    /// Moves the card along with a drag of `delta` points.
    fn pan(&mut self, delta: Vec2, base: f32, card_mm: Vec2) {
        self.centre_mm -= delta / (base * self.zoom);
        self.clamp(card_mm);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Icon {
    ZoomOut,
    ZoomIn,
    Previous,
    Slower,
    Play,
    Pause,
    Stop,
    Faster,
    Next,
}

/// A round icon button for the bars over the card.
fn icon_button(
    ui: &Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    icon: Icon,
    enabled: bool,
    hover: &str,
) -> egui::Response {
    let id = ui.scope_id().with(("icon", icon));
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let response = ui.interact(rect, id, sense);
    let visuals = ui.visuals();
    let color = if !enabled {
        visuals.weak_text_color()
    } else {
        visuals.widgets.style(&response).fg_stroke.color
    };
    if enabled && response.hovered() {
        painter.circle_filled(
            rect.center(),
            rect.width() / 2.0,
            visuals.widgets.hovered.weak_bg_fill,
        );
    }
    paint_icon(painter, rect.center(), icon, color);
    response.on_hover_text(hover)
}

/// Draws `icon` about 14 points wide around `c`.
fn paint_icon(painter: &egui::Painter, c: egui::Pos2, icon: Icon, color: Color32) {
    use egui::pos2;
    let stroke = Stroke::new(1.5, color);
    // A filled triangle pointing from `base_x` to `tip_x`, `h` high above and below the middle.
    let tri = |tip_x: f32, base_x: f32, h: f32| {
        let (top, bottom) = (pos2(base_x, c.y - h), pos2(base_x, c.y + h));
        let tip = pos2(tip_x, c.y);
        // Clockwise on screen, for egui's anti-aliasing.
        let points = if tip_x > base_x {
            vec![top, tip, bottom]
        } else {
            vec![bottom, tip, top]
        };
        painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
    };
    let bar = |x0: f32, x1: f32| {
        painter.rect_filled(
            egui::Rect::from_min_max(pos2(x0, c.y - 6.0), pos2(x1, c.y + 6.0)),
            0.0,
            color,
        );
    };
    match icon {
        Icon::ZoomOut | Icon::ZoomIn => {
            let c = c - egui::vec2(1.5, 1.5); // the lens; the handle goes down right
            let r = 6.5;
            painter.circle_stroke(c, r, stroke);
            let d = egui::vec2(1.0, 1.0).normalized();
            painter.line_segment([c + d * r, c + d * (r + 5.0)], Stroke::new(2.0, color));
            painter.line_segment([c - egui::vec2(3.0, 0.0), c + egui::vec2(3.0, 0.0)], stroke);
            if icon == Icon::ZoomIn {
                painter.line_segment([c - egui::vec2(0.0, 3.0), c + egui::vec2(0.0, 3.0)], stroke);
            }
        }
        Icon::Previous => {
            bar(c.x - 6.0, c.x - 4.0);
            tri(c.x - 4.0, c.x + 5.0, 6.0);
        }
        Icon::Next => {
            bar(c.x + 4.0, c.x + 6.0);
            tri(c.x + 4.0, c.x - 5.0, 6.0);
        }
        Icon::Slower => {
            tri(c.x - 7.0, c.x, 5.0);
            tri(c.x, c.x + 7.0, 5.0);
        }
        Icon::Faster => {
            tri(c.x + 7.0, c.x, 5.0);
            tri(c.x, c.x - 7.0, 5.0);
        }
        Icon::Play => tri(c.x + 5.5, c.x - 4.5, 6.5),
        Icon::Pause => {
            bar(c.x - 5.0, c.x - 1.5);
            bar(c.x + 1.5, c.x + 5.0);
        }
        Icon::Stop => {
            painter.rect_filled(
                egui::Rect::from_center_size(c, egui::vec2(10.0, 10.0)),
                1.0,
                color,
            );
        }
    }
}

/// Seconds between shots in a replay, slowest first.
/// The replay delay's range in seconds (the default-speed slider and the speed buttons).
const REPLAY_DELAY_MIN: f32 = 0.1;
const REPLAY_DELAY_MAX: f32 = 5.0;
/// Where the slower/faster buttons step to, slowest first.
const REPLAY_STEPS: [f32; 12] = [5.0, 4.0, 3.0, 2.0, 1.5, 1.0, 0.75, 0.5, 0.35, 0.25, 0.15, 0.1];

/// The animal shown with a replay delay, snail to rabbit (monochrome Noto Emoji, part of
/// egui's fonts).
fn replay_animal(delay_s: f32) -> &'static str {
    match delay_s {
        d if d >= 2.5 => "🐌",
        d if d >= 1.0 => "🐢",
        d if d >= 0.4 => "🐕",
        _ => "🐇",
    }
}

/// A delay as "0.25", "1.5" or "2": up to two decimals, no trailing zeros.
fn format_delay(delay_s: f32) -> String {
    format!("{}", (delay_s * 100.0).round() / 100.0)
}

/// The close-call margin's range in mm, and its default: a shot this close to a higher
/// score is flagged for manual review.
const CLOSE_CALL_MIN_MM: f32 = 0.1;
const CLOSE_CALL_MAX_MM: f32 = 2.0;
const CLOSE_CALL_DEFAULT_MM: f32 = 0.5;

/// What a shot needing review just missed, e.g. "0.32 mm from ring 10".
fn review_detail(h: Higher) -> String {
    if h.inner_ten {
        format!("{:.2} mm from an inner ten", h.gap_mm)
    } else {
        format!("{:.2} mm from ring {}", h.gap_mm, h.ring)
    }
}

/// At most this many close calls are listed on the notice; the rest are counted.
const REVIEW_MAX_LINES: usize = 8;

/// A notice at the top left of the card, `top` points below the card's top: a title, then
/// one line per item (`lines`, in shot order). The close calls, and the hits with no score.
/// With `hover`, a line can be clicked. Returns the clicked line, and the space the notice
/// takes from the card's top (for the next one).
#[allow(clippy::too_many_arguments)]
fn review_notice(
    ui: &Ui,
    painter: &egui::Painter,
    card_rect: egui::Rect,
    title: &str,
    top: f32,
    salt: &str,
    lines: &[String],
    hover: Option<&str>,
) -> (Option<usize>, f32) {
    let visuals = ui.visuals();
    let small = TextStyle::Small.resolve(ui.style());
    let title = painter.layout_no_wrap(
        title.into(),
        TextStyle::Body.resolve(ui.style()),
        visuals.strong_text_color(),
    );
    let mut galleys: Vec<_> = lines
        .iter()
        .take(REVIEW_MAX_LINES)
        .map(|l| painter.layout_no_wrap(l.clone(), small.clone(), visuals.text_color()))
        .collect();
    let more = lines.len().saturating_sub(REVIEW_MAX_LINES);
    let more = (more > 0)
        .then(|| painter.layout_no_wrap(format!("and {more} more"), small, visuals.weak_text_color()));

    let (pad, icon, gap, line_gap) = (8.0, 22.0, 8.0, 2.0);
    let text_w = galleys
        .iter()
        .chain(&more)
        .map(|g| g.size().x)
        .fold(title.size().x, f32::max);
    let rows_h: f32 = galleys.iter().chain(&more).map(|g| g.size().y + line_gap).sum();
    let size = egui::vec2(
        pad + icon + gap + text_w + pad + 4.0,
        pad + (title.size().y + 4.0 + rows_h).max(icon) + pad,
    );
    let rect = egui::Rect::from_min_size(card_rect.min + egui::vec2(10.0, 10.0 + top), size);
    // Clicks on the notice mustn't reach the card (and close the shot popup).
    ui.interact(rect, ui.scope_id().with((salt, "notice")), egui::Sense::click());
    painter.rect(
        rect,
        CornerRadius::same(10),
        visuals.window_fill,
        visuals.window_stroke,
        egui::StrokeKind::Inside,
    );

    // An exclamation mark in a filled circle, beside the title.
    let c = egui::pos2(
        rect.left() + pad + icon / 2.0,
        rect.top() + pad + title.size().y.max(icon) / 2.0,
    );
    painter.circle_filled(c, icon / 2.0, visuals.strong_text_color());
    let mark = visuals.window_fill;
    painter.line_segment(
        [c - egui::vec2(0.0, 6.0), c + egui::vec2(0.0, 2.0)],
        Stroke::new(2.5, mark),
    );
    painter.circle_filled(c + egui::vec2(0.0, 5.5), 1.5, mark);

    let x = c.x + icon / 2.0 + gap;
    let mut y = c.y - title.size().y / 2.0;
    let title_h = title.size().y;
    painter.galley(egui::pos2(x, y), title, Color32::PLACEHOLDER);
    y += title_h + 4.0;
    let mut clicked = None;
    for (i, g) in galleys.drain(..).enumerate() {
        let row = egui::Rect::from_min_size(
            egui::pos2(x - 4.0, y - 1.0),
            egui::vec2(text_w + 8.0, g.size().y + 2.0),
        );
        if let Some(hover) = hover {
            let response = ui.interact(row, ui.scope_id().with((salt, i)), egui::Sense::click());
            if response.hovered() {
                painter.rect_filled(row, 4.0, visuals.widgets.hovered.weak_bg_fill);
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.on_hover_text(hover).clicked() {
                clicked = Some(i);
            }
        }
        let h = g.size().y;
        painter.galley(egui::pos2(x, y), g, Color32::PLACEHOLDER);
        y += h + line_gap;
    }
    if let Some(g) = more {
        painter.galley(egui::pos2(x, y), g, Color32::PLACEHOLDER);
    }
    (clicked, top + size.y + 8.0)
}

/// Replaying the series on the card: the shots appear again one by one.
struct Replay {
    /// How many of the series' shots are on the card; `None` = all, no replay.
    shown: Option<usize>,
    playing: bool,
    next_at: Instant,
    /// Seconds between shots.
    delay_s: f32,
}

impl Default for Replay {
    fn default() -> Self {
        Self {
            shown: None,
            playing: false,
            next_at: Instant::now(),
            delay_s: 1.0,
        }
    }
}

impl Replay {
    fn visible(&self, len: usize) -> usize {
        self.shown.map_or(len, |n| n.min(len))
    }

    fn delay(&self) -> Duration {
        Duration::from_secs_f32(self.delay_s)
    }

    /// Plays from where it is, or from an empty card when all shots show; pauses if playing.
    fn play_pause(&mut self, len: usize, now: Instant) {
        if self.playing {
            self.playing = false;
            return;
        }
        if self.visible(len) >= len {
            self.shown = Some(0);
        }
        self.playing = true;
        self.next_at = now + self.delay();
    }

    fn stop(&mut self) {
        self.shown = None;
        self.playing = false;
    }

    fn previous(&mut self, len: usize) {
        self.shown = Some(self.visible(len).saturating_sub(1));
        self.playing = false;
    }

    fn next(&mut self, len: usize) {
        self.playing = false;
        if let Some(n) = self.shown {
            self.shown = (n + 1 < len).then_some(n + 1);
        }
    }

    /// `faster`: one step faster, else slower. A shot due later than the new delay comes sooner.
    fn change_speed(&mut self, faster: bool, now: Instant) {
        let step = if faster {
            REPLAY_STEPS.iter().find(|&&d| d < self.delay_s * 0.99)
        } else {
            REPLAY_STEPS.iter().rev().find(|&&d| d > self.delay_s * 1.01)
        };
        if let Some(&d) = step {
            self.delay_s = d;
        }
        self.next_at = self.next_at.min(now + self.delay());
    }

    /// Shows the next shot when it's due; the replay ends once all show. Returns when the
    /// next one is due.
    fn tick(&mut self, len: usize, now: Instant) -> Option<Instant> {
        if !self.playing {
            return None;
        }
        if now >= self.next_at {
            let n = self.visible(len) + 1;
            self.next_at = now + self.delay();
            if n >= len {
                self.stop();
                return None;
            }
            self.shown = Some(n);
        }
        Some(self.next_at)
    }
}

/// Size of the bars' round buttons and the bars' padding, in points.
const BAR_BUTTON: f32 = 28.0;
const BAR_PAD: f32 = 4.0;
const ZOOM_LABEL: f32 = 56.0;
/// The replay bar's speed label, e.g. "🐇 0.25s".
const REPLAY_LABEL: f32 = 72.0;

/// The rounded background of a bar of buttons over the card.
fn control_bar(ui: &Ui, painter: &egui::Painter, bar: egui::Rect, salt: &str) {
    // Clicks between the buttons mustn't reach the card (and close the shot popup).
    ui.interact(bar, ui.scope_id().with(salt), egui::Sense::click());
    let visuals = ui.visuals();
    painter.rect(
        bar,
        CornerRadius::same((bar.height() / 2.0) as u8),
        visuals.window_fill,
        visuals.window_stroke,
        egui::StrokeKind::Inside,
    );
}

/// A slot `width` wide, `x` points from the start of `bar`'s padding.
fn bar_slot(bar: egui::Rect, x: f32, width: f32) -> egui::Rect {
    egui::Rect::from_min_size(
        bar.min + egui::vec2(BAR_PAD + x, BAR_PAD),
        egui::vec2(width, BAR_BUTTON),
    )
}

/// Zoom out · percentage · zoom in. Clicking the percentage goes back to 100%. Returns the
/// new zoom when one was clicked.
fn zoom_controls(ui: &Ui, painter: &egui::Painter, bar: egui::Rect, zoom: f32) -> Option<f32> {
    control_bar(ui, painter, bar, "zoom_bar");
    let out_rect = bar_slot(bar, 0.0, BAR_BUTTON);
    let label_rect = bar_slot(bar, BAR_BUTTON, ZOOM_LABEL);
    let in_rect = bar_slot(bar, BAR_BUTTON + ZOOM_LABEL, BAR_BUTTON);

    let zoom_out = icon_button(ui, painter, out_rect, Icon::ZoomOut, zoom > ZOOM_MIN, "Zoom out");
    let zoom_in = icon_button(ui, painter, in_rect, Icon::ZoomIn, zoom < ZOOM_MAX, "Zoom in");
    let reset = ui.interact(label_rect, ui.scope_id().with("zoom_reset"), egui::Sense::click());
    let text_color = ui.visuals().widgets.style(&reset).text_color();
    painter.text(
        label_rect.center(),
        egui::Align2::CENTER_CENTER,
        format!("{:.0}%", zoom * 100.0),
        TextStyle::Body.resolve(ui.style()),
        text_color,
    );
    let reset = reset.on_hover_text("Back to 100%");

    if zoom_out.clicked() {
        ZOOM_STEPS.iter().rev().find(|&&z| z < zoom / 1.01).copied()
    } else if zoom_in.clicked() {
        ZOOM_STEPS.iter().find(|&&z| z > zoom * 1.01).copied()
    } else if reset.clicked() {
        Some(1.0)
    } else {
        None
    }
}

/// Previous · slower · play/pause · stop · faster · next, for a series of `len` shots.
/// Returns whether something was clicked.
fn replay_controls(
    ui: &Ui,
    painter: &egui::Painter,
    bar: egui::Rect,
    replay: &mut Replay,
    len: usize,
    now: Instant,
) -> bool {
    control_bar(ui, painter, bar, "replay_bar");
    let slot = |i: usize| bar_slot(bar, i as f32 * BAR_BUTTON, BAR_BUTTON);
    let every = format!("now a shot every {} s", format_delay(replay.delay_s));
    let replaying = replay.shown.is_some();
    let (play_icon, play_hover) = if replay.playing {
        (Icon::Pause, "Pause")
    } else if replaying {
        (Icon::Play, "Continue the replay")
    } else {
        (Icon::Play, "Replay the series")
    };
    let buttons = [
        icon_button(
            ui,
            painter,
            slot(0),
            Icon::Previous,
            replay.visible(len) > 0,
            "Previous shot",
        ),
        icon_button(
            ui,
            painter,
            slot(1),
            Icon::Slower,
            replay.delay_s < REPLAY_DELAY_MAX * 0.99,
            &format!("Slower ({every})"),
        ),
        icon_button(ui, painter, slot(2), play_icon, len > 0, play_hover),
        icon_button(
            ui,
            painter,
            slot(3),
            Icon::Stop,
            replaying,
            "Stop, show all shots",
        ),
        icon_button(
            ui,
            painter,
            slot(4),
            Icon::Faster,
            replay.delay_s > REPLAY_DELAY_MIN * 1.01,
            &format!("Faster ({every})"),
        ),
        icon_button(ui, painter, slot(5), Icon::Next, replaying, "Next shot"),
    ];
    let Some(clicked) = buttons.iter().position(|b| b.clicked()) else {
        return false;
    };
    match clicked {
        0 => replay.previous(len),
        1 => replay.change_speed(false, now),
        2 => replay.play_pause(len, now),
        3 => replay.stop(),
        4 => replay.change_speed(true, now),
        _ => replay.next(len),
    }
    true
}

/// The replay speed at the right end of the replay bar: an animal and the delay.
fn replay_speed_label(ui: &Ui, painter: &egui::Painter, bar: egui::Rect, delay_s: f32) {
    let rect = bar_slot(bar, 6.0 * BAR_BUTTON, REPLAY_LABEL);
    let delay = format_delay(delay_s);
    ui.interact(rect, ui.scope_id().with("replay_speed"), egui::Sense::hover())
        .on_hover_text(format!("A shot every {delay} s"));
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        format!("{} {delay}s", replay_animal(delay_s)),
        TextStyle::Body.resolve(ui.style()),
        ui.visuals().text_color(),
    );
}

enum Link {
    Idle,
    Connecting { name: String },
    Connected { id: String },
}

struct SensorApp {
    cmd: UnboundedSender<Command>,
    events: mpsc::Receiver<Event>,

    link: Link,
    sensors: Vec<SensorInfo>,
    heartbeat: Option<u32>,
    heartbeat_at: Option<Instant>,
    sensor_name: String,
    name_input: String,
    pin_supported: bool,
    pin_input: String,
    /// The user pressed "Change…" and has to confirm (it unpairs other computers).
    pin_confirm: bool,
    passkey_prompt: Option<PasskeyPrompt>,

    card: Card,
    card_texture: CardTexture,
    shots_supported: bool,
    /// The shots of the current series, oldest first. Kept across reconnects to the same
    /// sensor; cleared by the user or when another sensor connects.
    series: Vec<(protocol::Shot, Score)>,
    /// The series' hits with no position (a hole, a missed shot), each with how many scored
    /// shots came before it: listed in their place, not scored until resolved by hand.
    unscored: Vec<(usize, protocol::Shot)>,
    /// The hits resolved by hand (`resolve`), now in `series`: how, by (boot, seq).
    manual: HashMap<Key, How>,
    /// Resolving a hit by hand, while its window is open.
    resolving: Option<Resolving>,
    /// Where the card was drawn last: the resolve window opens beside it.
    card_rect: egui::Rect,
    series_sensor: Option<String>,
    /// Protocol v3: the sensor's state, its calibration and settings, and what its events
    /// said since connecting (read-only here; the calibration wizard comes in step 5d).
    status: Option<protocol::Status>,
    config: Option<(protocol::Calibration, protocol::Settings)>,
    event_counts: EventCounts,
    /// Every event written to a log file (`eventlog`), as the firmware's serial log lines.
    save_events: bool,
    event_log: Option<EventLog>,
    log_error: Option<String>,
    /// The calibration window: the wizard or the field check (step 5d), while open.
    cal: Option<CalTask>,
    /// Where the last calibration report was saved, or why it wasn't.
    cal_report: Option<String>,
    /// The card photo window (`CARDPHOTO.md` C3/C4), while open: the link for the phone.
    photo_task: Option<PhotoTask>,
    /// A photo being processed, and the sensor's shots it was taken with.
    photo_job: Option<PhotoJobRx>,
    /// The processed photo of the current series: under the shots, and its ground truth.
    card_photo: Option<CardPhoto>,
    photo_error: Option<String>,
    /// The count window (`CARDPHOTO.md` C7), while open: cards counted apart from the series.
    count: Option<count::CountTask>,
    /// The card view: 0 the printed card, 1 the photo.
    photo_fade: f32,
    /// (boot, seq) of the shot whose info popup is open on the card.
    selected_shot: Option<(u32, u32)>,
    view: CardView,
    replay: Replay,
    show_zoom_controls: bool,
    show_replay_controls: bool,
    show_close_calls: bool,
    /// A shot this close (mm) to a higher score is a close call.
    close_call_mm: f32,
    /// Seconds between shots when a replay starts; the bar's buttons change it from there.
    replay_delay_s: f32,

    /// The sensor this app connected to last; reconnected automatically when seen.
    remembered: Option<String>,
    /// Off after the user disconnects, so we don't reconnect behind their back.
    auto_connect: bool,
    auto_retry_at: Instant,

    message: Option<String>,
    fatal: Option<String>,
    dark_mode: bool,
    /// Decimal scoring (10.4) instead of traditional whole rings with X. Off by default.
    decimal_scoring: bool,
    settings_open: bool,
    /// `OPENSIGHTS_DEMO_HITS` set: the card with a synthetic series (shots, holes and a
    /// missed shot) without a sensor, to try resolving hits by hand.
    demo_hits: bool,
}

impl SensorApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::with_link(cc, ble::spawn(cc.egui_ctx.clone()))
    }

    /// With the BLE thread's channels (the UI tests pass channels with no thread).
    fn with_link(
        cc: &eframe::CreationContext<'_>,
        (cmd, events): (UnboundedSender<Command>, mpsc::Receiver<Event>),
    ) -> Self {
        let storage = cc.storage;
        let replay_delay_s = storage
            .and_then(|s| s.get_string(KEY_REPLAY_DELAY))
            .and_then(|v| v.parse().ok())
            .map_or(1.0, |v: f32| v.clamp(REPLAY_DELAY_MIN, REPLAY_DELAY_MAX));
        Self {
            cmd,
            events,
            link: Link::Idle,
            sensors: Vec::new(),
            heartbeat: None,
            heartbeat_at: None,
            sensor_name: String::new(),
            name_input: String::new(),
            pin_supported: false,
            pin_input: String::new(),
            pin_confirm: false,
            passkey_prompt: None,
            card: card::air_pistol(),
            card_texture: CardTexture::default(),
            shots_supported: false,
            series: Vec::new(),
            unscored: Vec::new(),
            manual: HashMap::new(),
            resolving: None,
            card_rect: egui::Rect::NOTHING,
            series_sensor: None,
            status: None,
            config: None,
            event_counts: EventCounts::default(),
            save_events: storage
                .and_then(|s| s.get_string(KEY_SAVE_EVENTS))
                .is_none_or(|v| v == "true"),
            event_log: eframe::storage_dir(APP_ID).map(|d| EventLog::new(d.join("logs"))),
            log_error: None,
            cal: None,
            cal_report: None,
            photo_task: None,
            photo_job: None,
            card_photo: None,
            photo_error: None,
            count: None,
            photo_fade: 0.0,
            selected_shot: None,
            view: CardView::default(),
            replay: Replay {
                delay_s: replay_delay_s,
                ..Replay::default()
            },
            show_zoom_controls: storage
                .and_then(|s| s.get_string(KEY_ZOOM_CONTROLS))
                .is_none_or(|v| v == "true"),
            show_replay_controls: storage
                .and_then(|s| s.get_string(KEY_REPLAY_CONTROLS))
                .is_none_or(|v| v == "true"),
            show_close_calls: storage
                .and_then(|s| s.get_string(KEY_CLOSE_CALLS))
                .is_none_or(|v| v == "true"),
            close_call_mm: storage
                .and_then(|s| s.get_string(KEY_CLOSE_CALL_MM))
                .and_then(|v| v.parse().ok())
                .map_or(CLOSE_CALL_DEFAULT_MM, |v: f32| {
                    v.clamp(CLOSE_CALL_MIN_MM, CLOSE_CALL_MAX_MM)
                }),
            replay_delay_s,
            remembered: storage
                .and_then(|s| s.get_string(KEY_REMEMBERED))
                .filter(|s| !s.is_empty()),
            auto_connect: true,
            auto_retry_at: Instant::now(),
            message: None,
            fatal: None,
            dark_mode: storage
                .and_then(|s| s.get_string(KEY_DARK))
                .is_none_or(|v| v == "true"),
            decimal_scoring: storage
                .and_then(|s| s.get_string(KEY_DECIMAL))
                .is_some_and(|v| v == "true"),
            settings_open: false,
            demo_hits: false,
        }
        .with_demo_hits()
    }

    fn with_demo_hits(self) -> Self {
        if std::env::var_os("OPENSIGHTS_DEMO_HITS").is_none() {
            return self;
        }
        self.demo_series()
    }

    /// The demo series (`OPENSIGHTS_DEMO_HITS`, the UI tests), as the sensor would send it:
    /// shots 1, 2, a hole, shots 3, 4, a missed shot, shot 5, a hole, shot 6.
    fn demo_series(mut self) -> Self {
        self.demo_hits = true;
        self.series_sensor = Some("demo".into());
        let hits: [(protocol::Kind, f32, f32); 9] = [
            (protocol::Kind::Shot, 3.1, -4.2),
            (protocol::Kind::Shot, -8.5, 2.0),
            (protocol::Kind::Hole, 0.0, 0.0),
            (protocol::Kind::Shot, 1.2, 0.8),
            (protocol::Kind::Shot, 14.0, 21.5),
            (protocol::Kind::Missed, 0.0, 0.0),
            (protocol::Kind::Shot, -2.4, -6.3),
            (protocol::Kind::Hole, 0.0, 0.0),
            (protocol::Kind::Shot, 5.5, 5.0),
        ];
        for (i, (kind, x, y)) in hits.into_iter().enumerate() {
            let shot = protocol::Shot {
                seq: i as u32 + 1,
                boot: 0xDE30,
                time_ms: 1000 * i as u32,
                x: (x * 100.0) as i32,
                y: (y * 100.0) as i32,
                event: 0,
                cal: 0,
                sigma: protocol::SIGMA_UNKNOWN,
                kind,
                flags: 0,
            };
            if shot.scored() {
                let score = self.card.score(shot.x_mm(), shot.y_mm());
                self.series.push((shot, score));
            } else {
                self.unscored.push((self.series.len(), shot));
            }
        }
        self
    }

    fn send(&self, c: Command) {
        let _ = self.cmd.send(c); // fails only if the BLE thread died, which shows as `fatal`
    }

    fn clear_series(&mut self) {
        self.series.clear();
        self.unscored.clear();
        self.manual.clear();
        self.resolving = None;
        self.card_photo = None;
    }

    /// The event log's own lines (`manual`).
    fn log_line(&mut self, text: &str) {
        if self.save_events
            && let Some(log) = &mut self.event_log
            && let Err(err) = log.line(text)
        {
            self.log_error = Some(format!("Could not write the event log: {err}"));
        }
    }

    /// Whether a shot of the series is on the card (not a miss counted by hand).
    fn on_card(&self, s: &protocol::Shot) -> bool {
        self.manual.get(&resolve::key(s)) != Some(&How::Miss)
    }

    /// Resolves the hit `key` (unscored, or resolved by hand before) as `how`; `pos` (mm)
    /// is where it was placed. It goes into the series in its place, and into the log.
    fn resolve_hit(&mut self, key: Key, how: How, pos: (f32, f32)) {
        let (x, y, score) = match how {
            How::Placed => {
                let (x, y) = ((pos.0 * 100.0).round() as i32, (pos.1 * 100.0).round() as i32);
                (x, y, self.card.score(x as f32 / 100.0, y as f32 / 100.0))
            }
            How::Same(other) => {
                let Some((s, score)) = self.series.iter().find(|(s, _)| resolve::key(s) == other) else {
                    return;
                };
                (s.x, s.y, *score)
            }
            How::Miss => (0, 0, resolve::MISS),
        };
        let i = if let Some(j) = self.unscored.iter().position(|(_, s)| resolve::key(s) == key) {
            let mut shot = self.unscored[j].1;
            (shot.x, shot.y) = (x, y);
            resolve::resolve(&mut self.series, &mut self.unscored, j, shot, score)
        } else if let Some(i) = self
            .series
            .iter()
            .position(|(s, _)| resolve::key(s) == key && !s.scored())
        {
            (self.series[i].0.x, self.series[i].0.y) = (x, y);
            self.series[i].1 = score;
            i
        } else {
            return;
        };
        self.manual.insert(key, how);
        let line = resolve::log_line(&self.series[i].0, Some(how), Some(i + 1), Some(score));
        self.log_line(&line);
    }

    /// Puts a hit resolved by hand back to "not scored".
    fn unresolve_hit(&mut self, key: Key) {
        let Some(i) = self
            .series
            .iter()
            .position(|(s, _)| resolve::key(s) == key && !s.scored())
        else {
            return;
        };
        let hit = resolve::unresolve(&mut self.series, &mut self.unscored, i);
        self.manual.remove(&key);
        if self.selected_shot == Some(key) {
            self.selected_shot = None;
        }
        self.log_line(&resolve::log_line(&hit, None, None, None));
    }

    /// The sensor's own shots in the series (the ones a card photo is checked against).
    fn measured_keys(&self) -> Vec<Key> {
        self.series
            .iter()
            .filter(|(s, _)| s.scored())
            .map(|(s, _)| resolve::key(s))
            .collect()
    }

    fn log_note(&mut self, text: &str) {
        if self.save_events
            && let Some(log) = &mut self.event_log
            && let Err(err) = log.note(text)
        {
            self.log_error = Some(format!("Could not write the event log: {err}"));
        }
    }

    fn connect(&mut self, id: &str) {
        self.message = None;
        self.auto_connect = true;
        self.auto_retry_at = Instant::now() + AUTO_RETRY;
        self.send(Command::Connect(id.to_owned()));
    }

    fn handle_events(&mut self) {
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                Event::Fatal(e) => self.fatal = Some(e),
                Event::Sensors(list) => self.sensors = list,
                Event::Connecting { name } => self.link = Link::Connecting { name },
                Event::Connected {
                    id,
                    name,
                    heartbeat,
                    pin_supported,
                    shots_supported,
                } => {
                    self.passkey_prompt = None;
                    self.pin_supported = pin_supported;
                    self.shots_supported = shots_supported;
                    if self.series_sensor.as_deref() != Some(id.as_str()) {
                        self.clear_series();
                        self.series_sensor = Some(id.clone());
                    }
                    self.event_counts = EventCounts::default();
                    self.status = None;
                    self.config = None;
                    self.cal = None;
                    self.log_note(&format!("connected to {name} ({id})"));
                    self.pin_input.clear();
                    self.pin_confirm = false;
                    self.remembered = Some(id.clone());
                    self.link = Link::Connected { id };
                    self.name_input = name.clone();
                    self.sensor_name = name;
                    self.heartbeat = heartbeat;
                    self.heartbeat_at = heartbeat.map(|_| Instant::now());
                    self.message = None;
                }
                Event::Disconnected { reason } => {
                    self.passkey_prompt = None;
                    if let Some(task) = &mut self.cal {
                        task.lost();
                    }
                    self.link = Link::Idle;
                    self.heartbeat = None;
                    self.heartbeat_at = None;
                    self.auto_retry_at = Instant::now() + AUTO_RETRY;
                    self.message = reason;
                }
                Event::Heartbeat(ms) => {
                    self.heartbeat = Some(ms);
                    self.heartbeat_at = Some(Instant::now());
                }
                Event::Shot(shot) if shot.scored() => {
                    let score = self.card.score(shot.x_mm(), shot.y_mm());
                    self.series.push((shot, score));
                }
                Event::Shot(shot) => self.unscored.push((self.series.len(), shot)),
                Event::Captured(e) => {
                    self.event_counts.add(&e);
                    match &mut self.cal {
                        Some(CalTask::Calibrate(w)) => {
                            w.on_event(&e);
                        }
                        Some(CalTask::Check(c)) => {
                            c.on_event(&e);
                        }
                        None => {}
                    }
                    if self.save_events
                        && let Some(log) = &mut self.event_log
                        && let Err(err) = log.event(&e)
                    {
                        self.log_error = Some(format!("Could not write the event log: {err}"));
                    }
                }
                Event::Status(st) => {
                    if let Some(CalTask::Calibrate(w)) = &mut self.cal {
                        w.on_status(&st);
                    }
                    self.status = Some(st);
                }
                Event::Config(cal, set) => {
                    if self.save_events
                        && let Some(log) = &mut self.event_log
                        && let Err(err) = log.config(&cal, &set)
                    {
                        self.log_error = Some(format!("Could not write the event log: {err}"));
                    }
                    self.config = Some((cal, set));
                }
                Event::Name(name) => {
                    self.name_input = name.clone();
                    self.sensor_name = name;
                }
                Event::PinChanged => {
                    self.pin_input.clear();
                    self.message = Some(
                        "PIN changed. Other computers that were paired have to pair again with the new PIN."
                            .into(),
                    );
                }
                Event::PasskeyRequest(reply) => {
                    self.passkey_prompt = Some(PasskeyPrompt {
                        reply,
                        input: String::new(),
                        focus: true,
                    });
                }
                Event::PasskeyCancelled => self.passkey_prompt = None,
                Event::Error(e) => self.message = Some(e),
            }
        }
        self.run_cal_actions();
    }

    /// Sends what the calibration wizard or the field check asks for.
    fn run_cal_actions(&mut self) {
        let actions = match &mut self.cal {
            Some(CalTask::Calibrate(w)) => w.take_actions(),
            Some(CalTask::Check(c)) => c.take_actions(),
            None => return,
        };
        for a in actions {
            self.send(match a {
                Action::CalMode(on) => Command::Control(protocol::encode_cal_mode(on).to_vec()),
                Action::Point(x, y) => Command::Control(protocol::encode_point(x, y).to_vec()),
                Action::NoPoint => Command::Control(protocol::encode_no_point().to_vec()),
                Action::WriteCalibration(c) => Command::WriteCalibration(c),
            });
        }
    }

    /// Saves the calibration report next to the event logs.
    fn save_cal_report(&mut self, text: &str, id: u32) {
        let Some(log) = &self.event_log else {
            self.cal_report = Some("No folder for the report on this system.".into());
            return;
        };
        let path = log
            .dir()
            .join(format!("calibration-{}-{id:08X}.txt", eventlog::utc_date()));
        self.cal_report = Some(
            match std::fs::create_dir_all(log.dir()).and_then(|_| std::fs::write(&path, text)) {
                Ok(()) => format!("Report saved: {}", path.display()),
                Err(e) => format!("Could not save the report: {e}"),
            },
        );
    }

    /// Whether this computer is paired with the sensor. Where the OS doesn't say,
    /// the sensor we connected to last is assumed to be.
    fn is_paired(&self, s: &SensorInfo) -> bool {
        s.paired
            .unwrap_or(self.remembered.as_deref() == Some(s.id.as_str()))
    }

    /// Reconnects to the remembered sensor when it shows up, if it is paired. Not while
    /// it is in pairing mode: the user may be pairing it with another computer.
    fn auto_connect(&mut self) {
        if !matches!(self.link, Link::Idle) || !self.auto_connect || self.fatal.is_some() {
            return;
        }
        let Some(id) = self.remembered.clone() else { return };
        let ready = self
            .sensors
            .iter()
            .any(|s| s.id == id && self.is_paired(s) && !s.adv.is_some_and(|a| a.pairing));
        if Instant::now() >= self.auto_retry_at && ready {
            let message = self.message.take();
            self.connect(&id);
            self.message = message; // keep showing why the last attempt failed
        }
    }
}

impl eframe::App for SensorApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.handle_events();
        self.auto_connect();
        let was_dark = self.dark_mode;

        egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(ui.visuals().window_fill)
                    .inner_margin(egui::Margin::symmetric(16, 8)),
            )
            .show(ui, |ui| self.status_bar(ui));

        // The demo also without Bluetooth (the UI tests on CI).
        let on_target = (matches!(self.link, Link::Connected { .. }) && self.fatal.is_none()) || self.demo_hits;
        egui::CentralPanel::default().show(ui, |ui| {
            if on_target {
                self.target_screen(ui);
                return;
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                centered_column(ui, |ui| {
                    ui.add_space(20.0);
                    title(ui, "OpenSights", "Electronic target prototype");
                    ui.add_space(14.0);
                    self.main_body(ui);
                    ui.add_space(16.0);
                });
            });
        });

        let ctx = ui.ctx().clone();
        if self.settings_open {
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("settings"),
                with_app_icon(
                    egui::ViewportBuilder::default()
                        .with_title("OpenSights · Settings")
                        .with_inner_size([460.0, 640.0])
                        .with_min_inner_size([360.0, 420.0]),
                ),
                |ui, _| {
                    if ui.input(|i| i.viewport().close_requested()) {
                        self.settings_open = false;
                    }
                    egui::CentralPanel::default().show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            centered_column(ui, |ui| {
                                ui.add_space(20.0);
                                title(ui, "Settings", "Appearance, scoring and the connected sensor");
                                ui.add_space(14.0);
                                self.settings_body(ui);
                                ui.add_space(16.0);
                            });
                        });
                    });
                    // Pairing is started from here, so the PIN prompt belongs here too.
                    self.passkey_modal(ui.ctx());
                },
            );
        } else {
            self.passkey_modal(&ctx);
        }

        if self.cal.is_some() {
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("calibration"),
                with_app_icon(
                    egui::ViewportBuilder::default()
                        .with_title("OpenSights · Calibration")
                        .with_inner_size([620.0, 720.0])
                        .with_min_inner_size([420.0, 480.0]),
                ),
                |ui, _| {
                    let close = ui.input(|i| i.viewport().close_requested());
                    egui::CentralPanel::default().show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            centered_column(ui, |ui| {
                                ui.add_space(20.0);
                                self.calibration_body(ui);
                                ui.add_space(16.0);
                            });
                        });
                    });
                    if close {
                        self.close_calibration();
                    }
                },
            );
            self.run_cal_actions();
        }

        self.poll_photo(&ctx);
        if self.photo_task.is_some() {
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("photo"),
                with_app_icon(
                    egui::ViewportBuilder::default()
                        .with_title("OpenSights · Card photo")
                        .with_inner_size([520.0, 780.0])
                        .with_min_inner_size([380.0, 480.0]),
                ),
                |ui, _| {
                    let close = ui.input(|i| i.viewport().close_requested());
                    egui::CentralPanel::default().show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            centered_column(ui, |ui| {
                                ui.add_space(20.0);
                                self.photo_body(ui);
                                ui.add_space(16.0);
                            });
                        });
                    });
                    if close {
                        self.photo_task = None; // stops the server
                    }
                },
            );
        }

        if let Some(task) = &mut self.count {
            let env = count::CountEnv {
                card: &self.card,
                decimal: self.decimal_scoring && self.card.decimal,
                zoom_controls: self.show_zoom_controls,
                close_call_mm: self.show_close_calls.then_some(self.close_call_mm),
                photos_dir: Self::photos_dir(),
            };
            task.poll(&env);
            let mut close = false;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("count"),
                with_app_icon(
                    egui::ViewportBuilder::default()
                        .with_title("OpenSights · Count cards")
                        .with_inner_size([1180.0, 800.0])
                        .with_min_inner_size([420.0, 480.0]),
                ),
                |ui, _| {
                    close = ui.input(|i| i.viewport().close_requested());
                    egui::CentralPanel::default().show(ui, |ui| {
                        ui.add_space(8.0);
                        task.ui(ui, &env);
                    });
                },
            );
            if close {
                self.count = None; // stops the server and the worker
            }
        }

        if self.dark_mode != was_dark {
            ctx.set_theme(theme_pref(self.dark_mode));
        }
        // Keeps "updated 3 s ago" ticking; BLE events repaint on their own.
        if matches!(self.link, Link::Connected { .. }) {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(KEY_REMEMBERED, self.remembered.clone().unwrap_or_default());
        storage.set_string(KEY_DARK, self.dark_mode.to_string());
        storage.set_string(KEY_DECIMAL, self.decimal_scoring.to_string());
        storage.set_string(KEY_ZOOM_CONTROLS, self.show_zoom_controls.to_string());
        storage.set_string(KEY_REPLAY_CONTROLS, self.show_replay_controls.to_string());
        storage.set_string(KEY_CLOSE_CALLS, self.show_close_calls.to_string());
        storage.set_string(KEY_CLOSE_CALL_MM, self.close_call_mm.to_string());
        storage.set_string(KEY_REPLAY_DELAY, self.replay_delay_s.to_string());
        storage.set_string(KEY_SAVE_EVENTS, self.save_events.to_string());
    }
}

// ---- UI sections ----

impl SensorApp {
    fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let (dot, text) = match &self.link {
                Link::Connected { .. } => ("●", format!("Connected: {}", self.sensor_name)),
                Link::Connecting { name } => ("◌", format!("Connecting to {name}…")),
                Link::Idle => ("○", "No sensor connected.".to_owned()),
            };
            ui.label(RichText::new(dot).strong());
            ui.label(text);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let gear = ui
                    .add(egui::Button::new(RichText::new("⚙").size(18.0)).frame(false))
                    .on_hover_text("Settings");
                if gear.clicked() {
                    self.open_settings(ui.ctx());
                }
                if count_button(ui).clicked() {
                    self.open_count(ui.ctx());
                }
                ui.label(
                    RichText::new(format!("Protocol v{}", protocol::PROTOCOL_VERSION))
                        .small()
                        .weak(),
                );
            });
        });
    }

    /// Opens the settings window, or brings it to the front if it is already open.
    fn open_settings(&mut self, ctx: &egui::Context) {
        if self.settings_open {
            ctx.send_viewport_cmd_to(
                egui::ViewportId::from_hash_of("settings"),
                egui::ViewportCommand::Focus,
            );
        }
        self.settings_open = true;
    }

    /// Shows the last message, in the settings window while it's open (that's where the
    /// user acts), otherwise in the main window.
    fn message_notice(&mut self, ui: &mut Ui, in_settings: bool) {
        if in_settings == self.settings_open
            && let Some(msg) = self.message.clone()
            && notice(ui, &msg)
        {
            self.message = None;
        }
    }

    /// Main window while connected: the target card with the shots, and the series beside
    /// it (below it in a narrow window).
    fn target_screen(&mut self, ui: &mut Ui) {
        ui.add_space(8.0);
        self.message_notice(ui, false);
        let avail = ui.available_size();
        let gap = 16.0;
        if avail.x >= STACK_BELOW {
            let aspect = self.card.size_mm().y / self.card.size_mm().x;
            // The card as big as the height allows; the side column takes the rest of the
            // width, and the series card in it the rest of the height.
            let side_min = if self.card_photo.is_some() { SIDE_WIDTH_PHOTO } else { SIDE_WIDTH };
            let card_w = (avail.x - side_min - gap).min(avail.y / aspect).max(160.0);
            let side_w = (avail.x - card_w - gap).max(side_min);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                self.target_card(ui, card_w);
                ui.vertical(|ui| {
                    ui.set_width(side_w);
                    ui.set_height(avail.y);
                    self.series_column(ui);
                });
            });
        } else {
            // Stacked: the series card fills the window below the card, or scrolls with it
            // when the window is short.
            egui::ScrollArea::vertical().show(ui, |ui| {
                let width = ui.available_width();
                self.target_card(ui, width);
                ui.add_space(6.0);
                let card_h = width * self.card.size_mm().y / self.card.size_mm().x;
                let rest = (avail.y - card_h - 6.0 - ui.spacing().item_spacing.y).max(STACKED_SERIES_MIN);
                ui.allocate_ui(egui::vec2(width, rest), |ui| {
                    ui.set_height(rest);
                    self.series_column(ui);
                });
            });
        }
        self.resolve_window(ui.ctx());
    }

    /// The resolve window: an unscored hit (a hole, a missed shot) placed on the card, the
    /// same hole as another shot, or a miss, after a look at the paper card; or a hit
    /// resolved by hand changed or put back to "not scored". Opened from the hit's row or
    /// the card's notice, or by selecting a shot resolved by hand.
    fn resolve_window(&mut self, ctx: &egui::Context) {
        if self
            .resolving
            .as_ref()
            .is_some_and(|r| r.from_selection && self.selected_shot != Some(r.key))
        {
            self.resolving = None; // the selection moved on
        }
        if self.resolving.is_none()
            && let Some(k) = self.selected_shot.filter(|k| self.manual.contains_key(k))
        {
            self.resolving = Some(Resolving::new(k, true));
        }
        let Some(key) = self.resolving.as_ref().map(|r| r.key) else {
            return;
        };
        let unscored = self.unscored.iter().find(|(_, s)| resolve::key(s) == key).copied();
        let in_series = self.series.iter().position(|(s, _)| resolve::key(s) == key);
        let how = self.manual.get(&key).copied();
        let (hit, title) = match (unscored, in_series) {
            (Some((after, s)), _) => (s, format!("{} – not scored", hit_name(&s, after))),
            (None, Some(i)) if how.is_some() => (
                self.series[i].0,
                format!("Shot {} – {}", i + 1, how.map_or("", how_label)),
            ),
            _ => {
                self.resolving = None; // gone (the series cleared), or the sensor's own shot
                return;
            }
        };
        let decimal = self.decimal_scoring && self.card.decimal;
        let number = |k: Key| self.series.iter().position(|(s, _)| resolve::key(s) == k).map(|i| i + 1);
        // The shots it can have gone through: on the card, not itself.
        let others: Vec<(usize, Key, String)> = self
            .series
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, (s, _))| resolve::key(s) != key && self.manual.get(&resolve::key(s)) != Some(&How::Miss))
            .map(|(i, (s, sc))| {
                let text = format!("Shot {} · {} · {:+.2}, {:+.2} mm", i + 1, sc.text(decimal), s.x_mm(), s.y_mm());
                (i, resolve::key(s), text)
            })
            .collect();

        enum Act {
            Stay,
            To(Step),
            Miss,
            Confirm,
            Undo,
            Close,
        }
        let mut act = Act::Stay;
        let mut open = true;
        let card = &self.card;
        let r = self.resolving.as_mut().unwrap();
        egui::Window::new(title)
            .id(egui::Id::unique("resolve_hit"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::RIGHT_TOP)
            .default_pos(self.card_rect.right_top() + egui::vec2(-12.0, 56.0))
            .default_width(330.0)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                let wrap = |ui: &mut Ui, t: &str| {
                    ui.add(egui::Label::new(t).wrap());
                };
                match r.step {
                    Step::Choose => {
                        match (how, unscored) {
                            (_, Some((after, s))) => {
                                wrap(ui, &unscored_line(after, &s));
                                wrap(ui, "Check the paper card, then:");
                            }
                            (Some(How::Placed), _) => {
                                let sc = card.score(hit.x_mm(), hit.y_mm());
                                wrap(ui, &format!(
                                    "Placed by hand at {:+.2}, {:+.2} mm: {}.",
                                    hit.x_mm(),
                                    hit.y_mm(),
                                    sc.text(decimal)
                                ));
                            }
                            (Some(How::Same(o)), _) => wrap(ui, &match number(o) {
                                Some(n) => format!("By hand: the same hole as shot {n}."),
                                None => "By hand: the same hole as an earlier shot.".to_string(),
                            }),
                            (Some(How::Miss), _) => wrap(ui, "Counted as a miss by hand: 0, not on the card."),
                            (None, None) => {}
                        }
                        let wide = |ui: &mut Ui, enabled: bool, t: &str| {
                            ui.add_enabled(enabled, egui::Button::new(t).min_size(egui::vec2(ui.available_width(), 0.0)))
                                .clicked()
                        };
                        if wide(ui, true, "Place it on the card…") {
                            act = Act::To(Step::Place);
                        }
                        if wide(ui, !others.is_empty(), "The same hole as another shot…") {
                            act = Act::To(Step::Same);
                        }
                        if wide(ui, how != Some(How::Miss), "A miss (0)") {
                            act = Act::Miss;
                        }
                        if how.is_some() && wide(ui, true, "Back to not scored") {
                            act = Act::Undo;
                        }
                    }
                    Step::Place => {
                        wrap(
                            ui,
                            "Click the hole on the card, then drag the shot until its ring sits on the \
                             hole's edge. The arrow keys move it 0.05 mm (Shift: 0.5 mm). Zoom in for a \
                             close fit.",
                        );
                        let mut p = r.pos.unwrap_or((0.0, 0.0));
                        let mut changed = false;
                        ui.horizontal(|ui| {
                            for (label, v) in [("x", &mut p.0), ("y", &mut p.1)] {
                                ui.label(label);
                                changed |= ui
                                    .add(egui::DragValue::new(v).speed(0.02).fixed_decimals(2).suffix(" mm"))
                                    .changed();
                            }
                        });
                        if changed {
                            r.pos = Some(p);
                        }
                        match r.pos {
                            Some(p) => {
                                let sc = card.score(p.0, p.1);
                                ui.label(RichText::new(format!("Score {}", sc.text(decimal))).strong());
                            }
                            None => {
                                ui.label(RichText::new("Not placed yet.").weak());
                            }
                        }
                        ui.horizontal(|ui| {
                            if ui.add_enabled(r.pos.is_some(), egui::Button::new("Confirm")).clicked() {
                                act = Act::Confirm;
                            }
                            if ui.button("Back").clicked() {
                                act = Act::To(Step::Choose);
                            }
                        });
                    }
                    Step::Same => {
                        wrap(ui, "Click the shot on the card whose hole it went through, or pick it:");
                        let shown = r
                            .same
                            .and_then(|k| others.iter().find(|o| o.1 == k))
                            .map_or("Pick a shot", |o| o.2.as_str());
                        egui::ComboBox::from_id_salt("resolve_same")
                            .selected_text(shown)
                            .width(ui.available_width())
                            .show_ui(ui, |ui| {
                                for (_, k, text) in &others {
                                    ui.selectable_value(&mut r.same, Some(*k), text.as_str());
                                }
                            });
                        ui.horizontal(|ui| {
                            if ui.add_enabled(r.same.is_some(), egui::Button::new("Confirm")).clicked() {
                                act = Act::Confirm;
                            }
                            if ui.button("Back").clicked() {
                                act = Act::To(Step::Choose);
                            }
                        });
                    }
                }
            });
        if !open {
            act = Act::Close;
        }
        match act {
            Act::Stay => {}
            Act::To(step) => {
                r.step = step;
                r.grab = None;
                if step != Step::Choose {
                    // On the card now: the popup out of the way, and kept open when the
                    // selection moves (a click picks the place or the shot).
                    r.from_selection = false;
                    self.selected_shot = None;
                }
                // Changing a hit resolved by hand: from where it is.
                if step == Step::Place && r.pos.is_none() && matches!(how, Some(How::Placed | How::Same(_))) {
                    r.pos = Some((hit.x_mm(), hit.y_mm()));
                }
                if step == Step::Same && r.same.is_none() {
                    r.same = match how {
                        Some(How::Same(o)) => Some(o),
                        _ => None,
                    };
                }
            }
            Act::Miss => {
                self.resolving = None;
                self.resolve_hit(key, How::Miss, (0.0, 0.0));
            }
            Act::Confirm => {
                let r = self.resolving.take().unwrap();
                match (r.step, r.pos, r.same) {
                    (Step::Place, Some(p), _) => self.resolve_hit(key, How::Placed, p),
                    (Step::Same, _, Some(o)) => self.resolve_hit(key, How::Same(o), (0.0, 0.0)),
                    _ => {}
                }
            }
            Act::Undo => {
                self.resolving = None;
                self.unresolve_hit(key);
            }
            Act::Close => {
                let r = self.resolving.take().unwrap();
                if r.from_selection {
                    self.selected_shot = None;
                }
            }
        }
    }

    /// The card, `width` points wide, with every shot of the series; the newest in color.
    /// Clicking a shot opens a small popup with its info; clicking it again, clicking
    /// elsewhere on the card or Escape closes it. Zoom with the wheel or the controls at the
    /// bottom; drag to move the card while zoomed in.
    fn target_card(&mut self, ui: &mut Ui, width: f32) {
        let card_mm = self.card.size_mm();
        let base = width / card_mm.x; // points per mm at 100%
        let (rect, response) = ui.allocate_exact_size(card_mm * base, egui::Sense::click_and_drag());
        self.card_rect = rect;

        // Zoom (about the pointer) and pan.
        self.view.clamp(card_mm);
        let view_before = self.view;
        if response.hovered() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = pinch * (scroll / 200.0).exp();
            if factor != 1.0 {
                let anchor = response.hover_pos().map_or(Vec2::ZERO, |p| p - rect.center());
                self.view.zoom_by(factor, anchor, base, card_mm);
            }
            // The wheel zooms the card; it mustn't also scroll the page around it.
            ui.input_mut(|i| i.smooth_scroll_delta.y = 0.0);
        }
        // Resolving a hit by hand, placing it: a drag that starts on the placed shot moves it
        // (anywhere else it moves the card, as always). It follows the pointer, held where it
        // was grabbed (egui starts a drag only after a few points: deltas would lose them).
        {
            let scale = base * self.view.zoom;
            let origin = rect.center() - self.view.centre_mm * scale;
            let centre = self.card.to_card(0.0, 0.0);
            if response.drag_started()
                && let Some(r) = self.resolving.as_mut().filter(|r| r.step == Step::Place)
            {
                let reach = (self.card.pellet_diameter_mm / 2.0 * scale).max(10.0);
                r.grab = match (r.pos, ui.input(|i| i.pointer.press_origin())) {
                    (Some(p), Some(press)) => {
                        let at = origin + self.card.to_card(p.0, p.1) * scale;
                        (at.distance(press) <= reach).then(|| at - press)
                    }
                    _ => None,
                };
            }
            if response.drag_stopped()
                && let Some(r) = &mut self.resolving
                && let Some(grab) = r.grab.take()
                && let Some(ptr) = response.interact_pointer_pos().or(ui.input(|i| i.pointer.latest_pos()))
            {
                let m = (ptr + grab - origin) / scale;
                r.pos = Some((m.x - centre.x, centre.y - m.y));
            }
            if let Some(r) = &mut self.resolving
                && let Some(grab) = r.grab
                && let Some(ptr) = response.interact_pointer_pos()
            {
                let m = (ptr + grab - origin) / scale;
                r.pos = Some((m.x - centre.x, centre.y - m.y));
            }
        }
        let moving = self.resolving.as_ref().is_some_and(|r| r.grab.is_some());
        if response.dragged() && !moving {
            self.view.pan(response.drag_delta(), base, card_mm);
        }

        let scale = base * self.view.zoom; // points per mm
        let origin = rect.center() - self.view.centre_mm * scale; // the card's top left
        let to_screen = |mm: Vec2| origin + mm * scale;
        let painter = ui.painter_at(rect);
        let ppp = ui.ctx().pixels_per_point();
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        // With a card photo, the printed card is drawn where the real card hung (turned and
        // shifted in the holder, `photo::CardPose`), so its rings and numbers lie on the
        // photo's; the shots stay in the sensor's frame, on the photo's holes.
        let pose = self
            .card_photo
            .as_ref()
            .map_or(photo::CardPose::IDENTITY, |cp| cp.p.pose);
        let centre = self.card.to_card(0.0, 0.0);
        // A point of the printed card (mm from its top left) in the view, and back.
        let posed = |m: Vec2| -> Vec2 {
            let f = pose.apply((f64::from(m.x - centre.x), f64::from(centre.y - m.y)));
            self.card.to_card(f.0 as f32, f.1 as f32)
        };
        let unposed = |m: Vec2| -> Vec2 {
            let p = pose.invert((f64::from(m.x - centre.x), f64::from(centre.y - m.y)));
            self.card.to_card(p.0 as f32, p.1 as f32)
        };
        let quad = |mm: egui::Rect| {
            [mm.left_top(), mm.right_top(), mm.right_bottom(), mm.left_bottom()]
                .map(|c| to_screen(posed(c.to_vec2())))
        };
        if let Some(tex) = self.card_texture.get(ui.ctx(), &self.card, width * ppp) {
            let all = egui::Rect::from_min_size(egui::Pos2::ZERO, card_mm);
            paint_quad(&painter, tex.id(), quad(all));
        }
        if self.view.zoom > 1.0 {
            // A sharp render of the visible part, over the stretched full card. Rendered
            // once the view stops moving; while it moves the last one moves along.
            let visible = egui::Rect::from_center_size(self.view.centre_mm.to_pos2(), rect.size() / scale);
            // The part of the printed card in view (a turned card: what covers the view).
            let region = egui::Rect::from_points(
                &[
                    visible.left_top(),
                    visible.right_top(),
                    visible.right_bottom(),
                    visible.left_bottom(),
                ]
                .map(|c| unposed(c.to_vec2()).to_pos2()),
            );
            let px = [
                (region.width() * scale * ppp).round() as u32,
                (region.height() * scale * ppp).round() as u32,
            ];
            let settled = self.view == view_before && !response.dragged();
            if let Some((tex, mm)) = self
                .card_texture
                .detail(ui.ctx(), &self.card, region, px, settled)
            {
                paint_quad(&painter, tex.id(), quad(mm));
            }
            if !settled {
                ui.ctx().request_repaint();
            }
        }

        // The card photo under the shots (C4), faded in over the printed card, and the holes
        // found in it as rings, also with the photo faded out.
        let radius = self.card.pellet_diameter_mm / 2.0 * scale;
        let fade = self.photo_fade;
        if let Some(cp) = &mut self.card_photo {
            if fade > 0.0
                && let Some(tex) = cp.texture(ui.ctx())
            {
                painter.image(
                    tex.id(),
                    egui::Rect::from_min_size(origin, card_mm * scale),
                    uv,
                    Color32::WHITE.gamma_multiply(fade),
                );
            }
            for h in cp.holes() {
                let c = to_screen(self.card.to_card(h.0 as f32, h.1 as f32));
                painter.circle_stroke(c, radius, Stroke::new(3.0, Color32::from_black_alpha(150)));
                painter.circle_stroke(c, radius, Stroke::new(1.2, Color32::WHITE));
            }
        }

        // Replay: only the first `visible` shots are on the card.
        let now = Instant::now();
        let len = self.series.len();
        if !self.show_replay_controls {
            self.replay.stop();
        }
        if let Some(at) = self.replay.tick(len, now) {
            ui.ctx().request_repaint_after(at.saturating_duration_since(now));
        }
        let visible = self.replay.visible(len);

        let shot_pos = |shot: &protocol::Shot| to_screen(self.card.to_card(shot.x_mm(), shot.y_mm()));
        // A point in the view as a shot position (mm from the centre, y up).
        let centre_mm = self.card.to_card(0.0, 0.0);
        let to_mm = |p: egui::Pos2| {
            let m = (p - origin) / scale;
            (m.x - centre_mm.x, centre_mm.y - m.y)
        };
        for (i, (shot, _)) in self.series[..visible].iter().enumerate() {
            if !self.on_card(shot) {
                continue; // a miss counted by hand
            }
            let (fill, stroke) = if i + 1 == visible {
                (LAST_SHOT_COLOR, Stroke::new(1.5, Color32::WHITE))
            } else {
                (Color32::from_gray(150), Stroke::new(1.0, Color32::from_gray(30)))
            };
            if shot.scored() {
                painter.circle(shot_pos(shot), radius, fill, stroke);
            } else {
                // Resolved by hand: a dashed edge, white dashes on a dark ring (seen on the
                // card's black and white).
                painter.circle_filled(shot_pos(shot), radius, fill);
                painter.circle_stroke(shot_pos(shot), radius, Stroke::new(2.5, Color32::from_gray(20)));
                dashed_circle(&painter, shot_pos(shot), radius, Stroke::new(1.5, Color32::WHITE));
            }
        }

        // Resolving a hit: the shot being placed, or the shot whose hole it went through.
        let step = self.resolving.as_ref().map(|r| r.step);
        if let Some(r) = &self.resolving {
            if r.step == Step::Place
                && let Some(p) = r.pos
            {
                let c = to_screen(self.card.to_card(p.0, p.1));
                painter.circle_filled(c, radius, LAST_SHOT_COLOR.gamma_multiply(0.45));
                painter.circle_stroke(c, radius, Stroke::new(3.0, Color32::from_black_alpha(160)));
                painter.circle_stroke(c, radius, Stroke::new(1.2, Color32::WHITE));
                // A cross through its centre, to line it up with the hole's.
                let arm = radius * 1.6 + 6.0;
                for d in [egui::vec2(arm, 0.0), egui::vec2(0.0, arm)] {
                    painter.line_segment([c - d, c + d], Stroke::new(2.5, Color32::from_black_alpha(160)));
                    painter.line_segment([c - d, c + d], Stroke::new(1.0, Color32::WHITE));
                }
            }
            if r.step == Step::Same
                && let Some((s, _)) = r.same.and_then(|k| self.series.iter().find(|(s, _)| resolve::key(s) == k))
            {
                let c = shot_pos(s);
                painter.circle_stroke(c, radius + 3.0, Stroke::new(3.0, Color32::BLACK));
                painter.circle_stroke(c, radius + 3.0, Stroke::new(1.5, LAST_SHOT_COLOR));
            }
        }

        // The shot under the pointer: the nearest within the hole (at least a few points
        // wide on a small card), the newest of equally near ones (it's drawn on top).
        let hit_r = radius.max(6.0);
        let shot_at = |p: egui::Pos2| {
            self.series[..visible]
                .iter()
                .enumerate()
                .rev()
                .filter(|(_, (shot, _))| self.on_card(shot))
                .map(|(i, (shot, _))| (i, shot_pos(shot).distance(p)))
                .filter(|&(_, d)| d <= hit_r)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        };
        let on_placed = |p: egui::Pos2| {
            self.resolving
                .as_ref()
                .and_then(|r| r.pos.filter(|_| r.step == Step::Place))
                .is_some_and(|q| to_screen(self.card.to_card(q.0, q.1)).distance(p) <= radius.max(10.0))
        };
        if moving || (response.dragged() && self.view.zoom > 1.0) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if step == Some(Step::Place) && response.hover_pos().is_some_and(on_placed) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        } else if step == Some(Step::Place) && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        } else if response.hover_pos().and_then(shot_at).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        } else if response.hovered() && self.view.zoom > 1.0 {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if response.clicked() {
            let key = |i: usize| (self.series[i].0.boot, self.series[i].0.seq);
            let at = response.interact_pointer_pos();
            match step {
                // Placing: the click puts the shot there (again: moves it there).
                Some(Step::Place) => {
                    if let (Some(r), Some(p)) = (&mut self.resolving, at) {
                        r.pos = Some(to_mm(p));
                    }
                }
                // Same hole: the click picks the shot.
                Some(Step::Same) => {
                    let hit = at.and_then(shot_at).map(key);
                    if let Some(r) = &mut self.resolving
                        && let Some(k) = hit.filter(|&k| k != r.key)
                    {
                        r.same = Some(k);
                    }
                }
                _ => {
                    let hit = at.and_then(shot_at).map(key);
                    self.selected_shot = if hit == self.selected_shot { None } else { hit };
                }
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.selected_shot = None;
            self.resolving = None;
        }
        // Placing: the arrow keys move the shot 0.05 mm, with Shift 0.5 mm (while no field
        // has the keyboard).
        if step == Some(Step::Place) && ui.memory(|m| m.focused().is_none()) {
            let (dx, dy, shift) = ui.input(|i| {
                let k = |key| i.num_presses(key) as f32;
                (
                    k(egui::Key::ArrowRight) - k(egui::Key::ArrowLeft),
                    k(egui::Key::ArrowUp) - k(egui::Key::ArrowDown),
                    i.modifiers.shift,
                )
            });
            let step_mm = if shift { 0.5 } else { 0.05 };
            if (dx, dy) != (0.0, 0.0)
                && let Some(r) = &mut self.resolving
                && let Some(p) = &mut r.pos
            {
                p.0 += dx * step_mm;
                p.1 += dy * step_mm;
            }
        }

        // Zoom bottom right, replay bottom left; the replay one row up if both don't fit.
        let margin = 10.0;
        let height = BAR_BUTTON + 2.0 * BAR_PAD;
        let zoom_w = 2.0 * BAR_BUTTON + ZOOM_LABEL + 2.0 * BAR_PAD;
        let replay_w = 6.0 * BAR_BUTTON + REPLAY_LABEL + 2.0 * BAR_PAD;
        let bottom = rect.bottom() - margin - height;
        if self.show_zoom_controls {
            let bar = egui::Rect::from_min_size(
                egui::pos2(rect.right() - margin - zoom_w, bottom),
                egui::vec2(zoom_w, height),
            );
            if let Some(zoom) = zoom_controls(ui, &painter, bar, self.view.zoom) {
                self.view.zoom = zoom;
                self.view.clamp(card_mm);
                ui.ctx().request_repaint();
            }
        }
        if self.show_replay_controls {
            let crowded = self.show_zoom_controls && rect.width() < zoom_w + replay_w + 3.0 * margin;
            let top = if crowded { bottom - height - 8.0 } else { bottom };
            let bar = egui::Rect::from_min_size(
                egui::pos2(rect.left() + margin, top),
                egui::vec2(replay_w, height),
            );
            if replay_controls(ui, &painter, bar, &mut self.replay, len, now) {
                ui.ctx().request_repaint();
            }
            replay_speed_label(ui, &painter, bar, self.replay.delay_s);
        }

        // The photo's fade, top right, while there is one.
        if self.card_photo.is_some() {
            let w = 210.0;
            let bar = egui::Rect::from_min_size(
                egui::pos2(rect.right() - margin - w, rect.top() + margin),
                egui::vec2(w, height),
            );
            if let Some(f) = fade_control(ui, &painter, bar, self.photo_fade) {
                self.photo_fade = f;
                ui.ctx().request_repaint();
            }
        }

        // Every shot of the series that nearly scored higher, worth a look at the real card.
        // Stays until the series is cleared, also during a replay.
        let close_calls: Vec<(usize, Higher)> = self
            .series
            .iter()
            .enumerate()
            .filter(|(_, (s, _))| self.on_card(s))
            .filter_map(|(i, (s, _))| Some((i, self.card.higher(s.x_mm(), s.y_mm())?)))
            .filter(|(_, h)| h.gap_mm <= self.close_call_mm)
            .collect();
        let mut notice_top = 0.0;
        if self.show_close_calls && !close_calls.is_empty() {
            let lines: Vec<String> = close_calls
                .iter()
                .map(|&(i, h)| format!("Shot {} · {}", i + 1, review_detail(h)))
                .collect();
            let (clicked, below) = review_notice(
                ui,
                &painter,
                rect,
                "Close Call - Manual review recommended",
                0.0,
                "close_calls",
                &lines,
                Some("Show this shot"),
            );
            notice_top = below;
            if let Some(line) = clicked {
                let i = close_calls[line].0;
                let shot = &self.series[i].0;
                self.selected_shot = Some((shot.boot, shot.seq));
                if i >= visible {
                    self.replay.stop(); // not replayed yet: show all shots to show this one
                }
                ui.ctx().request_repaint();
            }
        }

        // The hits with no position (protocol v3): the shooter checks the paper card, then
        // resolves them (a click on a line).
        if !self.unscored.is_empty() {
            let lines: Vec<String> = self
                .unscored
                .iter()
                .map(|(after, s)| unscored_line(*after, s))
                .collect();
            let (clicked, _) = review_notice(
                ui,
                &painter,
                rect,
                "Not scored - check the card",
                notice_top,
                "unscored",
                &lines,
                Some("Resolve it: place it on the card, the same hole as another shot, or a miss"),
            );
            if let Some(line) = clicked {
                let key = resolve::key(&self.unscored[line].1);
                self.selected_shot = None;
                self.resolving = Some(Resolving::new(key, false));
            }
        }

        // Gone when the series was cleared or replaced.
        let Some(i) = self
            .selected_shot
            .and_then(|key| self.series.iter().position(|(s, _)| (s.boot, s.seq) == key))
        else {
            self.selected_shot = None;
            return;
        };
        let (shot, score) = &self.series[i];
        let pos = shot_pos(shot);
        if !rect.contains(pos) || i >= visible || !self.on_card(shot) {
            return; // out of view, or not replayed yet (it shows again when it's back), or a miss
        }
        let how = self.manual.get(&resolve::key(shot)).copied();
        // A dark and a light ring, visible on the card's black and white parts.
        painter.circle_stroke(pos, radius + 3.0, Stroke::new(3.0, Color32::BLACK));
        painter.circle_stroke(pos, radius + 3.0, Stroke::new(1.5, Color32::WHITE));

        // Beside the shot, towards the card's middle so it stays on the card.
        let gap = radius + 8.0;
        let (offset, pivot) = if pos.x > rect.center().x {
            (egui::vec2(-gap, -gap), egui::Align2::RIGHT_BOTTOM)
        } else {
            (egui::vec2(gap, -gap), egui::Align2::LEFT_BOTTOM)
        };
        let decimal = self.decimal_scoring && self.card.decimal;
        egui::Area::new(egui::Id::unique("shot_info"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos + offset)
            .pivot(pivot)
            .constrain(true)
            .interactable(false)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Shot {}", i + 1)).strong());
                        ui.label("Score:");
                        ui.label(RichText::new(score.text(decimal)).size(20.0).strong());
                    });
                    ui.label(format!("x {:+.2} mm · y {:+.2} mm", shot.x_mm(), shot.y_mm()));
                    let off = shot.x_mm().hypot(shot.y_mm());
                    ui.label(RichText::new(format!("{off:.2} mm from the centre")).weak());
                    match how {
                        Some(How::Same(other)) => {
                            let n = self.series.iter().position(|(s, _)| resolve::key(s) == other);
                            let text = match n {
                                Some(n) => format!("By hand: the same hole as shot {}", n + 1),
                                None => "By hand: the same hole as an earlier shot".to_string(),
                            };
                            ui.label(RichText::new(text).small().weak());
                        }
                        Some(_) => {
                            ui.label(RichText::new("Placed by hand").small().weak());
                        }
                        None => {
                            if let Some(details) = shot_details(shot) {
                                ui.label(RichText::new(details).small().weak());
                            }
                        }
                    }
                    let higher = self.card.higher(shot.x_mm(), shot.y_mm());
                    if let Some(h) =
                        higher.filter(|h| self.show_close_calls && h.gap_mm <= self.close_call_mm)
                    {
                        let text = format!("⚠ Close call: {}, manual review recommended", review_detail(h));
                        ui.label(RichText::new(text).small());
                    }
                });
            });
    }

    /// Last shot and the series list with the total. The series card fills the rest of
    /// `ui`'s height; its list scrolls.
    fn series_column(&mut self, ui: &mut Ui) {
        let decimal = self.decimal_scoring && self.card.decimal;
        card(ui, "Last shot", |ui| {
            let Some((shot, score)) = self.series.last() else {
                let text = if self.shots_supported {
                    "No shots yet. Press BOOT on the target for a test shot."
                } else {
                    "This target's firmware doesn't send shots. Update the firmware."
                };
                ui.add(egui::Label::new(RichText::new(text).weak()).wrap());
                return;
            };
            let how = self.manual.get(&resolve::key(shot)).copied();
            ui.horizontal(|ui| {
                ui.label(RichText::new(score.text(decimal)).size(44.0).strong());
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let mut line = format!("Shot {} · ring {}", self.series.len(), score.ring);
                    if score.inner_ten && decimal {
                        line.push_str(" · inner ten");
                    }
                    ui.label(line);
                    if how == Some(How::Miss) {
                        ui.label(RichText::new("A miss, counted by hand").weak());
                    } else {
                        let off = shot.x_mm().hypot(shot.y_mm());
                        let hand = if how.is_some() { " · placed by hand" } else { "" };
                        ui.label(RichText::new(format!("{off:.1} mm from the centre{hand}")).weak());
                    }
                });
            });
        });

        // The series card fills the height the chart below it leaves.
        let chart = !self.series.is_empty();
        let series_h = if chart {
            (ui.available_height() - CHART_CARD_H - ui.spacing().item_spacing.y).max(200.0)
        } else {
            ui.available_height()
        };
        let mut clear = false;
        let mut photo = false;
        let (mut another, mut forget) = (false, false);
        let mut keep = self.card_photo.as_ref().is_some_and(|cp| cp.p.photo.meta.keep);
        let mut select: Option<(u32, u32)> = None;
        let width = ui.available_width();
        // With a card photo of this series: each shot's score on the paper, its hole gauged
        // against the printed rings (Some(None): no hole found; None: a shot after the
        // photo). A hit resolved by hand was judged on the paper: its score is the paper's.
        let paper: Option<Vec<Option<Option<Score>>>> = self
            .card_photo
            .as_ref()
            .filter(|cp| !cp.p.shots.is_empty())
            .map(|cp| {
                self.series
                    .iter()
                    .map(|(s, score)| {
                        if !s.scored() {
                            return Some(Some(*score));
                        }
                        cp.check(resolve::key(s))
                            .map(|c| c.paper.map(|q| self.card.score(q.0 as f32, q.1 as f32)))
                    })
                    .collect()
            });
        ui.allocate_ui(egui::vec2(width, series_h), |ui| {
            ui.set_height(series_h);
            card(ui, &format!("Series · {}", self.card.name), |ui| {
                let tenths: u32 = self.series.iter().map(|(_, s)| s.tenths as u32).sum();
                let rings: u32 = self.series.iter().map(|(_, s)| s.ring as u32).sum();
                ui.horizontal(|ui| {
                    let total = if decimal {
                        format!("{}.{}", tenths / 10, tenths % 10)
                    } else {
                        rings.to_string()
                    };
                    let inner_tens = self.series.iter().filter(|(_, s)| s.inner_ten).count();
                    ui.label(RichText::new(total).size(28.0).strong());
                    let n = self.series.len();
                    let mut detail = format!("{n} shot{}", if n == 1 { "" } else { "s" });
                    if decimal {
                        detail.push_str(&format!(" · {rings} in whole rings"));
                    } else if inner_tens > 0 {
                        detail.push_str(&format!(" · {inner_tens}X"));
                    }
                    if !self.manual.is_empty() {
                        detail.push_str(&format!(" · {} by hand", self.manual.len()));
                    }
                    if !self.unscored.is_empty() {
                        detail.push_str(&format!(" · {} not scored", self.unscored.len()));
                    }
                    if let Some(paper) = &paper {
                        let differ = paper
                            .iter()
                            .zip(&self.series)
                            .filter(|(p, (_, s))| p.flatten().is_some_and(|p| p.text(decimal) != s.text(decimal)))
                            .count();
                        let on_paper: Option<Vec<Score>> = paper.iter().map(|p| p.flatten()).collect();
                        match on_paper {
                            Some(p) if decimal => {
                                let t: u32 = p.iter().map(|s| s.tenths as u32).sum();
                                detail.push_str(&format!(" · paper {}.{}", t / 10, t % 10));
                            }
                            Some(p) => {
                                let r: u32 = p.iter().map(|s| s.ring as u32).sum();
                                detail.push_str(&format!(" · paper {r}"));
                            }
                            None => detail.push_str(" · paper incomplete"),
                        }
                        if differ > 0 {
                            detail.push_str(&format!(" ({differ} differ)"));
                        }
                    }
                    ui.label(RichText::new(detail).weak());
                });
                if self.series.is_empty() && self.unscored.is_empty() && self.card_photo.is_none() {
                    photo = photo_button(ui);
                    ui.allocate_space(ui.available_size()); // fill the column anyway
                    return;
                }
                ui.separator();
                // The buttons at the bottom, the list scrolling in the height between.
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        clear = ui
                            .add(egui::Button::new("Clear series").frame(false))
                            .on_hover_text("Start a new series. Also clears the sensor's stored shots.")
                            .clicked();
                        if let Some(cp) = &self.card_photo {
                            another = ui.add(egui::Button::new("Take another photo").frame(false)).clicked();
                            if ui.add(egui::Button::new("Open the folder").frame(false)).clicked()
                                && let Some(dir) = cp.p.photo.image.parent()
                            {
                                open_folder(dir);
                            }
                            forget = ui
                                .add(egui::Button::new("Remove from the card").frame(false))
                                .on_hover_text("Takes the photo off the card and the series (the file stays).")
                                .clicked();
                        } else {
                            photo = photo_button(ui);
                        }
                    });
                    if self.card_photo.is_some() {
                        ui.checkbox(&mut keep, "Keep the photo for evaluation (else removed after 30 days)");
                    }
                    ui.separator();
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        if let Some(cp) = &self.card_photo {
                            photo_summary(ui, cp, &self.series);
                            ui.separator();
                        }
                        egui::ScrollArea::vertical()
                            .id_salt("series")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                select = self.series_list(ui, paper.as_deref(), decimal);
                            });
                    });
                });
            });
        });
        if chart {
            let title = if decimal {
                "Shots per score"
            } else {
                "Shots per ring"
            };
            card(ui, title, |ui| {
                let scores: Vec<Score> = self.series.iter().map(|(_, s)| *s).collect();
                score_chart(ui, &scores, decimal);
            });
        }
        if let Some(key) = select {
            // A row clicked: its popup on the card (clicking it again closes it); an
            // unscored hit's opens the resolve window.
            if self.unscored.iter().any(|(_, s)| resolve::key(s) == key) {
                self.selected_shot = None;
                self.resolving = Some(Resolving::new(key, false));
            } else if self.selected_shot == Some(key) {
                self.selected_shot = None;
            } else {
                self.selected_shot = Some(key);
                let i = self.series.iter().position(|(s, _)| (s.boot, s.seq) == key);
                if i.is_some_and(|i| i >= self.replay.visible(self.series.len())) {
                    self.replay.stop(); // not replayed yet: show all shots to show this one
                }
            }
        }
        if let Some(cp) = &mut self.card_photo
            && keep != cp.p.photo.meta.keep
        {
            match upload::set_keep(&cp.p.photo.meta_file, keep) {
                Ok(()) => cp.p.photo.meta.keep = keep,
                Err(e) => self.message = Some(format!("Could not mark the photo: {e}")),
            }
        }
        if forget {
            self.card_photo = None;
        }
        if clear {
            self.send(Command::ClearShots);
            self.clear_series();
        }
        if photo {
            self.open_photo(ui.ctx());
        }
        if another {
            // The window again, with a new link and the QR code.
            self.open_photo(ui.ctx());
            let label = self.series_label();
            if let Some(t) = &mut self.photo_task {
                t.new_link(&label);
                t.show_link = true;
            }
        }
    }

    /// The series, newest first, a hit with no score after the scored shot it followed.
    /// Without a card photo: # and the score. With one: #, the OpenSights score, the paper's
    /// (both red where they differ), and the ground truth per shot. A scored row is
    /// clickable: returns the shot clicked, (boot, seq). So is an unscored hit's row (it
    /// opens the resolve window); a hit resolved by hand is marked "by hand".
    fn series_list(&self, ui: &mut Ui, paper: Option<&[Option<Option<Score>>]>, decimal: bool) -> Option<(u32, u32)> {
        let mut clicked = None;
        let red = ui.visuals().error_fg_color;
        let text = |s: &Score, strong: bool| {
            let t = RichText::new(s.text(decimal));
            if strong { t.strong() } else { t.weak() }
        };
        // With the inner ten's X, for the table's cells.
        let text_x = |s: &Score, strong: bool| {
            let x = if s.inner_ten && decimal { " X" } else { "" };
            let t = RichText::new(format!("{}{x}", s.text(decimal)));
            if strong { t.strong() } else { t.weak() }
        };
        let resolving = self.resolving.as_ref().map(|r| r.key);
        let by_hand = |key: Key| self.manual.get(&key).map(|&h| how_label(h));
        let Some(paper) = paper else {
            // No photo: two columns.
            for i in (0..=self.series.len()).rev() {
                for (_, hit) in self.unscored.iter().rev().filter(|(after, _)| *after == i) {
                    let key = resolve::key(hit);
                    let bg = ui.painter().add(egui::Shape::Noop);
                    let row = ui
                        .horizontal(|ui| {
                            ui.label(RichText::new("·").weak());
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(unscored_label(hit)).weak().italics());
                            });
                        })
                        .response
                        .rect;
                    if row_click(ui, row, bg, key, resolving == Some(key), RESOLVE_TIP, &hit_name(hit, i)) {
                        clicked = Some(key);
                    }
                }
                let Some(i) = i.checked_sub(1) else { break };
                let (shot, score) = &self.series[i];
                let key = (shot.boot, shot.seq);
                let bg = ui.painter().add(egui::Shape::Noop);
                let row = ui
                    .horizontal(|ui| {
                        ui.label(RichText::new(format!("{}", i + 1)).weak());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(text(score, true));
                            if score.inner_ten && decimal {
                                ui.label(RichText::new("X").small().weak());
                            }
                            if let Some(h) = by_hand(key) {
                                ui.label(RichText::new(h).small().weak().italics());
                            }
                        });
                    })
                    .response
                    .rect;
                if row_click(ui, row, bg, key, self.selected_shot == Some(key), SHOW_TIP, &format!("Shot {}", i + 1)) {
                    clicked = Some(key);
                }
            }
            return clicked;
        };
        egui::Grid::new("series_table")
            .num_columns(6)
            .spacing([14.0, 4.0])
            .show(ui, |ui| {
                let head = |ui: &mut Ui, t: &str, tip: &str| {
                    let r = ui.label(RichText::new(t).small().weak());
                    if !tip.is_empty() {
                        r.on_hover_text(tip);
                    }
                };
                head(ui, "#", "");
                head(ui, "OpenSights", "The sensor's score.");
                head(ui, "Paper", "The score of the shot's hole in the card photo, gauged against the printed rings.");
                head(ui, "Shot x, y (mm)", "Where the sensor put the shot.");
                head(ui, "Hole − shot (mm)", "The hole in the photo minus the sensor's position: the sensor's error.");
                head(ui, "Off (mm)", "How far the sensor was off.");
                ui.end_row();
                for i in (0..=self.series.len()).rev() {
                    for (_, hit) in self.unscored.iter().rev().filter(|(after, _)| *after == i) {
                        let key = resolve::key(hit);
                        let bg = ui.painter().add(egui::Shape::Noop);
                        let mut row = ui.label(RichText::new("·").weak()).rect;
                        row = row.union(ui.label(RichText::new(unscored_label(hit)).weak().italics()).rect);
                        for _ in 0..4 {
                            row = row.union(ui.label("").rect);
                        }
                        if row_click(ui, row, bg, key, resolving == Some(key), RESOLVE_TIP, &hit_name(hit, i)) {
                            clicked = Some(key);
                        }
                        ui.end_row();
                    }
                    let Some(i) = i.checked_sub(1) else { break };
                    let (shot, score) = &self.series[i];
                    let key = (shot.boot, shot.seq);
                    let bg = ui.painter().add(egui::Shape::Noop);
                    let p = paper.get(i).copied().flatten();
                    let differ = matches!(p, Some(Some(p)) if p.text(decimal) != score.text(decimal));
                    let mut row = ui.label(RichText::new(format!("{}", i + 1)).weak()).rect;
                    let mut cell = |ui: &mut Ui, t: RichText| row = row.union(ui.label(t).rect);
                    cell(ui, if differ { text_x(score, true).color(red) } else { text_x(score, true) });
                    match p {
                        Some(Some(ps)) => {
                            cell(ui, if differ { text_x(&ps, true).color(red) } else { text_x(&ps, false) });
                        }
                        Some(None) => cell(ui, RichText::new("–").weak()),
                        None => cell(ui, RichText::new("·").weak()),
                    }
                    let check = self.card_photo.as_ref().and_then(|cp| cp.check(key));
                    match check {
                        _ if !shot.scored() => {
                            // Resolved by hand: no measurement to check.
                            let h = by_hand(key).unwrap_or("by hand");
                            cell(ui, RichText::new(h).weak().italics());
                            cell(ui, RichText::new(""));
                            cell(ui, RichText::new(""));
                        }
                        Some(c) => {
                            cell(ui, RichText::new(format!("{:+.2}, {:+.2}", c.shot.0, c.shot.1)));
                            match c.diff {
                                Some(d) => {
                                    cell(ui, RichText::new(format!("{:+.3}, {:+.3}", d.0, d.1)));
                                    cell(ui, RichText::new(format!("{:.3}", d.0.hypot(d.1))).strong());
                                }
                                None => {
                                    cell(ui, RichText::new("no hole found").weak().italics());
                                    cell(ui, RichText::new(""));
                                }
                            }
                        }
                        None => {
                            // A shot after the photo.
                            cell(ui, RichText::new("after the photo").weak().italics());
                            cell(ui, RichText::new(""));
                            cell(ui, RichText::new(""));
                        }
                    }
                    if row_click(ui, row, bg, key, self.selected_shot == Some(key), SHOW_TIP, &format!("Shot {}", i + 1)) {
                        clicked = Some(key);
                    }
                    ui.end_row();
                }
            });
        clicked
    }

    /// Main window without a connection.
    fn main_body(&mut self, ui: &mut Ui) {
        if let Some(e) = &self.fatal {
            card(ui, "Bluetooth unavailable", |ui| {
                ui.label(e);
                ui.label(RichText::new("Check that Bluetooth is switched on, then restart the app.").weak());
            });
            return;
        }
        self.message_notice(ui, false);
        match &self.link {
            Link::Connected { .. } => {} // shown by target_screen
            Link::Connecting { name } => connecting_card(ui, name),
            Link::Idle => self.no_sensor_card(ui),
        }
    }

    /// Main window without a connection: points to the settings window, where sensors are paired.
    fn no_sensor_card(&mut self, ui: &mut Ui) {
        let remembered = self
            .remembered
            .as_deref()
            .and_then(|id| self.sensors.iter().find(|s| s.id == id));
        if let Some(s) = remembered
            && self.auto_connect
            && self.is_paired(s)
        {
            let name = s.name.clone();
            card(ui, "No sensor connected", |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Reconnecting to {name}…"));
                });
            });
            return;
        }
        let text = if self.remembered.is_none() {
            "No sensor is paired with this app yet. Pair one in Settings."
        } else if !self.auto_connect {
            "Disconnected. To connect again, or to pair another sensor, open Settings."
        } else {
            "Your sensor isn't in range. Check that it's powered, or pair another sensor in Settings."
        };
        let mut open = false;
        card(ui, "No sensor connected", |ui| {
            ui.add(egui::Label::new(text).wrap());
            open = ui.button("⚙  Open settings").clicked();
        });
        if open {
            self.open_settings(ui.ctx());
        }
    }

    /// Settings window: appearance, and everything about the sensor and its connection.
    fn settings_body(&mut self, ui: &mut Ui) {
        self.message_notice(ui, true);
        card(ui, "Appearance", |ui| {
            ui.horizontal(|ui| {
                ui.label("Dark mode");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    toggle_switch(ui, &mut self.dark_mode);
                });
            });
        });
        card(ui, "Scoring", |ui| {
            ui.horizontal(|ui| {
                ui.label("Decimal scoring");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    toggle_switch(ui, &mut self.decimal_scoring);
                });
            });
            ui.label(
                RichText::new("Off: whole rings, X for an inner ten. On: tenths, up to 10.9.")
                    .small()
                    .weak(),
            );
        });
        card(ui, "Target card", |ui| {
            ui.horizontal(|ui| {
                ui.label("Zoom controls");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    toggle_switch(ui, &mut self.show_zoom_controls);
                });
            });
            ui.label(
                RichText::new("The wheel zooms and dragging moves the card either way.")
                    .small()
                    .weak(),
            );
            ui.horizontal(|ui| {
                ui.label("Replay controls");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    toggle_switch(ui, &mut self.show_replay_controls);
                });
            });
            ui.horizontal(|ui| {
                ui.label("Default replay speed");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let slider =
                        egui::Slider::new(&mut self.replay_delay_s, REPLAY_DELAY_MIN..=REPLAY_DELAY_MAX)
                            .step_by(0.05)
                            .custom_formatter(|v, _| {
                                format!("{} {}s", replay_animal(v as f32), format_delay(v as f32))
                            })
                            .custom_parser(|t| {
                                t.trim()
                                    .trim_end_matches('s')
                                    .split_whitespace()
                                    .last()?
                                    .parse()
                                    .ok()
                            });
                    if ui.add(slider).changed() {
                        self.replay.delay_s = self.replay_delay_s;
                    }
                });
            });
            ui.label(RichText::new("Seconds between shots in a replay.").small().weak());
            ui.horizontal(|ui| {
                ui.label("Close call notification");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    toggle_switch(ui, &mut self.show_close_calls);
                });
            });
            ui.add_enabled_ui(self.show_close_calls, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Close call within");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add(
                            egui::Slider::new(&mut self.close_call_mm, CLOSE_CALL_MIN_MM..=CLOSE_CALL_MAX_MM)
                                .step_by(0.05)
                                .fixed_decimals(2)
                                .suffix(" mm"),
                        );
                    });
                });
            });
            ui.label(
                RichText::new("Flags shots this close to a higher score for manual review.")
                    .small()
                    .weak(),
            );
        });
        if let Some(e) = &self.fatal {
            card(ui, "Bluetooth unavailable", |ui| {
                ui.label(e);
            });
            return;
        }
        match &self.link {
            Link::Connected { id } => {
                let id = id.clone();
                card(ui, "Sensor settings", |ui| {
                    self.name_card(ui);
                    ui.separator();
                    self.pin_card(ui);
                    ui.separator();
                    self.heartbeat_card(ui);
                    ui.separator();
                    self.measurement_card(ui);
                    ui.separator();
                    section_title(ui, "Connection");
                    ui.label(format!("Connected to {}.", self.sensor_name));
                    ui.horizontal(|ui| {
                        if ui.button("Disconnect").clicked() {
                            self.auto_connect = false;
                            self.send(Command::Disconnect);
                        }
                        let forget = ui
                            .add(egui::Button::new("Forget sensor").frame(false))
                            .on_hover_text("Disconnect and stop reconnecting to this sensor automatically.");
                        if forget.clicked() && self.remembered.as_deref() == Some(id.as_str()) {
                            self.remembered = None;
                            self.auto_connect = false;
                            self.send(Command::Disconnect);
                        }
                    });
                });
            }
            Link::Connecting { name } => connecting_card(ui, name),
            Link::Idle => {
                self.sensors_card(ui);
                pairing_help_card(ui);
            }
        }
    }

    /// Protocol v3, read-only: the calibration, the settings, the air, what the events said,
    /// and the event log. (Changing them: serial for now, the calibration wizard in step 5d.)
    fn measurement_card(&mut self, ui: &mut Ui) {
        section(ui, "Measurement", |ui| {
            let weak = |ui: &mut Ui, t: String| {
                ui.add(egui::Label::new(RichText::new(t).weak()).wrap());
            };
            match (&self.status, &self.config) {
                (Some(st), Some((cal, set))) => {
                    ui.label(calibration_text(st, cal));
                    if st.flags & protocol::ST_CAL_MODE != 0 {
                        let point = if st.flags & protocol::ST_POINT != 0 {
                            format!(", template point ({:.2}, {:.2}) mm", st.pt_x, st.pt_y)
                        } else {
                            String::new()
                        };
                        ui.label(
                            RichText::new(format!("Calibration mode is on{point}: nothing is scored."))
                                .strong(),
                        );
                    }
                    weak(ui, settings_text(set));
                    weak(ui, air_text(st));
                    if st.flags & protocol::ST_MTU_SMALL != 0 {
                        ui.label(
                            "This computer's Bluetooth link is too small for the events (MTU under 183): \
                             they stay on the sensor.",
                        );
                    }
                }
                _ => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Reading the sensor's settings…").weak());
                    });
                }
            }
            ui.label(self.event_counts.text());
            let ready = self.status.is_some() && self.config.is_some() && self.cal.is_none();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(ready, egui::Button::new("Calibrate…"))
                    .on_hover_text("Sparks at the template's points, then the fit, then the record written to the sensor.")
                    .clicked()
                {
                    self.open_calibration();
                }
                if ui
                    .add_enabled(ready, egui::Button::new("Field check…"))
                    .on_hover_text("A few sparks at the template's check point: how far off the sensor solves them.")
                    .clicked()
                {
                    self.cal = Some(CalTask::Check(FieldCheck::new(&calwizard::grid_template())));
                    self.cal_report = None;
                }
            });
            ui.checkbox(&mut self.save_events, "Save every event to a log file")
                .on_hover_text(
                    "As the target's serial log lines (cfg, rec): firmware/tools/replay reads them.",
                );
            if self.save_events {
                match &self.event_log {
                    Some(log) => {
                        let dir = log.dir().display().to_string();
                        ui.horizontal(|ui| {
                            ui.add(egui::Label::new(RichText::new(&dir).small().weak()).wrap());
                            if ui.small_button("Open").clicked() {
                                open_folder(log.dir());
                            }
                        });
                    }
                    None => weak(ui, "No folder for the log on this system.".into()),
                }
                if let Some(e) = &self.log_error {
                    ui.label(e);
                }
            }
        });
    }

    fn heartbeat_card(&mut self, ui: &mut Ui) {
        section(ui, "Heartbeat", |ui| match self.heartbeat {
            Some(ms) => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(group_digits(ms)).size(40.0).strong());
                    ui.label(RichText::new("ms").weak());
                });
                let ago = self.heartbeat_at.map_or(0, |t| t.elapsed().as_secs());
                ui.label(RichText::new(format!("Uptime {} · updated {ago} s ago", uptime(ms))).weak());
            }
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Waiting for the first heartbeat…").weak());
                });
            }
        });
    }

    fn name_card(&mut self, ui: &mut Ui) {
        section(ui, "Name", |ui| {
            let checked = protocol::validate_name(&self.name_input);
            let unchanged = checked.as_deref().is_ok_and(|n| n == self.sensor_name);
            let mut submit = false;
            ui.horizontal(|ui| {
                let button_w = 92.0;
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.name_input)
                        .hint_text("Sensor name")
                        .char_limit(protocol::NAME_MAX_LEN + 10) // room to show the length error
                        .margin(egui::vec2(10.0, 7.0))
                        .desired_width(ui.available_width() - button_w - ui.spacing().item_spacing.x),
                );
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let button = ui.add_enabled(
                    checked.is_ok() && !unchanged,
                    egui::Button::new("Rename").min_size(egui::vec2(button_w, 0.0)),
                );
                submit = (button.clicked() || enter) && checked.is_ok() && !unchanged;
            });
            let hint = match &checked {
                Err(e) => e.clone(),
                Ok(name) => format!(
                    "{}/{} characters · stored on the sensor",
                    name.len(),
                    protocol::NAME_MAX_LEN
                ),
            };
            ui.label(RichText::new(hint).small().weak());
            if submit && let Ok(name) = checked {
                self.send(Command::SetName(name));
            }
        });
    }

    fn pin_card(&mut self, ui: &mut Ui) {
        section(ui, "Pairing PIN", |ui| {
            if !self.pin_supported {
                ui.label(RichText::new("This sensor's firmware can't change its PIN.").weak());
                return;
            }
            self.pin_input.retain(|c| c.is_ascii_digit());
            let checked = protocol::validate_pin(&self.pin_input);
            if let (true, Ok(pin)) = (self.pin_confirm, &checked) {
                ui.add(
                    egui::Label::new(format!(
                        "Change the PIN to {}? Every other computer paired with this sensor is \
                         unpaired and has to pair again with the new PIN.",
                        protocol::format_pin(*pin)
                    ))
                    .wrap(),
                );
                ui.horizontal(|ui| {
                    if ui.button("Change PIN").clicked() {
                        self.send(Command::SetPin(*pin));
                        self.pin_confirm = false;
                    }
                    if ui.add(egui::Button::new("Cancel").frame(false)).clicked() {
                        self.pin_confirm = false;
                    }
                });
                return;
            }
            self.pin_confirm = false;
            ui.horizontal(|ui| {
                let button_w = 92.0;
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.pin_input)
                        .hint_text("New 6-digit PIN")
                        .char_limit(protocol::PIN_DIGITS)
                        .margin(egui::vec2(10.0, 7.0))
                        .desired_width(ui.available_width() - button_w - ui.spacing().item_spacing.x),
                );
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let button = ui.add_enabled(
                    checked.is_ok(),
                    egui::Button::new("Change…").min_size(egui::vec2(button_w, 0.0)),
                );
                self.pin_confirm = (button.clicked() || enter) && checked.is_ok();
            });
            let hint = match &checked {
                Err(e) if !self.pin_input.is_empty() => e.clone(),
                _ => "Needed to pair a computer. Changing it unpairs all other computers.".into(),
            };
            ui.label(RichText::new(hint).small().weak());
        });
    }

    /// The PIN popup, while BlueZ waits for the passkey of the sensor being paired.
    fn passkey_modal(&mut self, ctx: &egui::Context) {
        let Some(prompt) = &mut self.passkey_prompt else {
            return;
        };
        let name = match &self.link {
            Link::Connecting { name } => name.clone(),
            _ => "the sensor".into(),
        };
        let mut answer = None;
        let modal = egui::Modal::new(egui::Id::unique("passkey")).show(ctx, |ui| {
            ui.set_width(320.0);
            ui.label(RichText::new("Enter PIN").size(20.0).strong());
            ui.add(egui::Label::new(format!("Pairing with {name}. Type the sensor's 6-digit PIN.")).wrap());
            prompt.input.retain(|c| c.is_ascii_digit());
            let checked = protocol::validate_pin(&prompt.input);
            let edit = ui.add(
                egui::TextEdit::singleline(&mut prompt.input)
                    .hint_text(protocol::format_pin(0))
                    .char_limit(protocol::PIN_DIGITS)
                    .font(FontId::new(24.0, FontFamily::Proportional))
                    .margin(egui::vec2(10.0, 7.0))
                    .desired_width(f32::INFINITY),
            );
            if std::mem::take(&mut prompt.focus) {
                edit.request_focus();
            }
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.label(
                RichText::new(format!(
                    "{} unless it was changed. Enter it within 30 seconds.",
                    protocol::format_pin(protocol::DEFAULT_PASSKEY)
                ))
                .small()
                .weak(),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let pair = ui.add_enabled(checked.is_ok(), egui::Button::new("Pair"));
                if let Ok(pin) = checked
                    && (pair.clicked() || enter)
                {
                    answer = Some(Some(pin));
                }
                if ui.add(egui::Button::new("Cancel").frame(false)).clicked() {
                    answer = Some(None);
                }
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(None); // Esc or a click outside
        }
        if let Some(pin) = answer
            && let Some(prompt) = self.passkey_prompt.take()
        {
            prompt.reply.send(pin);
        }
    }

    fn sensors_card(&mut self, ui: &mut Ui) {
        let mut connect_to = None;
        card(ui, "Sensors nearby", |ui| {
            if self.sensors.is_empty() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Searching for sensors…").weak());
                });
                return;
            }
            for (i, s) in self.sensors.iter().enumerate() {
                if i > 0 {
                    ui.separator();
                }
                let paired = self.is_paired(s);
                let pairing = s.adv.is_some_and(|a| a.pairing);
                let supported = s.adv.is_none_or(|a| a.version == protocol::PROTOCOL_VERSION);

                let mut detail = Vec::new();
                if !supported {
                    detail.push(format!(
                        "protocol v{}, not supported",
                        s.adv.map_or(0, |a| a.version)
                    ));
                } else if pairing {
                    detail.push("ready to pair".to_owned());
                } else if paired {
                    detail.push("paired".to_owned());
                } else {
                    detail.push("not paired".to_owned());
                }
                if let Some(rssi) = s.rssi {
                    detail.push(format!("{rssi} dBm"));
                }

                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new(&s.name).strong());
                        ui.label(RichText::new(capitalize(&detail.join(" · "))).small().weak());
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // In pairing mode the app pairs afresh, even if it was paired before.
                        let label = if pairing { "Pair" } else { "Connect" };
                        let button = ui
                            .add_enabled(supported && (pairing || paired), egui::Button::new(label))
                            .on_disabled_hover_text(
                                "Not paired with this computer. Hold BOOT on the sensor for 2 s \
                                 to put it in pairing mode.",
                            );
                        if button.clicked() {
                            connect_to = Some(s.id.clone());
                        }
                    });
                });
            }
        });
        if let Some(id) = connect_to {
            self.connect(&id);
        }
    }
}

fn title(ui: &mut Ui, title: &str, subtitle: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(RichText::new(title).heading().strong());
        ui.label(RichText::new(subtitle).weak());
    });
}

fn connecting_card(ui: &mut Ui, name: &str) {
    card(ui, "Connecting", |ui| {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(format!("Connecting to {name}…"));
        });
        ui.label(
            RichText::new(format!(
                "If asked for a PIN, enter the sensor's PIN ({} unless it was changed).",
                protocol::format_pin(protocol::DEFAULT_PASSKEY)
            ))
            .weak(),
        );
    });
}

fn pairing_help_card(ui: &mut Ui) {
    card(ui, "Pairing a new sensor", |ui| {
        let steps = [
            "Hold BOOT on the sensor for 2 seconds, until the blue LED blinks fast.".to_owned(),
            "Press Pair next to the sensor above (within 60 seconds).".to_owned(),
            format!(
                "Enter the sensor's PIN ({} unless it was changed). Forgot it? Holding BOOT \
                 for 10 s resets it and unpairs all computers.",
                protocol::format_pin(protocol::DEFAULT_PASSKEY)
            ),
        ];
        for (i, step) in steps.iter().enumerate() {
            ui.horizontal_top(|ui| {
                ui.label(RichText::new(format!("{}", i + 1)).strong());
                ui.add(egui::Label::new(RichText::new(step).weak()).wrap());
            });
        }
    });
}

// ---- Formatting ----

/// 1234567 → "1 234 567" (thin spaces, so it reads well at any size).
fn group_digits(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push('\u{2009}');
        }
        out.push(c);
    }
    out
}

fn uptime(ms: u32) -> String {
    let s = ms / 1000;
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

// ---- Look ----

fn theme_pref(dark: bool) -> ThemePreference {
    if dark {
        ThemePreference::Dark
    } else {
        ThemePreference::Light
    }
}

/// IBM Plex Sans, compiled into the binary so release builds don't depend on installed fonts.
fn install_fonts(ctx: &egui::Context) {
    ctx.add_font(egui::epaint::text::FontInsert::new(
        "IBMPlexSans-Regular",
        egui::FontData::from_static(include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf")),
        vec![egui::epaint::text::InsertFontFamily {
            family: FontFamily::Proportional,
            priority: egui::epaint::text::FontPriority::Highest,
        }],
    ));
}

/// Shared type scale and shapes, plus a grayscale palette per theme (no accent color).
fn install_style(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        use FontFamily::{Monospace, Proportional};
        style.text_styles = [
            (TextStyle::Heading, FontId::new(28.0, Proportional)),
            (TextStyle::Body, FontId::new(15.0, Proportional)),
            (TextStyle::Button, FontId::new(15.0, Proportional)),
            (TextStyle::Small, FontId::new(12.0, Proportional)),
            (TextStyle::Monospace, FontId::new(13.0, Monospace)),
        ]
        .into();
        let s = &mut style.spacing;
        s.item_spacing = egui::vec2(10.0, 10.0);
        s.button_padding = egui::vec2(16.0, 7.0);
        s.interact_size.y = 32.0;
        let w = &mut style.visuals.widgets;
        for wv in [
            &mut w.noninteractive,
            &mut w.inactive,
            &mut w.hovered,
            &mut w.active,
            &mut w.open,
        ] {
            wv.corner_radius = CornerRadius::same(8);
        }
        w.hovered.expansion = 0.0;
        w.active.expansion = 0.0;
    });
    ctx.style_mut_of(Theme::Dark, |style| {
        let v = &mut style.visuals;
        v.panel_fill = Color32::from_gray(18);
        v.window_fill = Color32::from_gray(26);
        v.window_stroke = Stroke::new(1.0, Color32::from_gray(40));
        v.extreme_bg_color = Color32::from_gray(14);
        v.selection.bg_fill = Color32::from_gray(80);
        v.selection.stroke = Stroke::new(1.0, Color32::from_gray(230));
    });
    ctx.style_mut_of(Theme::Light, |style| {
        let v = &mut style.visuals;
        v.panel_fill = Color32::from_gray(242);
        v.window_fill = Color32::WHITE;
        v.window_stroke = Stroke::new(1.0, Color32::from_gray(222));
        v.extreme_bg_color = Color32::from_gray(248);
        v.selection.bg_fill = Color32::from_gray(200);
        v.selection.stroke = Stroke::new(1.0, Color32::from_gray(20));
    });
}

fn centered_column(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui)) {
    let avail = ui.available_width();
    let width = avail.min(CONTENT_MAX_WIDTH);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(((avail - width) / 2.0).max(0.0));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.set_width(width);
            add_contents(ui);
        });
    });
}

fn card(ui: &mut Ui, title: &str, add_contents: impl FnOnce(&mut Ui)) {
    let v = ui.visuals();
    egui::Frame::new()
        .fill(v.window_fill)
        .stroke(v.window_stroke)
        .corner_radius(12)
        .inner_margin(18)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(title.to_uppercase())
                    .small()
                    .weak()
                    .extra_letter_spacing(1.2),
            );
            ui.add_space(2.0);
            add_contents(ui);
        });
}

/// Height of the chart card under the series card (the chart itself gets what's left of it).
const CHART_CARD_H: f32 = 210.0;

/// A "nice" step (1, 2 or 5 × 10ⁿ) that splits `range` into at most `max_steps` steps.
fn nice_step(range: f32, max_steps: usize) -> f32 {
    let mut magnitude = 10f32.powf((range.max(1e-6) / max_steps as f32).log10().floor());
    loop {
        for m in [1.0, 2.0, 5.0] {
            let step = m * magnitude;
            if range / step <= max_steps as f32 + 1e-3 {
                return step;
            }
        }
        magnitude *= 10.0;
    }
}

/// Number of shots per score: bars per ring for whole-ring scoring, points per tenth for
/// decimal scoring. Both axes fit the data. Fills the rest of `ui`.
fn score_chart(ui: &mut Ui, scores: &[Score], decimal: bool) {
    let size = egui::vec2(ui.available_width(), ui.available_height().max(110.0));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    let font = TextStyle::Small.resolve(ui.style());
    let (text, weak) = (visuals.text_color(), visuals.weak_text_color());
    let grid = visuals.widgets.noninteractive.bg_stroke;

    // Shots per score, in tenths (a whole ring n is n * 10).
    let mut counts = std::collections::BTreeMap::<u16, u32>::new();
    for s in scores {
        *counts
            .entry(if decimal { s.tenths } else { s.ring as u16 * 10 })
            .or_default() += 1;
    }
    let (Some((&lo, _)), Some((&hi, _))) = (counts.first_key_value(), counts.last_key_value()) else {
        return;
    };
    let max_count = counts.values().copied().max().unwrap_or(1);

    // Y: 0 up to a nice step above the highest count.
    let y_step = nice_step(max_count as f32, 4).max(1.0);
    let y_max = (max_count as f32 / y_step).ceil() * y_step;
    let label_w = painter
        .layout_no_wrap(format!("{y_max}"), font.clone(), weak)
        .size()
        .x;
    let label_h = font.size + 4.0;
    let plot = egui::Rect::from_min_max(
        rect.min + egui::vec2(label_w + 8.0, label_h / 2.0),
        rect.max - egui::vec2(10.0, label_h + 2.0),
    );
    let y_at = |count: f32| plot.bottom() - count / y_max * plot.height();
    let mut y = 0.0;
    while y <= y_max + 1e-3 {
        let py = y_at(y);
        painter.hline(plot.x_range(), py, grid);
        painter.text(
            egui::pos2(plot.left() - 6.0, py),
            egui::Align2::RIGHT_CENTER,
            format!("{y}"),
            font.clone(),
            weak,
        );
        y += y_step;
    }

    let hover = response.hover_pos();
    let tip = if decimal {
        // X: whole rings around the data (a whole top score is its own end), ticks at a
        // nice step of tenths.
        let x_lo = (lo / 10 * 10) as f32;
        let x_hi = if hi % 10 == 0 && hi > lo {
            hi
        } else {
            (hi / 10 + 1) * 10
        } as f32;
        let x_at = |t: f32| plot.left() + (t - x_lo) / (x_hi - x_lo) * plot.width();
        let x_step = nice_step(x_hi - x_lo, 8).max(1.0);
        let mut t = x_lo;
        while t <= x_hi + 1e-3 {
            let px = x_at(t);
            painter.vline(
                px,
                plot.y_range(),
                Stroke::new(grid.width, grid.color.gamma_multiply(0.5)),
            );
            let label = if x_step >= 10.0 {
                format!("{}", t / 10.0)
            } else {
                format!("{:.1}", t / 10.0)
            };
            painter.text(
                egui::pos2(px, plot.bottom() + 3.0),
                egui::Align2::CENTER_TOP,
                label,
                font.clone(),
                weak,
            );
            t += x_step;
        }
        let nearest = hover.and_then(|p| {
            counts
                .iter()
                .map(|(&t, &n)| (t, n, egui::pos2(x_at(t as f32), y_at(n as f32)).distance(p)))
                .filter(|&(_, _, d)| d <= 10.0)
                .min_by(|a, b| a.2.total_cmp(&b.2))
        });
        for (&t, &n) in &counts {
            let hovered = nearest.is_some_and(|(nt, _, _)| nt == t);
            let r = if hovered { 5.0 } else { 3.5 };
            painter.circle_filled(egui::pos2(x_at(t as f32), y_at(n as f32)), r, text);
        }
        nearest.map(|(t, n, _)| format!("{}.{}: {n} shot{}", t / 10, t % 10, if n == 1 { "" } else { "s" }))
    } else {
        // X: one bar per ring, from the lowest to the highest ring shot.
        let (lo, hi) = (lo / 10, hi / 10);
        let slots = (hi - lo + 1) as f32;
        let slot_w = plot.width() / slots;
        let hovered = hover
            .filter(|p| plot.x_range().contains(p.x))
            .map(|p| lo + ((p.x - plot.left()) / slot_w).floor() as u16);
        for ring in lo..=hi {
            let n = counts.get(&(ring * 10)).copied().unwrap_or(0);
            let cx = plot.left() + (ring - lo) as f32 * slot_w + slot_w / 2.0;
            let bar_w = (slot_w * 0.6).min(36.0);
            let fill = if hovered == Some(ring) {
                text
            } else {
                text.gamma_multiply(0.6)
            };
            if n > 0 {
                let bar = egui::Rect::from_min_max(
                    egui::pos2(cx - bar_w / 2.0, y_at(n as f32)),
                    egui::pos2(cx + bar_w / 2.0, plot.bottom()),
                );
                painter.rect_filled(bar, 2.0, fill);
            }
            painter.text(
                egui::pos2(cx, plot.bottom() + 3.0),
                egui::Align2::CENTER_TOP,
                ring.to_string(),
                font.clone(),
                weak,
            );
        }
        hovered.filter(|r| (lo..=hi).contains(r)).map(|r| {
            let n = counts.get(&(r * 10)).copied().unwrap_or(0);
            let what = if r == 0 {
                "Miss".to_string()
            } else {
                format!("Ring {r}")
            };
            format!("{what}: {n} shot{}", if n == 1 { "" } else { "s" })
        })
    };
    painter.hline(plot.x_range(), plot.bottom(), Stroke::new(1.0, weak));
    if let Some(tip) = tip {
        response.on_hover_text_at_pointer(tip);
    }
}

/// A heading inside a card, for a card made of several sections.
fn section_title(ui: &mut Ui, title: &str) {
    ui.label(RichText::new(title).strong());
}

/// A titled section inside a card.
fn section(ui: &mut Ui, title: &str, add_contents: impl FnOnce(&mut Ui)) {
    section_title(ui, title);
    add_contents(ui);
}

/// A message above the cards in an outlined box. Returns true when dismissed.
fn notice(ui: &mut Ui, text: &str) -> bool {
    let stroke = Stroke::new(1.0, ui.visuals().text_color());
    egui::Frame::new()
        .stroke(stroke)
        .corner_radius(12)
        .inner_margin(egui::Margin::symmetric(18, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                let close = ui
                    .add(egui::Button::new("✕").frame(false))
                    .on_hover_text("Dismiss");
                ui.add(egui::Label::new(text).wrap());
                close.clicked()
            })
            .inner
        })
        .inner
}

/// An on/off switch. Grayscale like the rest: a dark track with a light knob when on.
fn toggle_switch(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let size = egui::vec2(44.0, 24.0);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, ui.is_enabled(), *on, ""));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool(response.id, *on);
        let v = ui.visuals();
        let radius = rect.height() / 2.0;
        let (track, knob) = if *on {
            (v.strong_text_color(), v.window_fill)
        } else {
            (v.widgets.inactive.bg_fill, v.strong_text_color())
        };
        let stroke = if response.hovered() {
            v.widgets.hovered.bg_stroke
        } else {
            v.window_stroke
        };
        ui.painter()
            .rect(rect, radius, track, stroke, egui::StrokeKind::Inside);
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        ui.painter()
            .circle_filled(egui::pos2(x, rect.center().y), radius - 4.0, knob);
    }
    response
}

/// The calibration window's task (step 5d).
enum CalTask {
    Calibrate(Box<Wizard>),
    Check(FieldCheck),
}

impl CalTask {
    /// The link dropped: nothing more can be sent; the sensor leaves calibration mode by
    /// itself only on a reboot, so say so.
    fn lost(&mut self) {
        match self {
            CalTask::Calibrate(w) if !matches!(w.phase, Phase::Done { .. }) => {
                w.phase = Phase::Failed(
                    "The sensor disconnected. When it's back, start again (the sensor may still be in calibration mode: \
                     cancelling a new run or a field check turns it off)."
                        .into(),
                );
                w.take_actions();
            }
            CalTask::Check(c) => {
                c.done = true;
                c.take_actions();
            }
            _ => {}
        }
    }
}

impl SensorApp {
    fn open_calibration(&mut self) {
        let (Some(st), Some((cal, _))) = (&self.status, &self.config) else {
            return;
        };
        self.cal_report = None;
        match Wizard::new(calwizard::grid_template(), st, cal, false) {
            Ok(w) => self.cal = Some(CalTask::Calibrate(Box::new(w))),
            Err(e) => self.message = Some(e),
        }
    }

    /// The window's close button: cancels a run (calibration mode off).
    fn close_calibration(&mut self) {
        match &mut self.cal {
            Some(CalTask::Calibrate(w)) => w.cancel(),
            Some(CalTask::Check(c)) => c.cancel(),
            None => {}
        }
        self.run_cal_actions();
        self.cal = None;
    }

    fn calibration_body(&mut self, ui: &mut Ui) {
        let sensor = self.sensor_name.clone();
        match &mut self.cal {
            Some(CalTask::Calibrate(_)) => self.wizard_ui(ui, &sensor),
            Some(CalTask::Check(_)) => self.check_ui(ui),
            None => {}
        }
    }

    fn wizard_ui(&mut self, ui: &mut Ui, sensor: &str) {
        let Some(CalTask::Calibrate(w)) = &mut self.cal else {
            return;
        };
        title(
            ui,
            "Calibration",
            &format!("{} · template \"{}\"", sensor, w.template.name),
        );
        ui.add_space(10.0);
        let mut close = false;
        let mut save: Option<(String, u32)> = None;
        match w.phase.clone() {
            Phase::Ready => {
                card(ui, "Before you start", |ui| {
                    ui.add(egui::Label::new(&w.template.description).wrap());
                    ui.label(format!(
                        "{} points, {} sparks each ({} in all). Put the spark gap in each point's hole in turn and make the sparks; \
                         the sensor is in calibration mode meanwhile, so nothing is scored.",
                        w.template.points.len(),
                        w.template.sparks_per_point,
                        w.template.points.len() * w.template.sparks_per_point
                    ));
                    template_map(ui, &w.template, None, &w.sparks);
                    ui.horizontal(|ui| {
                        if ui.button("Start").clicked() {
                            w.start();
                        }
                        close = ui.button("Cancel").clicked();
                    });
                });
            }
            Phase::Collecting { point } => {
                let p = w.template.points[point];
                let got = w.sparks[point].len();
                let want = w.template.sparks_per_point;
                card(
                    ui,
                    &format!("Point {} of {}", point + 1, w.template.points.len()),
                    |ui| {
                        ui.label(
                            RichText::new(format!("({:+.0}, {:+.0}) mm", p.0, p.1))
                                .size(26.0)
                                .strong(),
                        );
                        ui.label("Put the spark gap in this point's hole and make sparks.");
                        ui.add(
                            egui::ProgressBar::new(got as f32 / want as f32)
                                .text(format!("{got} of {want} sparks")),
                        );
                        let (have, all) = w.progress();
                        let mut detail = format!("{have} of {all} sparks in all");
                        if w.ignored > 0 {
                            detail += &format!(
                                " · {} not counted (another point, a channel without both edges, or other settings)",
                                w.ignored
                            );
                        }
                        ui.add(egui::Label::new(RichText::new(detail).weak()).wrap());
                        template_map(ui, &w.template, Some(point), &w.sparks);
                        ui.horizontal(|ui| {
                            let last = point + 1 == w.template.points.len();
                            let next = if last { "Fit" } else { "Next point" };
                            if ui.add_enabled(w.point_done(), egui::Button::new(next)).clicked() {
                                w.next_point();
                            }
                            if ui
                                .add_enabled(got > 0, egui::Button::new("Redo this point"))
                                .clicked()
                            {
                                w.redo_point();
                            }
                            if ui.button("Cancel").clicked() {
                                w.cancel();
                            }
                        });
                    },
                );
            }
            Phase::Fitted(r) => {
                card(ui, "The fit", |ui| {
                    ui.label(format!(
                        "{} sparks at {} points, {} left out as outliers · residual {:.1} ns",
                        r.sparks,
                        r.points.len(),
                        r.outliers,
                        r.rms_ns
                    ));
                    ui.label(
                        RichText::new(format!(
                            "Position error: {:.3} mm rms before, {:.3} mm after",
                            r.before_rms_mm, r.after_rms_mm
                        ))
                        .strong(),
                    );
                    for warn in fit_warnings(&r) {
                        ui.label(RichText::new(format!("⚠ {warn}")).strong());
                    }
                    ui.add_space(6.0);
                    egui::Grid::new("fit_channels").striped(true).show(ui, |ui| {
                        for h in ["", "x mm", "y mm", "k", "offset ns", "sigma ns"] {
                            ui.label(RichText::new(h).weak());
                        }
                        ui.end_row();
                        for c in &r.channels {
                            ui.label(format!("CH{}", c.channel + 1));
                            ui.label(format!("{:.3} ±{:.3}", c.x, c.sd_x));
                            ui.label(format!("{:.3} ±{:.3}", c.y, c.sd_y));
                            ui.label(format!("{:.4} ±{:.4}", c.k, c.sd_k));
                            ui.label(format!("{:.1} ±{:.1}", c.offset_ns, c.sd_offset));
                            ui.label(format!("{:.1}", c.sigma_ns));
                            ui.end_row();
                        }
                    });
                    ui.add_space(6.0);
                    egui::CollapsingHeader::new("Error per point").show(ui, |ui| {
                        egui::Grid::new("fit_points").striped(true).show(ui, |ui| {
                            for h in ["point mm", "before mean, worst", "after mean, worst"] {
                                ui.label(RichText::new(h).weak());
                            }
                            ui.end_row();
                            for p in &r.points {
                                ui.label(format!("({:+.0}, {:+.0})", p.x, p.y));
                                ui.label(format!("{:.3}, {:.3}", p.before_mean, p.before_worst));
                                ui.label(format!("{:.3}, {:.3}", p.after_mean, p.after_worst));
                                ui.end_row();
                            }
                        });
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Write to the sensor").clicked() {
                            save = w.report(sensor).map(|t| (t, r.id));
                            let date = eventlog::utc_date().replace('-', "").parse().unwrap_or(0);
                            w.write(date);
                        }
                        if ui.button("Discard").clicked() {
                            w.cancel();
                        }
                    });
                });
            }
            Phase::Writing { expected } => {
                card(ui, "Writing", |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!(
                            "Waiting for the sensor to confirm calibration {expected:08X}…"
                        ));
                    });
                });
            }
            Phase::Done { id } => {
                card(ui, "Done", |ui| {
                    ui.label(RichText::new(format!("The sensor holds calibration {id:08X} now.")).strong());
                    ui.label("Calibration mode is off again. A field check is a good next step.");
                    close = ui.button("Close").clicked();
                });
            }
            Phase::Failed(why) => {
                card(ui, "Stopped", |ui| {
                    ui.add(egui::Label::new(why).wrap());
                    close = ui.button("Close").clicked();
                });
            }
            Phase::Cancelled => {
                card(ui, "Cancelled", |ui| {
                    ui.label("Nothing was written; calibration mode is off.");
                    close = ui.button("Close").clicked();
                });
            }
        }
        if let Some((text, id)) = save {
            self.save_cal_report(&text, id);
        }
        if let Some(r) = &self.cal_report {
            ui.add(egui::Label::new(RichText::new(r).small().weak()).wrap());
        }
        if close {
            self.cal = None;
        }
    }

    fn check_ui(&mut self, ui: &mut Ui) {
        let Some(CalTask::Check(c)) = &mut self.cal else {
            return;
        };
        title(
            ui,
            "Field check",
            "Sparks at the template's check point, solved by the sensor",
        );
        ui.add_space(10.0);
        let mut close = false;
        card(
            ui,
            &format!("Check point ({:+.0}, {:+.0}) mm", c.point.0, c.point.1),
            |ui| match (c.done, c.result()) {
                (true, Some(r)) => {
                    let verdict = if r.pass {
                        format!("Within {:.2} mm: the calibration holds.", c.limit_mm)
                    } else {
                        format!("More than {:.2} mm off: calibrate again.", c.limit_mm)
                    };
                    ui.label(RichText::new(verdict).size(20.0).strong());
                    ui.label(format!(
                        "{} sparks: mean {:.3} mm, worst {:.3} mm from the point.",
                        r.sparks, r.mean_mm, r.worst_mm
                    ));
                    close = ui.button("Close").clicked();
                }
                (true, None) => {
                    ui.label("Stopped.");
                    close = ui.button("Close").clicked();
                }
                (false, _) => {
                    ui.label("Put the spark gap in the check point's hole and make sparks. Calibration mode is on: nothing is scored.");
                    let got = c.errors.len();
                    ui.add(
                        egui::ProgressBar::new(got as f32 / c.needed as f32)
                            .text(format!("{got} of {} sparks", c.needed)),
                    );
                    if c.ignored > 0 {
                        ui.label(
                            RichText::new(format!(
                                "{} not counted (another point, or not solved)",
                                c.ignored
                            ))
                            .weak(),
                        );
                    }
                    if ui.button("Cancel").clicked() {
                        c.cancel();
                    }
                }
            },
        );
        if close {
            self.cal = None;
        }
    }
}

/// What to look at before writing a fit.
fn fit_warnings(r: &calib::fit::FitResult) -> Vec<String> {
    let mut w = Vec::new();
    for c in &r.channels {
        if c.sd_k > 0.05 {
            w.push(format!(
                "CH{}'s k is uncertain (±{:.3}): more sparks of varied loudness help.",
                c.channel + 1,
                c.sd_k
            ));
        }
    }
    if r.rms_ns > 150.0 {
        w.push(format!(
            "The residual is high ({:.0} ns): check the mics and the sparks.",
            r.rms_ns
        ));
    }
    if r.after_rms_mm > 0.1 {
        w.push(format!(
            "Shots would still be {:.2} mm off on average.",
            r.after_rms_mm
        ));
    }
    if r.outliers * 10 > r.sparks {
        w.push(format!("{} sparks were left out as outliers.", r.outliers));
    }
    w
}

/// The template's points in the frame: done points filled, the current one ringed.
fn template_map(
    ui: &mut Ui,
    t: &calwizard::Template,
    current: Option<usize>,
    sparks: &[Vec<calib::fit::Spark>],
) {
    let side = ui.available_width().min(280.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let v = ui.visuals();
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, v.weak_text_color()),
        egui::StrokeKind::Inside,
    );
    let half = 125.0f32; // the 250 mm frame
    let to = |p: (f32, f32)| rect.center() + egui::vec2(p.0 / half * side / 2.0, -p.1 / half * side / 2.0);
    // The scoring area (200 mm) and the check point.
    let a = side / 2.0 * 100.0 / half;
    painter.rect_stroke(
        egui::Rect::from_center_size(rect.center(), egui::vec2(2.0 * a, 2.0 * a)),
        0.0,
        Stroke::new(0.5, v.weak_text_color()),
        egui::StrokeKind::Inside,
    );
    painter.circle_stroke(to(t.check_point), 3.0, Stroke::new(1.0, v.weak_text_color()));
    for (i, &p) in t.points.iter().enumerate() {
        let c = to(p);
        let done = sparks.get(i).is_some_and(|s| s.len() >= t.sparks_per_point);
        if done {
            painter.circle_filled(c, 5.0, v.strong_text_color());
        } else {
            painter.circle_stroke(c, 5.0, Stroke::new(1.0, v.text_color()));
        }
        if current == Some(i) {
            painter.circle_stroke(c, 10.0, Stroke::new(2.5, LAST_SHOT_COLOR));
        }
    }
}

/// What the sensor's events said since connecting (protocol v3).
#[derive(Default)]
struct EventCounts {
    total: u32,
    verdicts: [u32; 8],
    holes: u32,
    missed: u32,
    last: Option<String>,
}

impl EventCounts {
    fn add(&mut self, e: &protocol::Event) {
        self.total += 1;
        match e.kind {
            protocol::Kind::Shot => {
                if let Some(v) = self.verdicts.get_mut(e.verdict as usize) {
                    *v += 1;
                }
                let mode = if e.flags & protocol::EV_CAL_MODE != 0 {
                    " (calibration mode)"
                } else {
                    ""
                };
                self.last = Some(format!("{}{mode}", e.verdict_name().unwrap_or("?")));
            }
            protocol::Kind::Hole => {
                self.holes += 1;
                self.last = Some("a hole".into());
            }
            protocol::Kind::Missed => {
                self.missed += 1;
                self.last = Some("missed".into());
            }
        }
    }

    fn text(&self) -> String {
        if self.total == 0 {
            return "No events since connecting.".into();
        }
        let mut parts: Vec<String> = protocol::VERDICTS
            .iter()
            .zip(self.verdicts)
            .filter(|(_, n)| *n > 0)
            .map(|(v, n)| format!("{n} {v}"))
            .collect();
        if self.holes > 0 {
            parts.push(format!(
                "{} hole{}",
                self.holes,
                if self.holes == 1 { "" } else { "s" }
            ));
        }
        if self.missed > 0 {
            parts.push(format!("{} missed", self.missed));
        }
        format!(
            "{} event{} since connecting: {}. Last: {}.",
            self.total,
            if self.total == 1 { "" } else { "s" },
            parts.join(", "),
            self.last.as_deref().unwrap_or("-")
        )
    }
}

/// What the sensor said about a shot (protocol v3): its uncertainty and flags.
fn shot_details(s: &protocol::Shot) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(sigma) = s.sigma_mm() {
        parts.push(format!("±{sigma:.2} mm (1σ)"));
    }
    for (bit, text) in [
        (protocol::SHOT_CURTAIN, "confirmed by the curtain"),
        (protocol::SHOT_DROPPED, "one mic left out"),
        (protocol::SHOT_AIR_ASSUMED, "air partly assumed"),
        (protocol::SHOT_CAL_STALE, "calibration changed since it was made"),
    ] {
        if s.flags & bit != 0 {
            parts.push(text.to_string());
        }
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Resolving a hit by hand (the resolve window, `resolve`).
struct Resolving {
    key: Key,
    step: Step,
    /// Placing: where (mm from the centre, y up), once clicked or typed.
    pos: Option<(f32, f32)>,
    /// The same hole: the shot it went through.
    same: Option<Key>,
    /// A drag that started on the placed shot (moving it, not the card): the shot's
    /// offset from the pointer, points.
    grab: Option<Vec2>,
    /// Opened by selecting a shot resolved by hand: closes when the selection moves on.
    from_selection: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Choose,
    Place,
    Same,
}

impl Resolving {
    fn new(key: Key, from_selection: bool) -> Self {
        Resolving {
            key,
            step: Step::Choose,
            pos: None,
            same: None,
            grab: None,
            from_selection,
        }
    }
}

/// An unscored hit by what it was and where it came: "A hole after shot 4".
fn hit_name(s: &protocol::Shot, after: usize) -> String {
    let what = match s.kind {
        protocol::Kind::Hole => "A hole",
        _ => "A missed shot",
    };
    if after == 0 {
        format!("{what} before shot 1")
    } else {
        format!("{what} after shot {after}")
    }
}

/// A circle with a dashed edge (a shot resolved by hand).
fn dashed_circle(painter: &egui::Painter, c: egui::Pos2, r: f32, stroke: Stroke) {
    let n = 64;
    let pts: Vec<egui::Pos2> = (0..=n)
        .map(|k| {
            let a = k as f32 / n as f32 * std::f32::consts::TAU;
            c + r * egui::vec2(a.cos(), a.sin())
        })
        .collect();
    let dash = (r * std::f32::consts::TAU / 16.0).max(2.0);
    painter.extend(egui::Shape::dashed_line(&pts, stroke, dash * 0.6, dash * 0.4));
}

/// A hit resolved by hand, in the series list.
fn how_label(h: How) -> &'static str {
    match h {
        How::Placed => "placed by hand",
        How::Same(_) => "same hole, by hand",
        How::Miss => "miss, by hand",
    }
}

/// A hit with no score, in the series list.
fn unscored_label(s: &protocol::Shot) -> &'static str {
    match s.kind {
        protocol::Kind::Hole => "hole – not scored",
        _ => "missed – not scored",
    }
}

/// A hit with no score, on the card's notice: where it came in the series, and what it was.
fn unscored_line(after: usize, s: &protocol::Shot) -> String {
    let what = match s.kind {
        protocol::Kind::Hole => "through an existing hole (the curtain saw it, no sound)",
        _ => "missed: a shot in the dead time after the last one, not measured",
    };
    if after == 0 {
        format!("Before shot 1 · {what}")
    } else {
        format!("After shot {after} · {what}")
    }
}

fn calibration_text(st: &protocol::Status, cal: &protocol::Calibration) -> String {
    if st.cal == 0 {
        return "No calibration record: the mics' nominal positions.".into();
    }
    let (y, m, d) = (cal.date / 10_000, cal.date / 100 % 100, cal.date % 100);
    let stale = if st.flags & protocol::ST_CAL_STALE != 0 {
        " · STALE: a value changed since"
    } else {
        ""
    };
    format!(
        "Calibration {:08X} of {y:04}-{m:02}-{d:02} ({} points, {} sparks, residual {:.0} ns){stale}",
        st.cal, cal.points, cal.sparks, cal.res_ns
    )
}

fn settings_text(s: &protocol::Settings) -> String {
    let off = |v: f32, f: &dyn Fn(f32) -> String| if v > 0.0 { f(v) } else { "off".into() };
    let curtain = if s.curtain_hi_ns > 0.0 {
        format!(
            "{:.0}–{:.0} µs",
            s.curtain_lo_ns / 1000.0,
            s.curtain_hi_ns / 1000.0
        )
    } else {
        "off".into()
    };
    format!(
        "Residual limit {:.0} ns, drop-one {} · lost {} · area {} · together {} · pulse {} · curtain {curtain} · span {} mm · dead time {} ms · channels {:02X}",
        s.res_limit_ns,
        if s.drop_one { "on" } else { "off" },
        if s.max_lost < 0 {
            "off".into()
        } else {
            s.max_lost.to_string()
        },
        off(s.area_mm, &|v| format!("±{v:.0} mm")),
        off(s.together_ns, &|v| format!("{:.0} µs", v / 1000.0)),
        off(s.pulse_max_ns, &|v| format!("{:.0} µs", v / 1000.0)),
        s.span_mm,
        s.deadtime_ms,
        s.mask
    )
}

fn air_text(st: &protocol::Status) -> String {
    let all = protocol::MEAS_T | protocol::MEAS_RH | protocol::MEAS_P;
    let how = match st.measured {
        m if m == all => "measured".to_string(),
        0 => "assumed: no air sensor".to_string(),
        m => format!(
            "partly assumed ({} measured)",
            [
                (protocol::MEAS_T, "T"),
                (protocol::MEAS_RH, "RH"),
                (protocol::MEAS_P, "p")
            ]
            .iter()
            .filter(|(b, _)| m & b != 0)
            .map(|(_, n)| *n)
            .collect::<Vec<_>>()
            .join(", ")
        ),
    };
    format!(
        "Air {:.1} °C, {:.0} % RH, {:.0} hPa ({how}) · speed of sound {:.2} m/s",
        st.t_c, st.rh, st.hpa, st.c
    )
}

/// The card photo window's link for the phone (`CARDPHOTO.md` C3).
struct PhotoTask {
    server: Result<upload::UploadServer, String>,
    url: Option<String>,
    qr: Option<(usize, Vec<bool>)>,
    status: String,
    /// The QR code shows: before the first photo, and after "Take another photo".
    show_link: bool,
}

impl PhotoTask {
    /// A new link (the old one stops working).
    fn new_link(&mut self, series: &str) {
        if let Ok(s) = &self.server {
            let url = s.offer(series);
            self.qr = upload::qr(&url).ok();
            self.url = Some(url);
            self.status = "Waiting for the phone…".into();
        }
    }
}

/// What a processing thread starts from.
enum PhotoJob {
    /// Uploaded and stored already.
    Stored(upload::Photo),
    /// A file dropped on the window: stored first.
    Bytes(Vec<u8>),
    /// A synthetic photo of the series (`OPENSIGHTS_DEMO_PHOTO` set).
    Demo,
}

/// A photo being processed: its result, and the sensor's shots it was taken with.
type PhotoJobRx = (mpsc::Receiver<Result<photo::Processed, String>>, Vec<Key>);

/// A processed card photo of the current series (C4).
struct CardPhoto {
    p: photo::Processed,
    texture: Option<egui::TextureHandle>,
    /// The sensor's shots it was checked against, in the order of `p.shots`.
    keys: Vec<Key>,
}

impl CardPhoto {
    fn texture(&mut self, ctx: &egui::Context) -> Option<&egui::TextureHandle> {
        if self.texture.is_none()
            && let Some(img) = self.p.image.take()
        {
            self.texture = Some(ctx.load_texture("card-photo", img, egui::TextureOptions::LINEAR));
        }
        self.texture.as_ref()
    }

    /// The check of the sensor's shot `key`; None for a shot after the photo.
    fn check(&self, key: Key) -> Option<&photo::ShotCheck> {
        self.keys.iter().position(|&k| k == key).and_then(|i| self.p.shots.get(i))
    }

    /// Every hole found (frame mm): the shots' and the others.
    fn holes(&self) -> Vec<(f64, f64)> {
        self.p
            .shots
            .iter()
            .filter_map(|s| s.hole)
            .chain(self.p.holes.iter().copied())
            .collect()
    }
}

impl SensorApp {
    fn photos_dir() -> Option<std::path::PathBuf> {
        eframe::storage_dir(APP_ID).map(|d| d.join("photos"))
    }

    fn series_label(&self) -> String {
        if self.sensor_name.is_empty() {
            "series".into()
        } else {
            self.sensor_name.clone()
        }
    }

    /// Opens the card photo window (or brings it to the front): old originals cleaned up,
    /// the server started, a link made.
    fn open_photo(&mut self, ctx: &egui::Context) {
        if self.photo_task.is_some() {
            ctx.send_viewport_cmd_to(
                egui::ViewportId::from_hash_of("photo"),
                egui::ViewportCommand::Focus,
            );
            return;
        }
        let Some(dir) = Self::photos_dir() else {
            self.message = Some("No folder for the photos on this system.".into());
            return;
        };
        let cleaned = match upload::clean_up(&dir, upload::now_unix()) {
            Ok(r) if !r.is_empty() => format!(
                " ({} photo{} older than {} days removed.)",
                r.len(),
                if r.len() == 1 { "" } else { "s" },
                upload::KEEP_DAYS
            ),
            _ => String::new(),
        };
        let c = ctx.clone();
        let mut task = PhotoTask {
            server: upload::UploadServer::start_lan(dir, move || c.request_repaint()),
            url: None,
            qr: None,
            status: String::new(),
            show_link: self.card_photo.is_none(),
        };
        task.new_link(&self.series_label());
        task.status.push_str(&cleaned);
        self.photo_task = Some(task);
    }

    /// Opens the count window (or brings it to the front).
    fn open_count(&mut self, ctx: &egui::Context) {
        if self.count.is_some() {
            ctx.send_viewport_cmd_to(egui::ViewportId::from_hash_of("count"), egui::ViewportCommand::Focus);
            return;
        }
        let Some(dir) = Self::photos_dir() else {
            self.message = Some("No folder for the photos on this system.".into());
            return;
        };
        let _ = upload::clean_up(&dir, upload::now_unix());
        self.count = Some(count::CountTask::open(ctx, dir));
    }

    /// The server's news and the processing's result.
    fn poll_photo(&mut self, ctx: &egui::Context) {
        let mut received = Vec::new();
        if let Some(t) = &mut self.photo_task
            && let Ok(s) = &t.server
        {
            while let Some(e) = s.try_recv() {
                match e {
                    upload::UploadEvent::Opened { series } => {
                        t.status = format!("The phone opened the page ({series}).")
                    }
                    upload::UploadEvent::Refused { series, reason } => {
                        t.status = format!("Refused ({series}): {reason}")
                    }
                    upload::UploadEvent::Received(p) => {
                        t.status = format!("Received ({:.1} MB).", p.meta.bytes as f64 / 1e6);
                        t.show_link = false;
                        received.push(p);
                    }
                }
            }
        }
        for p in received {
            self.process_photo(PhotoJob::Stored(p), ctx);
        }
        if let Some((rx, keys)) = &self.photo_job {
            match rx.try_recv() {
                Ok(Ok(p)) => {
                    self.card_photo = Some(CardPhoto {
                        p,
                        texture: None,
                        keys: keys.clone(),
                    });
                    if self.photo_fade == 0.0 {
                        self.photo_fade = 0.6;
                    }
                    self.photo_job = None;
                    // Done: the results are in the series panel (an error keeps the window).
                    self.photo_task = None; // stops the server
                }
                Ok(Err(e)) => {
                    self.photo_error = Some(format!("The photo could not be used: {e}."));
                    self.photo_job = None;
                    if let Some(t) = &mut self.photo_task {
                        t.show_link = true;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.photo_error = Some("The processing stopped unexpectedly.".into());
                    self.photo_job = None;
                }
            }
        }
    }

    /// Processes a photo on a thread of its own, against the series' shots as they are now.
    fn process_photo(&mut self, job: PhotoJob, ctx: &egui::Context) {
        let Some(dir) = Self::photos_dir() else {
            return;
        };
        // The sensor's shots only: one placed by hand isn't a measurement.
        let shots: Vec<(f64, f64)> = self
            .series
            .iter()
            .filter(|(s, _)| s.scored())
            .map(|(s, _)| (f64::from(s.x_mm()), f64::from(s.y_mm())))
            .collect();
        let series = self.series_label();
        let (tx, rx) = mpsc::channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let card = card::air_pistol();
            let run = || -> Result<photo::Processed, String> {
                let now = upload::now_unix();
                let photo = match job {
                    PhotoJob::Stored(p) => p,
                    PhotoJob::Bytes(b) => upload::store(&dir, &series, &b, now)?,
                    PhotoJob::Demo => {
                        let b = photo::demo_photo(&card, &shots, 0.1, now)?;
                        upload::store(&dir, &format!("{series}-demo"), &b, now)?
                    }
                };
                photo::process(photo, &card, &shots)
            };
            let _ = tx.send(run());
            c.request_repaint();
        });
        self.photo_job = Some((rx, self.measured_keys()));
        self.photo_error = None;
    }

    fn photo_body(&mut self, ui: &mut Ui) {
        title(
            ui,
            "Card photo",
            "The shot card as the ground truth for the sensor",
        );
        ui.add_space(14.0);
        let dropped: Vec<Vec<u8>> = ui.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.bytes().ok())
                .collect()
        });
        if let Some(b) = dropped.into_iter().next() {
            self.process_photo(PhotoJob::Bytes(b), ui.ctx());
        }
        if let Some(e) = self.photo_error.clone()
            && notice(ui, &e)
        {
            self.photo_error = None;
        }
        let (mut new_link, mut demo) = (false, false);
        let no_photo = self.card_photo.is_none() && self.photo_job.is_none();
        if let Some(t) = &self.photo_task
            && (t.show_link || no_photo)
        {
            card(ui, "From the phone", |ui| {
                match &t.server {
                    Err(e) => {
                        ui.label(format!("No link for the phone: {e}."));
                    }
                    Ok(_) => {
                        ui.add(
                            egui::Label::new(
                                "Scan the code with the phone's camera and take the photo. The phone \
                                 must be on the same Wi-Fi as this computer.",
                            )
                            .wrap(),
                        );
                        ui.add_space(8.0);
                        if let Some((w, m)) = &t.qr {
                            qr_code(ui, *w, m, 230.0);
                        }
                        if let Some(u) = &t.url {
                            ui.label(RichText::new(u).small().weak());
                        }
                        ui.add_space(4.0);
                        ui.add(egui::Label::new(RichText::new(&t.status).strong()).wrap());
                        new_link = ui
                            .add(egui::Button::new("New link").frame(false))
                            .on_hover_text("The old link stops working.")
                            .clicked();
                    }
                }
                ui.add_space(4.0);
                ui.label(RichText::new("Or drop a photo file on this window.").weak());
                if std::env::var_os("OPENSIGHTS_DEMO_PHOTO").is_some() {
                    demo = ui.button("Demo: a synthetic photo of this series").clicked();
                }
            });
        }
        if self.photo_job.is_some() {
            card(ui, "Processing", |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Finding the rings and the holes (a few seconds)…");
                });
            });
        }
        if new_link {
            let label = self.series_label();
            if let Some(t) = &mut self.photo_task {
                t.new_link(&label);
            }
        }
        if demo {
            self.process_photo(PhotoJob::Demo, ui.ctx());
        }
    }

}

/// A row of the series list: clickable (it selects the shot), highlighted when selected.
/// `bg`: a placeholder shape added before the row, so the highlight lies under its text.
const SHOW_TIP: &str = "Show it on the card";
const RESOLVE_TIP: &str = "Not scored: place it on the card, mark it the same hole as another shot, or count it a miss";

/// `label` names the row for screen readers (and the UI tests).
#[allow(clippy::too_many_arguments)]
fn row_click(
    ui: &Ui,
    rect: egui::Rect,
    bg: egui::layers::ShapeIdx,
    key: (u32, u32),
    selected: bool,
    tip: &str,
    label: &str,
) -> bool {
    let rect = rect.expand2(egui::vec2(4.0, 1.0));
    let r = ui
        .interact(rect, ui.scope_id().with(("series_row", key)), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    if selected || r.hovered() {
        let fill = if selected {
            ui.visuals().selection.bg_fill
        } else {
            ui.visuals().widgets.hovered.weak_bg_fill
        };
        ui.painter().set(bg, egui::Shape::rect_filled(rect, 4.0, fill));
    }
    r.clicked()
}

/// The card photo's result above the series table: the sensor against the holes, warnings,
/// and the photo's details folded away.
fn photo_summary(ui: &mut Ui, cp: &CardPhoto, series: &[(protocol::Shot, Score)]) {
    let p = &cp.p;
    // A shot's number in the series now.
    let number = |key: Key| series.iter().position(|(s, _)| resolve::key(s) == key).map(|i| i + 1);
    if let Some(rms) = p.rms_mm {
        let found = p.shots.iter().filter(|s| s.diff.is_some()).count();
        let mut line = format!("Card photo: {rms:.3} mm rms, the sensor against the holes, {found} of {} shots", p.shots.len());
        if let Some((i, d)) = p.worst() {
            match cp.keys.get(i).and_then(|&k| number(k)) {
                Some(n) => line.push_str(&format!(" · worst {d:.3} mm (shot {n})")),
                None => line.push_str(&format!(" · worst {d:.3} mm")),
            }
        }
        ui.add(egui::Label::new(RichText::new(line).strong()).wrap());
    } else {
        let n = p.holes.len();
        ui.label(format!("Card photo: no shots in the series, {n} hole{} found.", if n == 1 { "" } else { "s" }));
    }
    let measured: Vec<Key> = series
        .iter()
        .filter(|(s, _)| s.scored())
        .map(|(s, _)| resolve::key(s))
        .collect();
    if cp.keys != measured {
        ui.add(
            egui::Label::new(format!(
                "⚠ Processed with {} of the sensor's shots; the series has {} now. Take another photo.",
                cp.keys.len(),
                measured.len()
            ))
            .wrap(),
        );
    }
    for w in p.warnings() {
        ui.add(egui::Label::new(format!("⚠ {w}")).wrap());
    }
    egui::CollapsingHeader::new(RichText::new("The photo's details").small().weak())
        .id_salt("photo_details")
        .show(ui, |ui| {
            let mut details = format!(
                "{:.1} px per mm; the rings fit to {:.3} mm rms ({} of {} edge points); lens k1 {:+.3}. \
                 Taken {:.0}° from straight above: the holes expected to within about {:.2} mm (rms).",
                p.px_per_mm, p.reg_rms_mm, p.reg_points.0, p.reg_points.1, p.k1, p.camera_deg, p.expected_mm
            );
            if let Some(pl) = &p.placement {
                let c = p.pose.centre;
                details.push_str(&format!(
                    " The card in the holder: its centre at {:+.2}, {:+.2} mm from the sensor's, turned {:+.2}°{}; \
                     the app's card is drawn there, the shots where the sensor put them (scored from the sensor's \
                     centre; the Paper column gauges the holes on the print).",
                    c.0,
                    c.1,
                    p.pose.angle_deg(),
                    if p.scaled {
                        format!(", printed at {:.4} of the sensor's scale", 1.0 / pl.scale)
                    } else {
                        String::new()
                    }
                ));
            }
            details.push_str(&format!(" Processed in {:.1} s.", p.seconds));
            ui.add(egui::Label::new(RichText::new(details).small().weak()).wrap());
        });
}

/// A texture on a quadrilateral (corners from its top left, clockwise): the printed card,
/// turned where the real card hung.
fn paint_quad(painter: &egui::Painter, tex: egui::TextureId, corners: [egui::Pos2; 4]) {
    let mut mesh = egui::Mesh::with_texture(tex);
    let uvs = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    for (pos, uv) in corners.into_iter().zip(uvs) {
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv: egui::pos2(uv.0, uv.1),
            color: Color32::WHITE,
        });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// "Count card" with a small target, in the status bar: opens the count window.
fn count_button(ui: &mut Ui) -> egui::Response {
    let font = TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap("Count card".into(), font, Color32::PLACEHOLDER);
    let (icon, gap, pad) = (16.0, 6.0, egui::vec2(6.0, 2.0));
    let size = egui::vec2(icon + gap + galley.size().x, galley.size().y.max(icon)) + 2.0 * pad;
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let visuals = ui.visuals();
    let color = visuals.widgets.style(&response).fg_stroke.color;
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, 4.0, visuals.widgets.hovered.weak_bg_fill);
    }
    let c = egui::pos2(rect.left() + pad.x + icon / 2.0, rect.center().y);
    painter.circle_stroke(c, 7.0, Stroke::new(1.2, color));
    painter.circle_stroke(c, 4.2, Stroke::new(1.2, color));
    painter.circle_filled(c, 1.8, color);
    let text_at = egui::pos2(c.x + icon / 2.0 + gap, rect.center().y - galley.size().y / 2.0);
    painter.galley(text_at, galley, color);
    response.on_hover_text("Count a card from a photo: its holes and its score, apart from the series.")
}

/// "Card photo", under the series.
fn photo_button(ui: &mut Ui) -> bool {
    ui.add(egui::Button::new("Card photo").frame(false))
        .on_hover_text("A photo of the shot card from the phone: the holes against the shots.")
        .clicked()
}

/// A QR code, `size` points wide with its quiet zone, black on white in either theme.
fn qr_code(ui: &mut Ui, w: usize, dark: &[bool], size: f32) {
    let n = w + 8; // 4 modules of quiet zone each side
    let ppp = ui.ctx().pixels_per_point();
    // Whole pixels per module, so the modules don't blur into each other.
    let cell = ((size * ppp / n as f32).floor().max(1.0)) / ppp;
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(cell * n as f32), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, Color32::WHITE);
    for y in 0..w {
        let mut x = 0;
        while x < w {
            if !dark[y * w + x] {
                x += 1;
                continue;
            }
            let start = x;
            while x < w && dark[y * w + x] {
                x += 1;
            }
            let min = rect.min + Vec2::new((start + 4) as f32, (y + 4) as f32) * cell;
            let run = egui::Rect::from_min_size(min, Vec2::new((x - start) as f32 * cell, cell));
            painter.rect_filled(run, 0.0, Color32::BLACK);
        }
    }
}

/// Card · track · Photo: how much of the photo shows over the printed card. Returns the new
/// value when it was changed.
fn fade_control(ui: &Ui, painter: &egui::Painter, bar: egui::Rect, fade: f32) -> Option<f32> {
    control_bar(ui, painter, bar, "fade_bar");
    let label_w = 48.0;
    let font = TextStyle::Small.resolve(ui.style());
    let color = ui.visuals().text_color();
    let left = bar_slot(bar, 0.0, label_w);
    let right = bar_slot(bar, bar.width() - 2.0 * BAR_PAD - label_w, label_w);
    painter.text(
        left.center(),
        egui::Align2::CENTER_CENTER,
        "Card",
        font.clone(),
        color,
    );
    painter.text(right.center(), egui::Align2::CENTER_CENTER, "Photo", font, color);
    let track = egui::Rect::from_min_max(
        egui::pos2(left.right() + 6.0, left.top()),
        egui::pos2(right.left() - 6.0, left.bottom()),
    );
    let response = ui
        .interact(
            track,
            ui.scope_id().with("fade_track"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_text("The photo over the printed card");
    let mut new = None;
    if (response.dragged() || response.clicked())
        && let Some(p) = response.interact_pointer_pos()
    {
        new = Some(((p.x - track.left()) / track.width()).clamp(0.0, 1.0));
    }
    let t = new.unwrap_or(fade);
    let y = track.center().y;
    let v = ui.visuals();
    painter.line_segment(
        [egui::pos2(track.left(), y), egui::pos2(track.right(), y)],
        Stroke::new(3.0, v.widgets.inactive.bg_fill),
    );
    let x = egui::lerp(track.left()..=track.right(), t);
    painter.circle(
        egui::pos2(x, y),
        7.0,
        v.widgets.style(&response).fg_stroke.color,
        Stroke::NONE,
    );
    new.filter(|n| *n != fade)
}

/// Opens a folder in the system's file manager.
fn open_folder(dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(dir);
    let program = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(dir).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    /// The resolve window driven as a user would: a hole placed on the card (a click, a
    /// drag of the placed shot, the arrow keys, Confirm), the missed shot counted a miss,
    /// the other hole the same hole as a shot (picked on the card), then the first one
    /// changed and put back to "not scored". Checks the series, the order, the scores and
    /// the log's `manual` lines. `UI_SHOTS=<dir>` saves an image of each step.
    #[test]
    fn resolve_hits_by_hand() {
        let logs = std::env::temp_dir().join(format!("opensights-ui-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&logs);
        let log_dir = logs.clone();
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_eframe(move |cc| {
                // No BLE thread (CI has no Bluetooth; macOS would stop the test).
                let link = (tokio::sync::mpsc::unbounded_channel().0, mpsc::channel().1);
                let mut app = SensorApp::with_link(cc, link).demo_series();
                app.event_log = Some(EventLog::new(log_dir));
                app.save_events = true;
                app
            });
        let shots_dir = std::env::var_os("UI_SHOTS").map(std::path::PathBuf::from);
        let step = |h: &mut egui_kittest::Harness<SensorApp>, name: &str| {
            h.run_steps(4);
            if let Some(dir) = &shots_dir {
                let img = h.render().expect("render");
                img.save(dir.join(format!("{name}.png"))).unwrap();
            }
        };
        step(&mut h, "0-start");
        let seqs = |app: &SensorApp| app.series.iter().map(|(s, _)| s.seq).collect::<Vec<_>>();
        assert_eq!(seqs(h.state()), [1, 2, 4, 5, 7, 9]);
        assert_eq!(h.state().unscored.len(), 3);
        let total = |app: &SensorApp| app.series.iter().map(|(_, s)| u32::from(s.ring)).sum::<u32>();
        let before = total(h.state());

        // A shot position (mm) on the screen, the card at 100 %.
        let at = |app: &SensorApp, x: f32, y: f32| {
            let r = app.card_rect;
            r.min + app.card.to_card(x, y) * (r.width() / app.card.size_mm().x)
        };
        let mm_per_point = {
            let app = h.state();
            app.card.size_mm().x / app.card_rect.width()
        };
        let click = |h: &mut egui_kittest::Harness<SensorApp>, p: egui::Pos2| {
            h.hover_at(p);
            h.run_steps(1);
            h.drag_at(p);
            h.run_steps(1);
            h.drop_at(p);
        };

        // The first hole: its row opens the window; placed by a click, dragged, nudged.
        h.get_by_label("A hole after shot 2").click();
        step(&mut h, "1-window");
        h.get_by_label("Place it on the card…").click();
        step(&mut h, "2-place");
        let p = at(h.state(), 10.0, -5.0);
        click(&mut h, p);
        step(&mut h, "3-clicked");
        let pos = h.state().resolving.as_ref().unwrap().pos.unwrap();
        assert!((pos.0 - 10.0).abs() < mm_per_point && (pos.1 + 5.0).abs() < mm_per_point, "{pos:?}");
        // Dragged from the placed shot: it moves 2 mm right and 2 mm up (the card stays).
        let (from, to) = (at(h.state(), pos.0, pos.1), at(h.state(), pos.0 + 2.0, pos.1 + 2.0));
        h.hover_at(from);
        h.run_steps(1);
        h.drag_at(from);
        h.run_steps(1);
        for k in 1..=10 {
            h.hover_at(from + (to - from) * (k as f32 / 10.0));
            h.run_steps(1);
        }
        h.drop_at(to);
        step(&mut h, "4-dragged");
        let moved = h.state().resolving.as_ref().unwrap().pos.unwrap();
        assert!((moved.0 - pos.0 - 2.0).abs() < 2.0 * mm_per_point, "{moved:?}");
        assert!((moved.1 - pos.1 - 2.0).abs() < 2.0 * mm_per_point, "{moved:?}");
        assert_eq!(h.state().view.zoom, 1.0);
        h.key_press(egui::Key::ArrowRight);
        h.key_press(egui::Key::ArrowRight);
        h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowDown);
        step(&mut h, "5-nudged");
        let nudged = h.state().resolving.as_ref().unwrap().pos.unwrap();
        assert!((nudged.0 - moved.0 - 0.1).abs() < 1e-3 && (nudged.1 - moved.1 + 0.5).abs() < 1e-3, "{nudged:?}");
        h.get_by_label("Confirm").click();
        step(&mut h, "6-placed");
        let app = h.state();
        assert!(app.resolving.is_none());
        assert_eq!(seqs(app), [1, 2, 3, 4, 5, 7, 9]);
        let (placed, score) = app.series[2];
        assert_eq!((placed.x, placed.y), ((nudged.0 * 100.0).round() as i32, (nudged.1 * 100.0).round() as i32));
        assert_eq!(score, app.card.score(placed.x_mm(), placed.y_mm()));
        assert_eq!(app.manual.get(&(placed.boot, 3)), Some(&How::Placed));
        let placed_ring = u32::from(score.ring);

        // The missed shot: now after shot 5 (the hole is shot 3). A miss.
        h.get_by_label("A missed shot after shot 5").click();
        h.run_steps(2);
        h.get_by_label("A miss (0)").click();
        step(&mut h, "7-miss");
        let app = h.state();
        assert_eq!(seqs(app), [1, 2, 3, 4, 5, 6, 7, 9]);
        assert_eq!(app.series[5].1, resolve::MISS);
        assert_eq!(total(app), before + placed_ring);

        // The other hole (after shot 7): the same hole as shot 4 (seq 4, at 1.2, 0.8 mm),
        // picked on the card.
        h.get_by_label("A hole after shot 7").click();
        h.run_steps(2);
        h.get_by_label("The same hole as another shot…").click();
        h.run_steps(2);
        let p = at(h.state(), 1.2, 0.8);
        click(&mut h, p);
        step(&mut h, "8-same");
        assert_eq!(h.state().resolving.as_ref().unwrap().same, Some((0xDE30, 4)));
        h.get_by_label("Confirm").click();
        h.run_steps(2);
        let app = h.state();
        assert_eq!(seqs(app), [1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert!(app.unscored.is_empty());
        assert_eq!((app.series[7].0.x, app.series[7].0.y), (120, 80));
        assert_eq!(app.series[7].1, app.series[3].1);
        assert_eq!(app.manual.get(&(0xDE30, 8)), Some(&How::Same((0xDE30, 4))));

        // Shot 3 (placed by hand) selected: the window for it; back to not scored.
        h.get_by_label("Shot 3").click();
        step(&mut h, "9-selected");
        assert!(h.state().resolving.as_ref().is_some_and(|r| r.from_selection));
        h.get_by_label("Back to not scored").click();
        step(&mut h, "10-undone");
        let app = h.state();
        assert_eq!(seqs(app), [1, 2, 4, 5, 6, 7, 8, 9]);
        assert_eq!(app.unscored.iter().map(|(a, s)| (*a, s.seq)).collect::<Vec<_>>(), [(2, 3)]);
        assert!(!app.manual.contains_key(&(0xDE30, 3)));

        // The log: one line per decision, in order.
        let log = std::fs::read_to_string(logs.join(format!("{}.log", eventlog::utc_date()))).unwrap();
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("manual ")).collect();
        assert_eq!(lines.len(), 4, "{log}");
        assert!(lines[0].contains("seq=3 kind=hole as=place") && lines[0].contains(" shot=3 "));
        assert!(lines[1].contains("seq=6 kind=missed as=miss shot=6 score=0.0"));
        assert!(lines[2].contains("seq=8 kind=hole as=same same=0000DE30:4 x=1.20 y=0.80 shot=8"));
        assert_eq!(lines[3], "manual boot=0000DE30 seq=3 kind=hole as=undo");
        let _ = std::fs::remove_dir_all(&logs);
    }

    const CARD: Vec2 = egui::vec2(170.0, 170.0);
    const BASE: f32 = 3.0; // points per mm at 100%

    fn view() -> CardView {
        let mut v = CardView::default();
        v.clamp(CARD);
        v
    }

    #[test]
    fn starts_showing_the_whole_card() {
        let v = view();
        assert_eq!(v.zoom, 1.0);
        assert_eq!(v.centre_mm, CARD / 2.0);
    }

    #[test]
    fn wheel_zoom_keeps_the_point_under_the_pointer() {
        let mut v = view();
        let anchor = egui::vec2(60.0, -30.0);
        let at = |v: &CardView| v.centre_mm + anchor / (BASE * v.zoom);
        let before = at(&v);
        v.zoom_by(2.5, anchor, BASE, CARD);
        assert_eq!(v.zoom, 2.5);
        assert!((at(&v) - before).length() < 1e-4);
    }

    #[test]
    fn the_card_always_covers_the_view() {
        let mut v = view();
        v.zoom_by(4.0, Vec2::ZERO, BASE, CARD);
        v.pan(egui::vec2(1e6, 1e6), BASE, CARD); // far past the top left corner
        assert_eq!(v.centre_mm, CARD / 8.0);
        v.zoom_by(0.01, Vec2::ZERO, BASE, CARD); // back out past 100%
        assert_eq!(v.zoom, ZOOM_MIN);
        assert_eq!(v.centre_mm, CARD / 2.0);
    }

    #[test]
    fn replay_shows_the_shots_one_by_one_then_all() {
        let mut r = Replay::default();
        let t = Instant::now();
        let d = r.delay();
        assert_eq!(r.visible(3), 3);
        r.play_pause(3, t);
        assert_eq!(r.visible(3), 0);
        assert_eq!(r.tick(3, t), Some(t + d)); // not due yet
        assert_eq!(r.visible(3), 0);
        r.tick(3, t + d);
        assert_eq!(r.visible(3), 1);
        r.tick(3, t + d * 2);
        assert_eq!(r.visible(3), 2);
        assert_eq!(r.tick(3, t + d * 3), None); // the last one ends the replay
        assert_eq!((r.shown, r.playing), (None, false));
    }

    #[test]
    fn replay_steps_and_stops() {
        let mut r = Replay::default();
        r.previous(5);
        assert_eq!(r.shown, Some(4));
        r.previous(5);
        r.next(5);
        assert_eq!(r.shown, Some(4));
        r.next(5); // past the last: back to all
        assert_eq!(r.shown, None);
        r.previous(5);
        r.play_pause(5, Instant::now()); // continues from 4, doesn't restart
        assert_eq!((r.shown, r.playing), (Some(4), true));
        r.play_pause(5, Instant::now());
        assert!(!r.playing);
        r.stop();
        assert_eq!(r.visible(5), 5);
    }

    #[test]
    fn replay_speed_stays_in_range() {
        let mut r = Replay::default();
        let now = Instant::now();
        for _ in 0..20 {
            r.change_speed(true, now);
        }
        assert_eq!(r.delay_s, REPLAY_DELAY_MIN);
        for _ in 0..20 {
            r.change_speed(false, now);
        }
        assert_eq!(r.delay_s, REPLAY_DELAY_MAX);
        // From a default between the steps, to the next step.
        r.delay_s = 1.2;
        r.change_speed(true, now);
        assert_eq!(r.delay_s, 1.0);
        r.delay_s = 1.2;
        r.change_speed(false, now);
        assert_eq!(r.delay_s, 1.5);
    }

    #[test]
    fn nice_steps() {
        assert_eq!(nice_step(3.0, 4), 1.0);
        assert_eq!(nice_step(7.0, 4), 2.0);
        assert_eq!(nice_step(13.0, 4), 5.0);
        assert_eq!(nice_step(40.0, 4), 10.0);
        assert_eq!(nice_step(10.0, 8), 2.0); // one ring in tenths: 0.2 apart
        assert_eq!(nice_step(60.0, 8), 10.0); // six rings: one apart
    }

    #[test]
    fn app_icon_loads() {
        let icon = app_icon().expect("the embedded icon PNG decodes");
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
    }
}
