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

use crate::protocol::{self as proto, AdvInfo};

/// A sensor that isn't heard from for this long drops off the "nearby" list.
const SENSOR_STALE_AFTER: Duration = Duration::from_secs(15);
/// Connecting includes the OS pairing prompt, so leave the user time to type the PIN.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);

pub enum Command {
    Connect(String),
    Disconnect,
    SetLed(bool),
    SetName(String),
}

#[derive(Clone, Debug)]
pub struct SensorInfo {
    pub id: String,
    pub name: String,
    pub adv: Option<AdvInfo>,
    pub rssi: Option<i16>,
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
        led: bool,
        heartbeat: Option<u32>,
    },
    /// `reason` is None when the user asked for the disconnect.
    Disconnected {
        reason: Option<String>,
    },
    Heartbeat(u32),
    Led(bool),
    Name(String),
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

struct Connection {
    peripheral: Peripheral,
    led: Characteristic,
    name: Characteristic,
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

    let mut sensors: BTreeMap<String, (PeripheralId, SensorInfo, Instant)> = BTreeMap::new();
    let mut conn: Option<Connection> = None;
    let mut notifications: Option<Notifications> = None;
    let mut prune = tokio::time::interval(Duration::from_secs(2));

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
                    if refresh_sensor(&adapter, &id, &mut sensors).await => {
                        events.send(Event::Sensors(sensor_list(&sensors)));
                    }
                _ => {}
            },

            _ = prune.tick() => {
                let before = sensors.len();
                sensors.retain(|_, (_, _, seen)| seen.elapsed() < SENSOR_STALE_AFTER);
                if sensors.len() != before {
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
                    let peripheral = match adapter.peripheral(pid).await {
                        Ok(p) => p,
                        Err(e) => {
                            events.send(Event::Disconnected { reason: Some(e.to_string()) });
                            continue;
                        }
                    };
                    match tokio::time::timeout(CONNECT_TIMEOUT, open(&peripheral)).await {
                        Ok(Ok((c, stream, name, led, heartbeat))) => {
                            events.send(Event::Connected { id, name, led, heartbeat });
                            conn = Some(c);
                            notifications = Some(stream);
                        }
                        result => {
                            let reason = match result {
                                Ok(Err(e)) => e,
                                _ => "Timed out while connecting.".into(),
                            };
                            let _ = peripheral.disconnect().await;
                            events.send(Event::Disconnected { reason: Some(reason) });
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
                Some(Command::SetLed(on)) => {
                    if let Some(c) = &conn
                        && let Err(e) = c.peripheral.write(&c.led, &proto::encode_led(on), WriteType::WithResponse).await {
                            events.send(Event::Error(format!("Could not switch the LED: {e}")));
                        }
                }
                Some(Command::SetName(name)) => {
                    if let Some(c) = &conn
                        && let Err(e) = c.peripheral.write(&c.name, name.as_bytes(), WriteType::WithResponse).await {
                            events.send(Event::Error(format!("Could not rename the sensor: {e}")));
                        }
                }
            },

            n = next_notification(&mut notifications) => match n {
                Some(n) => {
                    let ev = match n.uuid {
                        u if u == proto::HEARTBEAT_UUID => proto::decode_heartbeat(&n.value).map(Event::Heartbeat),
                        u if u == proto::LED_UUID => proto::decode_led(&n.value).map(Event::Led),
                        u if u == proto::NAME_UUID => proto::decode_name(&n.value).map(Event::Name),
                        _ => None,
                    };
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

/// Waits for the next notification, or forever while not connected.
async fn next_notification(stream: &mut Option<Notifications>) -> Option<ValueNotification> {
    match stream {
        Some(s) => s.next().await,
        None => std::future::pending().await,
    }
}

/// Re-reads a peripheral's advertised data. Returns true if the sensor list changed.
async fn refresh_sensor(
    adapter: &Adapter,
    id: &PeripheralId,
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
    let info = SensorInfo {
        id: id.to_string(),
        name: props
            .local_name
            .or(props.advertisement_name)
            .unwrap_or_else(|| "Unnamed sensor".into()),
        adv,
        rssi: props.rssi,
    };
    let key = info.id.clone();
    let changed = sensors
        .get(&key)
        .is_none_or(|(_, old, _)| old.name != info.name || old.adv != info.adv || old.rssi != info.rssi);
    sensors.insert(key, (id.clone(), info, Instant::now()));
    changed
}

fn sensor_list(sensors: &BTreeMap<String, (PeripheralId, SensorInfo, Instant)>) -> Vec<SensorInfo> {
    sensors.values().map(|(_, info, _)| info.clone()).collect()
}

/// Connects, checks the protocol version, reads the current values and subscribes.
/// The first read needs an encrypted link, so this is where the OS pairing prompt
/// appears if this computer and the sensor are not bonded yet.
async fn open(p: &Peripheral) -> Result<(Connection, Notifications, String, bool, Option<u32>), String> {
    let refused = "The sensor refused the connection. If it isn't paired with this computer yet, \
                   hold BOOT for 2 s until the LED blinks, then press Pair.";

    p.connect().await.map_err(|e| format!("Could not connect: {e}"))?;
    p.discover_services().await.map_err(|_| refused.to_string())?;

    let chars = p.characteristics();
    let find = |uuid| {
        chars.iter().find(|c| c.uuid == uuid).cloned().ok_or_else(|| {
            "This device doesn't look like an ESP Sensor (characteristic missing).".to_string()
        })
    };
    let (version, heartbeat, led, name) = (
        find(proto::VERSION_UUID)?,
        find(proto::HEARTBEAT_UUID)?,
        find(proto::LED_UUID)?,
        find(proto::NAME_UUID)?,
    );

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
    let name_value = proto::decode_name(&p.read(&name).await.map_err(read_err)?).unwrap_or_default();
    let led_value = proto::decode_led(&p.read(&led).await.map_err(read_err)?).unwrap_or(false);
    let heartbeat_value = proto::decode_heartbeat(&p.read(&heartbeat).await.map_err(read_err)?);

    let stream = p.notifications().await.map_err(|e| e.to_string())?;
    for c in [&heartbeat, &led, &name] {
        p.subscribe(c)
            .await
            .map_err(|e| format!("Could not subscribe: {e}"))?;
    }

    Ok((
        Connection {
            peripheral: p.clone(),
            led,
            name,
        },
        stream,
        name_value,
        led_value,
        heartbeat_value,
    ))
}
