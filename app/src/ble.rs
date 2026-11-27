//! The BLE side of the app. A background thread owns the adapter and at most one
//! connection, and talks to the UI through two channels: [`Command`]s in, [`Event`]s out.
//! btleplug is async (tokio), while egui redraws on its own thread, so they are kept apart.

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use btleplug::api::{
    Central, CentralEvent, Characteristic, Manager as _, Peripheral as _, ScanFilter, ValueNotification,
    WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral, PeripheralId};
use eframe::egui;
use futures::{Stream, StreamExt};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::agent::{AgentEvent, PairingAgent, PasskeyReply};
use crate::eventlog;
use crate::protocol::{self as proto, AdvInfo};

/// After a firmware update: how many seconds to wait for the computer to read the new GATT
/// layout (see `open`).
const V3_DISCOVERY_TRIES: u32 = 8;

/// A sensor that isn't heard from for this long drops off the "nearby" list.
const SENSOR_STALE_AFTER: Duration = Duration::from_secs(15);
/// Connecting includes the pairing prompt, so leave the user time to type the PIN.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(90);
/// How often to look for the sensor again after deleting an old pairing.
const REDISCOVER_POLL: Duration = Duration::from_millis(250);

pub enum Command {
    Connect(String),
    Disconnect,
    SetName(String),
    /// Changes the sensor's pairing PIN (0–999 999). Unpairs all other computers.
    SetPin(u32),
    /// Makes the sensor forget the shots the app has received, so they aren't sent again.
    ClearShots,
    /// A Control command (calibration mode, template point: `protocol::encode_*`).
    Control(Vec<u8>),
    /// Writes the calibration record (the wizard's fit); the sensor confirms it in Status.
    WriteCalibration(Box<proto::Calibration>),
    /// Writes the settings (whole; the sensor checks them and drops them if out of range,
    /// with no error here). Taken, a new cfg ID in Status brings them back as `Config`.
    WriteSettings(Box<proto::Settings>),
}

#[derive(Clone, Debug)]
pub struct SensorInfo {
    pub id: String,
    pub name: String,
    pub adv: Option<AdvInfo>,
    pub rssi: Option<i16>,
    /// Whether this computer is paired with it. None where the OS doesn't tell (not Linux).
    pub paired: Option<bool>,
}

pub enum Event {
    /// The adapter could not be opened (no Bluetooth, permission denied, ...).
    Fatal(String),
    Sensors(Vec<SensorInfo>),
    Connecting {
        name: String,
    },
    Connected {
        id: String,
        name: String,
        heartbeat: Option<u32>,
        /// False for firmware without the PIN characteristic.
        pin_supported: bool,
        /// False for firmware without the shot characteristic.
        shots_supported: bool,
    },
    /// `reason` is None when the user asked for the disconnect.
    Disconnected {
        reason: Option<String>,
    },
    Heartbeat(u32),
    Name(String),
    /// A shot the app hasn't had before (duplicates are dropped here).
    Shot(proto::Shot),
    /// An event the app hasn't had before (protocol v3: every event, all verdicts).
    Captured(proto::Event),
    /// The sensor's state, as notified.
    Status(proto::Status),
    /// The sensor's calibration and settings, read together after a change. Their `cfg`
    /// line's ID matches the Status's cfg ID.
    Config(proto::Calibration, proto::Settings),
    /// The sensor accepted a new PIN.
    PinChanged,
    /// Pairing needs the sensor's PIN: ask the user, answer through the reply (Linux).
    PasskeyRequest(PasskeyReply),
    /// The pairing gave up before the user answered: close the PIN prompt.
    PasskeyCancelled,
    /// A command failed, but the connection is still up.
    Error(String),
}

/// Starts the BLE thread. Every event also asks egui to repaint.
pub fn spawn(ctx: egui::Context) -> (UnboundedSender<Command>, mpsc::Receiver<Event>) {
    let (cmd_tx, cmd_rx) = unbounded_channel();
    let (ev_tx, ev_rx) = mpsc::channel();
    let events = EventSink { tx: ev_tx, ctx };
    std::thread::Builder::new()
        .name("ble".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build();
            match rt {
                Ok(rt) => rt.block_on(async {
                    if let Err(e) = run(cmd_rx, &events).await {
                        events.send(Event::Fatal(e));
                    }
                }),
                Err(e) => events.send(Event::Fatal(format!("Could not start the BLE runtime: {e}"))),
            }
        })
        .expect("failed to start the BLE thread");
    (cmd_tx, ev_rx)
}

