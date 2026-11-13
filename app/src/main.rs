// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ble;
mod protocol;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Color32, CornerRadius, FontFamily, FontId, Layout, RichText, Stroke, TextStyle, Theme,
    ThemePreference, Ui,
};
use tokio::sync::mpsc::UnboundedSender;

use ble::{Command, Event, SensorInfo};

const CONTENT_MAX_WIDTH: f32 = 460.0;
/// After a failed automatic reconnect, wait this long before trying again.
const AUTO_RETRY: Duration = Duration::from_secs(10);
const KEY_REMEMBERED: &str = "remembered_sensor";
const KEY_DARK: &str = "dark_mode";

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([480.0, 700.0])
            .with_min_inner_size([360.0, 420.0])
            .with_title("ESP Sensor"),
        ..Default::default()
    };
    eframe::run_native(
        "esp-sensor-app",
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

    /// The sensor this app connected to last; reconnected automatically when seen.
    remembered: Option<String>,
    /// Off after the user disconnects, so we don't reconnect behind their back.
    auto_connect: bool,
    auto_retry_at: Instant,

    message: Option<String>,
    fatal: Option<String>,
    dark_mode: bool,
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
                } => {
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
                Event::Name(name) => {
                    self.name_input = name.clone();
                    self.sensor_name = name;
                }
                Event::Error(e) => self.message = Some(e),
            }
        }
    }

    /// Reconnects to the remembered sensor when it shows up.
    fn auto_connect(&mut self) {
        if !matches!(self.link, Link::Idle) || !self.auto_connect || self.fatal.is_some() {
            return;
        }
        let Some(id) = self.remembered.clone() else { return };
        if Instant::now() >= self.auto_retry_at && self.sensors.iter().any(|s| s.id == id) {
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

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                centered_column(ui, |ui| {
                    ui.add_space(20.0);
                    self.header(ui);
                    ui.add_space(14.0);
                    self.body(ui);
                    ui.add_space(16.0);
                });
            });
        });

        if self.dark_mode != was_dark {
            ui.ctx().set_theme(theme_pref(self.dark_mode));
        }
        // Keeps "updated 3 s ago" ticking; BLE events repaint on their own.
        if matches!(self.link, Link::Connected { .. }) {
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(KEY_REMEMBERED, self.remembered.clone().unwrap_or_default());
        storage.set_string(KEY_DARK, self.dark_mode.to_string());
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
                ui.label(
                    RichText::new(format!("Protocol v{}", protocol::PROTOCOL_VERSION))
                        .small()
                        .weak(),
                );
            });
        });
    }

    fn header(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new("ESP Sensor").heading().strong());
                ui.label(RichText::new("BLE test app").weak());
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                segmented(ui, &mut self.dark_mode, [(false, "Light"), (true, "Dark")]);
            });
        });
    }

    fn body(&mut self, ui: &mut Ui) {
        if let Some(e) = &self.fatal {
            card(ui, "Bluetooth unavailable", |ui| {
                ui.label(e);
                ui.label(RichText::new("Check that Bluetooth is switched on, then restart the app.").weak());
            });
            return;
        }

        if let Some(msg) = self.message.clone()
            && notice(ui, &msg)
        {
            self.message = None;
        }

        match &self.link {
            Link::Connected { id } => {
                let id = id.clone();
                self.heartbeat_card(ui);
                self.led_card(ui);
                self.name_card(ui);
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
            }
            Link::Connecting { name } => {
                let name = name.clone();
                card(ui, "Connecting", |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("Connecting to {name}…"));
                    });
                    ui.label(
                        RichText::new(format!(
                            "If your computer asks for a PIN or passkey, enter {}.",
                            protocol::PASSKEY
                        ))
                        .weak(),
                    );
                });
            }
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
                let remembered = self.remembered.as_deref() == Some(s.id.as_str());
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
                } else if remembered {
                    detail.push("paired".to_owned());
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
                        let label = if pairing && !remembered { "Pair" } else { "Connect" };
                        if ui.add_enabled(supported, egui::Button::new(label)).clicked() {
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

fn pairing_help_card(ui: &mut Ui) {
    card(ui, "Pairing a new sensor", |ui| {
        let steps = [
            "Hold BOOT on the sensor for 2 seconds, until the blue LED blinks fast.".to_owned(),
            "Press Pair next to the sensor above (within 60 seconds).".to_owned(),
            format!("When your computer asks for a PIN, enter {}.", protocol::PASSKEY),
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

fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: [(T, &str); 2]) {
    let v = ui.visuals();
    egui::Frame::new()
        .fill(v.extreme_bg_color)
        .stroke(v.window_stroke)
        .corner_radius(10)
        .inner_margin(3)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.spacing_mut().button_padding = egui::vec2(12.0, 4.0);
            ui.spacing_mut().interact_size.y = 26.0;
            // The header lays this out right to left; add in reverse so it reads left to right.
            for (opt, label) in options.into_iter().rev() {
                ui.selectable_value(value, opt, label);
            }
        });
}
