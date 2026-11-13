//! Pairing on Linux. BlueZ asks a "pairing agent" for the passkey; desktops only run one
//! while their Bluetooth settings are open, so without our own agent pairing just fails.
//! The app registers an agent and starts pairing itself (`Device1.Pair`), so BlueZ asks
//! this agent. The agent hands the request to the UI, which asks the user for the PIN,
//! and only answers for the sensor being paired. It does not become the system's default
//! agent.
//!
//! Other operating systems show their own pairing prompt: there [`PairingAgent`] does nothing.

use tokio::sync::oneshot;

/// What the agent tells the UI.
pub enum AgentEvent {
    /// BlueZ needs the sensor's PIN. Answer through the [`PasskeyReply`].
    PasskeyRequest(PasskeyReply),
    /// BlueZ gave up on the request (timeout, or the sensor disconnected).
    Cancelled,
}

/// The pending answer to a passkey request. Dropping it cancels the pairing.
pub struct PasskeyReply(oneshot::Sender<Option<u32>>);

impl PasskeyReply {
    /// `None` means the user cancelled.
    pub fn send(self, pin: Option<u32>) {
        let _ = self.0.send(pin); // the request is already over: nothing to answer
    }
}

pub type Notify = Box<dyn Fn(AgentEvent) + Send + Sync>;

#[cfg(target_os = "linux")]
pub use linux::PairingAgent;

#[cfg(not(target_os = "linux"))]
pub struct PairingAgent;

#[cfg(not(target_os = "linux"))]
impl PairingAgent {
    pub async fn register(_notify: Notify) -> Result<Self, String> {
        Ok(Self)
    }
    pub async fn is_paired(&self, _device: &str) -> Option<bool> {
        None // the OS keeps this to itself
    }
    pub async fn forget(&self, _device: &str) -> Result<(), String> {
        Ok(())
    }
    pub async fn pair(&self, _device: &str) -> Result<(), String> {
        Ok(()) // the OS pairs when the first read needs an encrypted link
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::sync::{Arc, Mutex, Weak};
    use std::time::Duration;

    use dbus::channel::{MatchingReceiver, Sender};
    use dbus::message::MatchRule;
    use dbus::nonblock::stdintf::org_freedesktop_dbus::Properties;
    use dbus::nonblock::{Proxy, SyncConnection};
    use dbus::{Message, Path};
    use tokio::sync::oneshot;

    use super::{AgentEvent, Notify, PasskeyReply};

    const AGENT_PATH: &str = "/org/espsensor/agent";
    /// Passkey entry: the sensor "displays" the passkey, the user types it in.
    const CAPABILITY: &str = "KeyboardOnly";
    const CALL_TIMEOUT: Duration = Duration::from_secs(5);
    /// Includes the user typing the PIN. BlueZ itself gives up after 30 s (the BLE limit).
    const PAIR_TIMEOUT: Duration = Duration::from_secs(60);

    pub struct PairingAgent {
        conn: Arc<SyncConnection>,
        /// D-Bus path of the device being paired; the agent only answers for that one.
        target: Arc<Mutex<Option<String>>>,
    }

    impl PairingAgent {
        /// Connects to the system bus and registers the agent with BlueZ. `notify` is
        /// called (from the BLE thread) when the user has to enter the PIN.
        pub async fn register(notify: Notify) -> Result<Self, String> {
            let (resource, conn) =
                dbus_tokio::connection::new_system_sync().map_err(|e| format!("D-Bus: {e}"))?;
            tokio::spawn(async move {
                let _ = resource.await; // runs until the connection drops
            });

            let target: Arc<Mutex<Option<String>>> = Arc::default();
            let t = target.clone();
            // Weak: the handler is owned by the connection, so a strong ref would be a cycle.
            let weak = Arc::downgrade(&conn);
            conn.start_receive(
                MatchRule::new_method_call().with_path(AGENT_PATH),
                Box::new(move |msg, conn| {
                    if let Some(reply) = handle(msg, &t, &notify, &weak) {
                        let _ = conn.send(reply);
                    }
                    true
                }),
            );

            Proxy::new("org.bluez", "/org/bluez", CALL_TIMEOUT, conn.clone())
                .method_call::<(), _, _, _>(
                    "org.bluez.AgentManager1",
                    "RegisterAgent",
                    (Path::from(AGENT_PATH), CAPABILITY),
                )
                .await
                .map_err(|e| format!("Could not register the pairing agent: {e}"))?;
            Ok(Self { conn, target })
        }

        fn device(&self, path: &str, timeout: Duration) -> Proxy<'_, Arc<SyncConnection>> {
            Proxy::new("org.bluez", path.to_owned(), timeout, self.conn.clone())
        }

        /// Whether BlueZ has keys for this device, None if BlueZ can't say (e.g. the entry
        /// is gone). `device` is its D-Bus object path.
        pub async fn is_paired(&self, device: &str) -> Option<bool> {
            self.device(device, CALL_TIMEOUT)
                .get::<bool>("org.bluez.Device1", "Paired")
                .await
                .ok()
        }

        /// Deletes BlueZ's keys (and the device entry; scanning finds it again).
        pub async fn forget(&self, device: &str) -> Result<(), String> {
            let Some((adapter, _)) = device.rsplit_once('/') else {
                return Err(format!("Unexpected device path {device}"));
            };
            Proxy::new("org.bluez", adapter.to_owned(), CALL_TIMEOUT, self.conn.clone())
                .method_call::<(), _, _, _>("org.bluez.Adapter1", "RemoveDevice", (Path::from(device),))
                .await
                .map_err(|e| format!("Could not remove the old pairing: {e}"))
        }

        /// Pairs and bonds with the device; the user enters the PIN when BlueZ asks.
        /// Does nothing if BlueZ already has keys for it.
        pub async fn pair(&self, device: &str) -> Result<(), String> {
            if self.is_paired(device).await == Some(true) {
                return Ok(());
            }
            *self.target.lock().unwrap() = Some(device.to_owned());
            let result = self
                .device(device, PAIR_TIMEOUT)
                .method_call::<(), _, _, _>("org.bluez.Device1", "Pair", ())
                .await;
            *self.target.lock().unwrap() = None;
            let Err(e) = result else { return Ok(()) };
            Err(match e.name().unwrap_or("") {
                "org.bluez.Error.AlreadyExists" => return Ok(()),
                "org.bluez.Error.AuthenticationCanceled" => "Pairing was cancelled.".to_owned(),
                "org.bluez.Error.AuthenticationFailed" | "org.bluez.Error.AuthenticationRejected" => {
                    "Pairing failed. Was the PIN right?".to_owned()
                }
                "org.bluez.Error.AuthenticationTimeout" => "Pairing timed out.".to_owned(),
                _ => format!("Pairing failed: {}", e.message().unwrap_or("unknown error")),
            })
        }
    }