#[derive(Clone)]
struct EventSink {
    tx: mpsc::Sender<Event>,
    ctx: egui::Context,
}

impl EventSink {
    fn send(&self, event: Event) {
        let _ = self.tx.send(event); // the UI is gone: nothing left to tell
        self.ctx.request_repaint();
    }
}

type Notifications = Pin<Box<dyn Stream<Item = ValueNotification> + Send>>;

/// An open connection plus the values read while opening it.
struct Opened {
    conn: Connection,
    stream: Notifications,
    name: String,
    heartbeat: Option<u32>,
    status: Option<proto::Status>,
}

struct Connection {
    peripheral: Peripheral,
    name: Characteristic,
    /// None on firmware from before the PIN characteristic.
    pin: Option<Characteristic>,
    /// None on firmware from before the shot characteristic.
    shot: Option<Characteristic>,
    /// Protocol v3 (required: the version check makes sure of it).
    control: Characteristic,
    calibration: Characteristic,
    settings: Characteristic,
    /// The cfg ID of the calibration and settings last read (eventlog::cfg_line's).
    config_id: Option<u32>,
}

impl Connection {
    /// Reads the calibration and settings. Returns them with their cfg ID.
    async fn read_config(&self) -> Result<(proto::Calibration, proto::Settings, u32), String> {
        let cal = self
            .peripheral
            .read(&self.calibration)
            .await
            .map_err(|e| e.to_string())?;
        let set = self
            .peripheral
            .read(&self.settings)
            .await
            .map_err(|e| e.to_string())?;
        let cal = proto::decode_calibration(&cal).ok_or("The sensor's calibration record can't be read.")?;
        let set = proto::decode_settings(&set).ok_or("The sensor's settings can't be read.")?;
        let (id, _) = eventlog::cfg_line(&cal, &set);
        Ok((cal, set, id))
    }
}

