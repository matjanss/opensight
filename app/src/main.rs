// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent;
mod ble;
mod card;
mod protocol;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Color32, CornerRadius, FontFamily, FontId, Layout, RichText, Stroke, TextStyle, Theme,
    ThemePreference, Ui, Vec2,
};
use tokio::sync::mpsc::UnboundedSender;

use agent::PasskeyReply;
use ble::{Command, Event, SensorInfo};
use card::{Card, CardTexture, Higher, Score};

const CONTENT_MAX_WIDTH: f32 = 460.0;
/// Width of the column next to the target card; below this window width they stack.
const SIDE_WIDTH: f32 = 280.0;
const STACK_BELOW: f32 = 640.0;
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

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 640.0])
            .with_min_inner_size([360.0, 420.0])
            .with_title("OpenSights"),
        ..Default::default()
    };
    eframe::run_native(
        "opensights-app",
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
const REPLAY_DELAYS: [f32; 6] = [4.0, 2.0, 1.0, 0.5, 0.25, 0.1];
/// Shown with each delay, snail to rabbit (monochrome Noto Emoji, part of egui's fonts).
const REPLAY_ANIMALS: [&str; 6] = ["🐌", "🐌", "🐢", "🐕", "🐇", "🐇"];
/// A shot this close (mm) to a higher score gets "Manual review recommended".
const REVIEW_MARGIN_MM: f32 = 0.5;

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

/// The close-call notice at the top left of the card: a title, then one line per close call
/// (`lines`, in shot order). Returns the index of a line that was clicked.
fn review_notice(ui: &Ui, painter: &egui::Painter, card_rect: egui::Rect, lines: &[String]) -> Option<usize> {
    let visuals = ui.visuals();
    let small = TextStyle::Small.resolve(ui.style());
    let title = painter.layout_no_wrap(
        "Close Call - Manual review recommended".into(),
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
    let rect = egui::Rect::from_min_size(card_rect.min + egui::vec2(10.0, 10.0), size);
    // Clicks on the notice mustn't reach the card (and close the shot popup).
    ui.interact(rect, ui.scope_id().with("review_notice"), egui::Sense::click());
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
        let response = ui.interact(row, ui.scope_id().with(("review_line", i)), egui::Sense::click());
        if response.hovered() {
            painter.rect_filled(row, 4.0, visuals.widgets.hovered.weak_bg_fill);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.on_hover_text("Show this shot").clicked() {
            clicked = Some(i);
        }
        let h = g.size().y;
        painter.galley(egui::pos2(x, y), g, Color32::PLACEHOLDER);
        y += h + line_gap;
    }
    if let Some(g) = more {
        painter.galley(egui::pos2(x, y), g, Color32::PLACEHOLDER);
    }
    clicked
}

/// Replaying the series on the card: the shots appear again one by one.
struct Replay {
    /// How many of the series' shots are on the card; `None` = all, no replay.
    shown: Option<usize>,
    playing: bool,
    next_at: Instant,
    /// Index into `REPLAY_DELAYS`.
    speed: usize,
}

impl Default for Replay {
    fn default() -> Self {
        Self {
            shown: None,
            playing: false,
            next_at: Instant::now(),
            speed: 2, // 1 s
        }
    }
}

impl Replay {
    fn visible(&self, len: usize) -> usize {
        self.shown.map_or(len, |n| n.min(len))
    }

    fn delay(&self) -> Duration {
        Duration::from_secs_f32(REPLAY_DELAYS[self.speed])
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
        self.speed = if faster {
            (self.speed + 1).min(REPLAY_DELAYS.len() - 1)
        } else {
            self.speed.saturating_sub(1)
        };
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
const REPLAY_LABEL: f32 = 64.0;

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
    let every = format!("now a shot every {} s", REPLAY_DELAYS[replay.speed]);
    let replaying = replay.shown.is_some();
    let last = REPLAY_DELAYS.len() - 1;
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
            replay.speed > 0,
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
            replay.speed < last,
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
fn replay_speed_label(ui: &Ui, painter: &egui::Painter, bar: egui::Rect, speed: usize) {
    let rect = bar_slot(bar, 6.0 * BAR_BUTTON, REPLAY_LABEL);
    let delay = REPLAY_DELAYS[speed];
    ui.interact(rect, ui.scope_id().with("replay_speed"), egui::Sense::hover())
        .on_hover_text(format!("A shot every {delay} s"));
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        format!("{} {delay}s", REPLAY_ANIMALS[speed]),
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
    led: bool,
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
    series_sensor: Option<String>,
    /// (boot, seq) of the shot whose info popup is open on the card.
    selected_shot: Option<(u32, u32)>,
    view: CardView,
    replay: Replay,
    show_zoom_controls: bool,
    show_replay_controls: bool,

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
}

impl SensorApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let storage = cc.storage;
        let (cmd, events) = ble::spawn(cc.egui_ctx.clone());
        Self {
            cmd,
            events,
            link: Link::Idle,
            sensors: Vec::new(),
            heartbeat: None,
            heartbeat_at: None,
            led: false,
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
            series_sensor: None,
            selected_shot: None,
            view: CardView::default(),
            replay: Replay::default(),
            show_zoom_controls: storage
                .and_then(|s| s.get_string(KEY_ZOOM_CONTROLS))
                .is_none_or(|v| v == "true"),
            show_replay_controls: storage
                .and_then(|s| s.get_string(KEY_REPLAY_CONTROLS))
                .is_none_or(|v| v == "true"),
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
        }
    }

    fn send(&self, c: Command) {
        let _ = self.cmd.send(c); // fails only if the BLE thread died, which shows as `fatal`
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
                    led,
                    heartbeat,
                    pin_supported,
                    shots_supported,
                } => {
                    self.passkey_prompt = None;
                    self.pin_supported = pin_supported;
                    self.shots_supported = shots_supported;
                    if self.series_sensor.as_deref() != Some(id.as_str()) {
                        self.series.clear();
                        self.series_sensor = Some(id.clone());
                    }
                    self.pin_input.clear();
                    self.pin_confirm = false;
                    self.remembered = Some(id.clone());
                    self.link = Link::Connected { id };
                    self.name_input = name.clone();
                    self.sensor_name = name;
                    self.led = led;
                    self.heartbeat = heartbeat;
                    self.heartbeat_at = heartbeat.map(|_| Instant::now());
                    self.message = None;
                }
                Event::Disconnected { reason } => {
                    self.passkey_prompt = None;
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
                Event::Led(on) => self.led = on,
                Event::Shot(shot) => {
                    let score = self.card.score(shot.x_mm(), shot.y_mm());
                    self.series.push((shot, score));
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

        let on_target = matches!(self.link, Link::Connected { .. }) && self.fatal.is_none();
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
                egui::ViewportBuilder::default()
                    .with_title("OpenSights · Settings")
                    .with_inner_size([460.0, 640.0])
                    .with_min_inner_size([360.0, 420.0]),
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
            let card_w = (avail.x - SIDE_WIDTH - gap).min(avail.y / aspect).max(160.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.add_space(((avail.x - card_w - gap - SIDE_WIDTH) / 2.0 - gap).max(0.0));
                self.target_card(ui, card_w);
                ui.vertical(|ui| {
                    ui.set_width(SIDE_WIDTH);
                    egui::ScrollArea::vertical().show(ui, |ui| self.series_column(ui));
                });
            });
        } else {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.target_card(ui, ui.available_width());
                ui.add_space(6.0);
                self.series_column(ui);
            });
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
        if response.dragged() {
            self.view.pan(response.drag_delta(), base, card_mm);
        }

        let scale = base * self.view.zoom; // points per mm
        let origin = rect.center() - self.view.centre_mm * scale; // the card's top left
        let to_screen = |mm: Vec2| origin + mm * scale;
        let painter = ui.painter_at(rect);
        let ppp = ui.ctx().pixels_per_point();
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        if let Some(tex) = self.card_texture.get(ui.ctx(), &self.card, width * ppp) {
            painter.image(
                tex.id(),
                egui::Rect::from_min_size(origin, card_mm * scale),
                uv,
                Color32::WHITE,
            );
        }
        if self.view.zoom > 1.0 {
            // A sharp render of the visible part, over the stretched full card. Rendered
            // once the view stops moving; while it moves the last one moves along.
            let visible = egui::Rect::from_center_size(self.view.centre_mm.to_pos2(), rect.size() / scale);
            let px = [
                (rect.width() * ppp).round() as u32,
                (rect.height() * ppp).round() as u32,
            ];
            let settled = self.view == view_before && !response.dragged();
            if let Some((tex, mm)) = self
                .card_texture
                .detail(ui.ctx(), &self.card, visible, px, settled)
            {
                let at = egui::Rect::from_min_max(to_screen(mm.min.to_vec2()), to_screen(mm.max.to_vec2()));
                painter.image(tex.id(), at, uv, Color32::WHITE);
            }
            if !settled {
                ui.ctx().request_repaint();
            }
        }

        let radius = self.card.pellet_diameter_mm / 2.0 * scale;
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
        for (i, (shot, _)) in self.series[..visible].iter().enumerate() {
            let (fill, stroke) = if i + 1 == visible {
                (LAST_SHOT_COLOR, Stroke::new(1.5, Color32::WHITE))
            } else {
                (Color32::from_gray(150), Stroke::new(1.0, Color32::from_gray(30)))
            };
            painter.circle(shot_pos(shot), radius, fill, stroke);
        }

        // The shot under the pointer: the nearest within the hole (at least a few points
        // wide on a small card), the newest of equally near ones (it's drawn on top).
        let hit_r = radius.max(6.0);
        let shot_at = |p: egui::Pos2| {
            self.series[..visible]
                .iter()
                .enumerate()
                .rev()
                .map(|(i, (shot, _))| (i, shot_pos(shot).distance(p)))
                .filter(|&(_, d)| d <= hit_r)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        };
        if response.dragged() && self.view.zoom > 1.0 {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if response.hover_pos().and_then(shot_at).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        } else if response.hovered() && self.view.zoom > 1.0 {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if response.clicked() {
            let key = |i: usize| (self.series[i].0.boot, self.series[i].0.seq);
            let hit = response.interact_pointer_pos().and_then(shot_at).map(key);
            self.selected_shot = if hit == self.selected_shot { None } else { hit };
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.selected_shot = None;
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
            replay_speed_label(ui, &painter, bar, self.replay.speed);
        }

        // Every shot of the series that nearly scored higher, worth a look at the real card.
        // Stays until the series is cleared, also during a replay.
        let close_calls: Vec<(usize, Higher)> = self
            .series
            .iter()
            .enumerate()
            .filter_map(|(i, (s, _))| Some((i, self.card.higher(s.x_mm(), s.y_mm())?)))
            .filter(|(_, h)| h.gap_mm <= REVIEW_MARGIN_MM)
            .collect();
        if !close_calls.is_empty() {
            let lines: Vec<String> = close_calls
                .iter()
                .map(|&(i, h)| format!("Shot {} · {}", i + 1, review_detail(h)))
                .collect();
            if let Some(line) = review_notice(ui, &painter, rect, &lines) {
                let i = close_calls[line].0;
                let shot = &self.series[i].0;
                self.selected_shot = Some((shot.boot, shot.seq));
                if i >= visible {
                    self.replay.stop(); // not replayed yet: show all shots to show this one
                }
                ui.ctx().request_repaint();
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
        if !rect.contains(pos) || i >= visible {
            return; // out of view, or not replayed yet; it shows again when it's back
        }
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
                    let higher = self.card.higher(shot.x_mm(), shot.y_mm());
                    if let Some(h) = higher.filter(|h| h.gap_mm <= REVIEW_MARGIN_MM) {
                        let text = format!("⚠ Close call: {}, manual review recommended", review_detail(h));
                        ui.label(RichText::new(text).small());
                    }
                });
            });
    }

    /// Last shot and the series list with the total.
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
            ui.horizontal(|ui| {
                ui.label(RichText::new(score.text(decimal)).size(44.0).strong());
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let mut line = format!("Shot {} · ring {}", self.series.len(), score.ring);
                    if score.inner_ten && decimal {
                        line.push_str(" · inner ten");
                    }
                    ui.label(line);
                    let off = shot.x_mm().hypot(shot.y_mm());
                    ui.label(RichText::new(format!("{off:.1} mm from the centre")).weak());
                });
            });
        });

        let mut clear = false;
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
                ui.label(RichText::new(detail).weak());
            });
            if self.series.is_empty() {
                return;
            }
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("series")
                .max_height(240.0)
                .show(ui, |ui| {
                    for (i, (_, score)) in self.series.iter().enumerate().rev() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{}", i + 1)).weak());
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(score.text(decimal)).strong());
                                if score.inner_ten && decimal {
                                    ui.label(RichText::new("X").small().weak());
                                }
                            });
                        });
                    }
                });
            ui.separator();
            clear = ui
                .add(egui::Button::new("Clear series").frame(false))
                .on_hover_text("Start a new series. Only clears this app's list.")
                .clicked();
        });
        if clear {
            self.series.clear();
        }
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
            ui.horizontal(|ui| {
                ui.label("Replay controls");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    toggle_switch(ui, &mut self.show_replay_controls);
                });
            });
            ui.label(
                RichText::new("The wheel zooms and dragging moves the card either way.")
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
                self.name_card(ui);
                self.pin_card(ui);
                self.heartbeat_card(ui);
                self.led_card(ui);
                card(ui, "Connection", |ui| {
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

    fn heartbeat_card(&mut self, ui: &mut Ui) {
        card(ui, "Heartbeat", |ui| match self.heartbeat {
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

    fn led_card(&mut self, ui: &mut Ui) {
        card(ui, "LED", |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(
                        RichText::new(if self.led { "On" } else { "Off" })
                            .size(28.0)
                            .strong(),
                    );
                    ui.label(RichText::new("Onboard LED, GPIO2").weak());
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = if self.led { "Turn off" } else { "Turn on" };
                    // No optimistic update: the label flips when the sensor confirms.
                    if ui.button(label).clicked() {
                        self.send(Command::SetLed(!self.led));
                    }
                });
            });
        });
    }

    fn name_card(&mut self, ui: &mut Ui) {
        card(ui, "Name", |ui| {
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
        card(ui, "Pairing PIN", |ui| {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        for _ in 0..10 {
            r.change_speed(true, now);
        }
        assert_eq!(r.speed, REPLAY_DELAYS.len() - 1);
        for _ in 0..10 {
            r.change_speed(false, now);
        }
        assert_eq!(r.speed, 0);
    }
}