    /// Handles a call from BlueZ on the agent object (org.bluez.Agent1). Returns the reply,
    /// or None if it is sent later (the passkey, once the user has typed it).
    fn handle(
        msg: Message,
        target: &Mutex<Option<String>>,
        notify: &Notify,
        conn: &Weak<SyncConnection>,
    ) -> Option<Message> {
        let device = msg.get1::<Path>().map(|p| p.to_string());
        let ours = device.is_some() && *target.lock().unwrap() == device;
        let member = msg.member().map(|m| m.to_string()); // owned: `msg` may move below
        match member.as_deref() {
            Some("RequestPasskey") if ours => {
                let conn = conn.upgrade()?;
                let (tx, rx) = oneshot::channel();
                notify(AgentEvent::PasskeyRequest(PasskeyReply(tx)));
                tokio::spawn(async move {
                    let reply = match rx.await {
                        Ok(Some(pin)) => msg.method_return().append1(pin),
                        _ => msg.error(&"org.bluez.Error.Canceled".into(), c"Cancelled by the user"),
                    };
                    let _ = conn.send(reply);
                });
                None
            }
            Some("Cancel") => {
                notify(AgentEvent::Cancelled);
                Some(msg.method_return())
            }
            // Nothing to show, and nothing to clean up: return without an error.
            Some("Release" | "DisplayPasskey" | "DisplayPinCode") => Some(msg.method_return()),
            _ => Some(msg.error(
                &"org.bluez.Error.Rejected".into(),
                c"Not an ESP Sensor being paired",
            )),
        }
    }
}