async fn run(mut commands: UnboundedReceiver<Command>, events: &EventSink) -> Result<(), String> {
    let manager = Manager::new()
        .await
        .map_err(|e| format!("Bluetooth is not available: {e}"))?;
    let adapter = manager
        .adapters()
        .await
        .map_err(|e| format!("Could not list Bluetooth adapters: {e}"))?
        .into_iter()
        .next()
        .ok_or("No Bluetooth adapter found.")?;
    let mut central_events = adapter.events().await.map_err(|e| e.to_string())?;
    adapter
        .start_scan(ScanFilter {
            services: vec![proto::SERVICE_UUID],
        })
        .await
        .map_err(|e| format!("Could not start scanning: {e}"))?;

    // Without the agent, pairing falls back to the OS prompt (if the desktop has one).
    let sink = events.clone();
    let notify = Box::new(move |ev| {
        sink.send(match ev {
            AgentEvent::PasskeyRequest(reply) => Event::PasskeyRequest(reply),
            AgentEvent::Cancelled => Event::PasskeyCancelled,
        })
    });
    let agent = match PairingAgent::register(notify).await {
        Ok(a) => Some(a),
        Err(e) => {
            events.send(Event::Error(format!(
                "{e}. Pair the sensor in your system's Bluetooth settings instead."
            )));
            None
        }
    };

    let mut sensors: BTreeMap<String, (PeripheralId, SensorInfo, Instant)> = BTreeMap::new();
    let mut conn: Option<Connection> = None;
    let mut notifications: Option<Notifications> = None;
    let mut prune = tokio::time::interval(Duration::from_secs(2));
    // (boot, seq) of the newest shot received, kept across reconnects: after a link drop
    // the sensor is asked for everything after it. The same for events.
    let mut last_shot: Option<(u32, u32)> = None;
    let mut last_event: Option<(u32, u32)> = None;

    loop {
        tokio::select! {
            Some(ev) = central_events.next() => match ev {
                CentralEvent::DeviceDisconnected(id) => {
                    if conn.as_ref().is_some_and(|c| c.peripheral.id() == id) {
                        conn = None;
                        notifications = None;
                        events.send(Event::Disconnected { reason: Some("The sensor disconnected.".into()) });
                    }
                }
                CentralEvent::DeviceDiscovered(id)
                | CentralEvent::DeviceUpdated(id)
                | CentralEvent::ManufacturerDataAdvertisement { id, .. }
                | CentralEvent::ServicesAdvertisement { id, .. }
                | CentralEvent::RssiUpdate { id, .. }
                    if refresh_sensor(&adapter, &id, agent.as_ref(), &mut sensors).await => {
                        events.send(Event::Sensors(sensor_list(&sensors)));
                    }
                _ => {}
            },

            _ = prune.tick() => {
                let before = sensors.len();
                sensors.retain(|_, (_, _, seen)| seen.elapsed() < SENSOR_STALE_AFTER);
                let mut changed = sensors.len() != before;
                // Pairing changes without an advertisement change (the app pairs, or the
                // user removes the sensor in the system settings), so poll it here.
                if let Some(agent) = &agent {
                    for (pid, info, _) in sensors.values_mut() {
                        let paired = agent.is_paired(&bluez_path(pid)).await;
                        if paired.is_some() && paired != info.paired {
                            info.paired = paired;
                            changed = true;
                        }
                    }
                }
                if changed {
                    events.send(Event::Sensors(sensor_list(&sensors)));
                }
            }

            cmd = commands.recv() => match cmd {
                None => break, // the UI closed
                Some(Command::Connect(id)) => {
                    if conn.is_some() { continue; }
                    let Some((pid, info, _)) = sensors.get(&id) else {
                        events.send(Event::Disconnected { reason: Some("That sensor is no longer in range.".into()) });
                        continue;
                    };
                    events.send(Event::Connecting { name: info.name.clone() });
                    let pid = pid.clone();
                    let pairing_mode = info.adv.is_some_and(|a| a.pairing);
                    let attempt = connect(&adapter, &pid, agent.as_ref(), pairing_mode, last_shot, last_event);
                    let result = tokio::time::timeout(CONNECT_TIMEOUT, attempt).await;
                    if let Ok(Err(e)) = &result { eprintln!("ble: connect failed: {e}"); }
                    match result {
                        Ok(Ok(Opened { conn: mut c, stream, name, heartbeat, status })) => {
                            let pin_supported = c.pin.is_some();
                            let shots_supported = c.shot.is_some();
                            events.send(Event::Connected { id, name, heartbeat, pin_supported, shots_supported });
                            match c.read_config().await {
                                Ok((cal, set, cfg)) => {
                                    c.config_id = Some(cfg);
                                    events.send(Event::Config(cal, set));
                                }
                                Err(e) => events.send(Event::Error(e)),
                            }
                            if let Some(st) = status { events.send(Event::Status(st)); }
                            conn = Some(c);
                            notifications = Some(stream);
                        }
                        Ok(Err(reason)) => events.send(Event::Disconnected { reason: Some(reason) }),
                        Err(_) => {
                            if let Ok(p) = adapter.peripheral(&pid).await {
                                let _ = p.disconnect().await;
                            }
                            events.send(Event::Disconnected { reason: Some("Timed out while connecting.".into()) });
                        }
                    }
                }
                Some(Command::Disconnect) => {
                    if let Some(c) = conn.take() {
                        notifications = None;
                        let _ = c.peripheral.disconnect().await;
                        events.send(Event::Disconnected { reason: None });
                    }
                }
                Some(Command::SetName(name)) => {
                    if let Some(c) = &conn
                        && let Err(e) = c.peripheral.write(&c.name, name.as_bytes(), WriteType::WithResponse).await {
                            events.send(Event::Error(format!("Could not rename the sensor: {e}")));
                        }
                }
                Some(Command::ClearShots) => {
                    if let Some(c) = &conn && let Some(chr) = &c.shot && let Some((boot, seq)) = last_shot {
                        let req = proto::encode_shot_clear(boot, seq);
                        if let Err(e) = c.peripheral.write(chr, &req, WriteType::WithResponse).await {
                            events.send(Event::Error(format!("Could not clear the sensor's shots: {e}")));
                        }
                    }
                }
                Some(Command::Control(bytes)) => {
                    if let Some(c) = &conn
                        && let Err(e) = c.peripheral.write(&c.control, &bytes, WriteType::WithResponse).await {
                            events.send(Event::Error(format!("Could not send a command to the sensor: {e}")));
                        }
                }
                Some(Command::WriteCalibration(cal)) => {
                    if let Some(c) = &conn
                        && let Err(e) = c.peripheral.write(&c.calibration, &proto::encode_calibration(&cal), WriteType::WithResponse).await {
                            events.send(Event::Error(format!("Could not write the calibration: {e}")));
                        }
                }
                Some(Command::WriteSettings(set)) => {
                    if let Some(c) = &conn
                        && let Err(e) = c.peripheral.write(&c.settings, &proto::encode_settings(&set), WriteType::WithResponse).await {
                            events.send(Event::Error(format!("Could not write the sensor's settings: {e}")));
                        }
                }
                Some(Command::SetPin(pin)) => {
                    if let Some(c) = &conn && let Some(chr) = &c.pin {
                        match c.peripheral.write(chr, &proto::encode_pin(pin), WriteType::WithResponse).await {
                            Ok(()) => events.send(Event::PinChanged),
                            Err(e) => events.send(Event::Error(format!("Could not change the PIN: {e}"))),
                        }
                    }
                }
            },

            n = next_notification(&mut notifications) => match n {
                Some(n) => {
                    let ev = match n.uuid {
                        u if u == proto::HEARTBEAT_UUID => proto::decode_heartbeat(&n.value).map(Event::Heartbeat),
                        u if u == proto::NAME_UUID => proto::decode_name(&n.value).map(Event::Name),
                        u if u == proto::SHOT_UUID => proto::decode_shot(&n.value)
                            .and_then(|shot| new_shot(&mut last_shot, shot, events))
                            .map(Event::Shot),
                        u if u == proto::EVENT_UUID => proto::decode_event(&n.value)
                            .and_then(|e| new_event(&mut last_event, e))
                            .map(Event::Captured),
                        u if u == proto::STATUS_UUID => proto::decode_status(&n.value).map(Event::Status),
                        _ => None,
                    };
                    // A new cfg ID (a change on the sensor): read its calibration and settings,
                    // so the event log gets the matching `cfg` line.
                    if let Some(Event::Status(st)) = &ev
                        && let Some(c) = &mut conn
                        && c.config_id != Some(st.cfg)
                    {
                        match c.read_config().await {
                            Ok((cal, set, cfg)) => {
                                c.config_id = Some(cfg);
                                events.send(Event::Config(cal, set));
                            }
                            Err(e) => events.send(Event::Error(e)),
                        }
                    }
                    if let Some(ev) = ev { events.send(ev); }
                }
                None => {
                    // The notification stream ends when the link goes down on some platforms.
                    if conn.take().is_some() {
                        events.send(Event::Disconnected { reason: Some("Connection lost.".into()) });
                    }
                    notifications = None;
                }
            },
        }
    }
    Ok(())
}

