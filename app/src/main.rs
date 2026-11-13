// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent;
mod ble;
mod protocol;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Color32, CornerRadius, FontFamily, FontId, Layout, RichText, Stroke, TextStyle, Theme,
    ThemePreference, Ui,
};
use tokio::sync::mpsc::UnboundedSender;

use agent::PasskeyReply;
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

/// The PIN popup while pairing: BlueZ waits for `reply`.
struct PasskeyPrompt {
    reply: PasskeyReply,
    input: String,
    focus: bool,
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

    /// The sensor this app connected to last; reconnected automatically when seen.
    remembered: Option<String>,
    /// Off after the user disconnects, so we don't reconnect behind their back.
    auto_connect: bool,
    auto_retry_at: Instant,

    message: Option<String>,
    fatal: Option<String>,
    dark_mode: bool,
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
                } => {
                    self.passkey_prompt = None;
                    self.pin_supported = pin_supported;
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

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                centered_column(ui, |ui| {
                    ui.add_space(20.0);
                    title(ui, "ESP Sensor", "BLE test app");
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
                    .with_title("ESP Sensor · Settings")
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
                                title(ui, "Settings", "Appearance and the connected sensor");
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

    /// Main window: what the sensor reports, nothing to configure.
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
            Link::Connected { .. } => {
                self.heartbeat_card(ui);
                self.led_card(ui);
            }
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