/// Drops shots the app already has (the sensor resends after a link drop). Reports a gap,
/// which means the sensor's buffer overflowed while the app was away.
fn new_shot(last: &mut Option<(u32, u32)>, shot: proto::Shot, events: &EventSink) -> Option<proto::Shot> {
    if let Some((boot, seq)) = *last
        && boot == shot.boot
    {
        if shot.seq <= seq {
            return None;
        }
        if shot.seq > seq + 1 {
            let n = shot.seq - seq - 1;
            events.send(Event::Error(format!(
                "{n} shot(s) were lost: the target couldn't keep them all while it wasn't connected."
            )));
        }
    }
    *last = Some((shot.boot, shot.seq));
    Some(shot)
}

/// Drops events the app already has (resent after a link drop). Unlike shots, a gap is no
/// error: the sensor keeps only the last 32 events, and they are diagnostics.
fn new_event(last: &mut Option<(u32, u32)>, e: proto::Event) -> Option<proto::Event> {
    if let Some((boot, seq)) = *last
        && boot == e.boot
        && e.seq <= seq
    {
        return None;
    }
    *last = Some((e.boot, e.seq));
    Some(e)
}

/// Waits for the next notification, or forever while not connected.
async fn next_notification(stream: &mut Option<Notifications>) -> Option<ValueNotification> {
    match stream {
        Some(s) => s.next().await,
        None => std::future::pending().await,
    }
}

/// BlueZ's D-Bus object path for a peripheral. Meaningless on other OSes, where the agent
/// ignores it.
fn bluez_path(pid: &PeripheralId) -> String {
    format!("/org/bluez/{pid}")
}

/// Re-reads a peripheral's advertised data. Returns true if the sensor list changed.
/// The pairing state is looked up once, for new sensors; the prune tick keeps it current.
async fn refresh_sensor(
    adapter: &Adapter,
    id: &PeripheralId,
    agent: Option<&PairingAgent>,
    sensors: &mut BTreeMap<String, (PeripheralId, SensorInfo, Instant)>,
) -> bool {
    let Ok(p) = adapter.peripheral(id).await else {
        return false;
    };
    let Ok(Some(props)) = p.properties().await else {
        return false;
    };
    let adv = proto::decode_adv(&props.manufacturer_data);
    // The scan filter should only let our sensors through, but not every OS applies it.
    if !props.services.contains(&proto::SERVICE_UUID) && adv.is_none() {
        return false;
    }
    let key = id.to_string();
    let paired = match (sensors.get(&key), agent) {
        (Some((_, old, _)), _) => old.paired,
        (None, Some(agent)) => agent.is_paired(&bluez_path(id)).await,
        (None, None) => None,
    };
    let info = SensorInfo {
        id: key.clone(),
        name: props
            .local_name
            .or(props.advertisement_name)
            .unwrap_or_else(|| "Unnamed sensor".into()),
        adv,
        rssi: props.rssi,
        paired,
    };
    let changed = sensors
        .get(&key)
        .is_none_or(|(_, old, _)| old.name != info.name || old.adv != info.adv || old.rssi != info.rssi);
    sensors.insert(key, (id.clone(), info, Instant::now()));
    changed
}

fn sensor_list(sensors: &BTreeMap<String, (PeripheralId, SensorInfo, Instant)>) -> Vec<SensorInfo> {
    sensors.values().map(|(_, info, _)| info.clone()).collect()
}

/// Connects to a sensor, pairing first if needed. Disconnects again if anything fails.
///
/// A sensor in pairing mode is always paired afresh: if this computer still has keys from
/// an earlier pairing that the sensor has since lost (reflashed, erased), the old keys
/// would be offered, rejected, and nothing would ask for the PIN.
async fn connect(
    adapter: &Adapter,
    pid: &PeripheralId,
    agent: Option<&PairingAgent>,
    pairing_mode: bool,
    last_shot: Option<(u32, u32)>,
    last_event: Option<(u32, u32)>,
) -> Result<Opened, String> {
    let path = bluez_path(pid);
    if pairing_mode
        && let Some(agent) = agent
        && agent.is_paired(&path).await == Some(true)
    {
        agent.forget(&path).await?;
        // Removing the keys also removes BlueZ's device entry; wait for the next advertisement.
        while adapter.peripheral(pid).await.is_err() {
            tokio::time::sleep(REDISCOVER_POLL).await;
        }
    }
    let p = adapter.peripheral(pid).await.map_err(|e| e.to_string())?;
    let result = open(&p, agent, &path, last_shot, last_event).await;
    if result.is_err() {
        let _ = p.disconnect().await;
    }
    result
}

/// Connects, pairs if needed, checks the protocol version, reads the current values and
/// subscribes. Asks for the shots after `last_shot` (all buffered ones if None). Without our agent (other OSes), the first read needs an encrypted link, so
/// that is where the OS pairing prompt appears if the two aren't bonded yet.
async fn open(
    p: &Peripheral,
    agent: Option<&PairingAgent>,
    path: &str,
    last_shot: Option<(u32, u32)>,
    last_event: Option<(u32, u32)>,
) -> Result<Opened, String> {
    let refused = "The sensor refused the connection. If it isn't paired with this computer yet, \
                   hold BOOT for 2 s until the LED blinks, then press Pair.";

    // Pair before connecting: BlueZ then connects itself and pairs with our agent's
    // capability (passkey entry). On a link that is already up it would use the system-wide
    // capability instead, which is "no input" without a default agent: that pairs without
    // the passkey (no MITM), and the sensor's characteristics refuse such a link.
    if let Some(agent) = agent {
        agent
            .pair(path)
            .await
            .map_err(|e| format!("{e} While the sensor's LED still blinks, press Pair to try again."))?;
    }
    p.connect().await.map_err(|e| format!("Could not connect: {e}"))?;
    p.discover_services().await.map_err(|_| refused.to_string())?;

    let chars = p.characteristics();
    let find = |uuid| {
        chars.iter().find(|c| c.uuid == uuid).cloned().ok_or_else(|| {
            "This device doesn't look like an OpenSights sensor (characteristic missing).".to_string()
        })
    };
    let (version, heartbeat, name) = (
        find(proto::VERSION_UUID)?,
        find(proto::HEARTBEAT_UUID)?,
        find(proto::NAME_UUID)?,
    );
    let pin = chars.iter().find(|c| c.uuid == proto::PIN_UUID).cloned();
    let shot = chars.iter().find(|c| c.uuid == proto::SHOT_UUID).cloned();

    let read_err = |e: btleplug::Error| {
        format!(
            "Could not read from the sensor: {e}. If pairing was cancelled or the PIN was wrong, try Pair again."
        )
    };
    let v = p.read(&version).await.map_err(read_err)?;
    if v.as_slice() != [proto::PROTOCOL_VERSION] {
        return Err(format!(
            "The sensor speaks protocol version {:?}, this app speaks {}. Update the firmware or the app.",
            v,
            proto::PROTOCOL_VERSION
        ));
    }
    // Version 3's characteristics. Right after a firmware update the computer may still have
    // the old layout cached: the sensor tells it to re-read (a "services changed" indication,
    // about a second after the link is secured, which the version read above did), and the
    // OS discovers again. Wait for that a few seconds.
    let mut chars = chars;
    for _ in 0..V3_DISCOVERY_TRIES {
        if chars.iter().any(|c| c.uuid == proto::EVENT_UUID) {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        p.discover_services().await.map_err(|_| refused.to_string())?;
        chars = p.characteristics();
    }
    let v3 = |uuid| chars.iter().find(|c| c.uuid == uuid).cloned();
    let missing = || {
        "This computer still has the sensor's old Bluetooth layout (from before a firmware update). \
         Hold BOOT on the sensor for 2 s until the LED blinks, then press Pair: pairing again makes \
         the computer read it afresh."
            .to_string()
    };
    let event = v3(proto::EVENT_UUID).ok_or_else(missing)?;
    let status = v3(proto::STATUS_UUID).ok_or_else(missing)?;
    let control = v3(proto::CONTROL_UUID).ok_or_else(missing)?;
    let calibration = v3(proto::CALIBRATION_UUID).ok_or_else(missing)?;
    let settings = v3(proto::SETTINGS_UUID).ok_or_else(missing)?;
    let name_value = proto::decode_name(&p.read(&name).await.map_err(read_err)?).unwrap_or_default();
    let status_value = proto::decode_status(&p.read(&status).await.map_err(read_err)?);
    let heartbeat_value = proto::decode_heartbeat(&p.read(&heartbeat).await.map_err(read_err)?);

    let stream = p.notifications().await.map_err(|e| e.to_string())?;
    for c in [&heartbeat, &name, &status] {
        p.subscribe(c)
            .await
            .map_err(|e| format!("Could not subscribe: {e}"))?;
    }
    if let Some(shot) = &shot {
        // Subscribe first, so nothing sent in between is missed (duplicates are dropped).
        p.subscribe(shot)
            .await
            .map_err(|e| format!("Could not subscribe to shots: {e}"))?;
        let (boot, next) = last_shot.map_or((0, 0), |(boot, seq)| (boot, seq + 1));
        p.write(
            shot,
            &proto::encode_shot_request(boot, next),
            WriteType::WithResponse,
        )
        .await
        .map_err(|e| format!("Could not ask for missed shots: {e}"))?;
    }
    // Events after shots: the sensor sends a waiting shot first anyway.
    p.subscribe(&event)
        .await
        .map_err(|e| format!("Could not subscribe to events: {e}"))?;
    let (boot, next) = last_event.map_or((0, 0), |(boot, seq)| (boot, seq + 1));
    p.write(
        &event,
        &proto::encode_event_request(boot, next),
        WriteType::WithResponse,
    )
    .await
    .map_err(|e| format!("Could not ask for missed events: {e}"))?;

    Ok(Opened {
        conn: Connection {
            peripheral: p.clone(),
            name,
            pin,
            shot,
            control,
            calibration,
            settings,
            config_id: None,
        },
        stream,
        name: name_value,
        heartbeat: heartbeat_value,
        status: status_value,
    })
}
