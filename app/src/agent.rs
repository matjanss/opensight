//! Pairing on Linux. BlueZ asks a "pairing agent" for the passkey; desktops only run one
//! while their Bluetooth settings are open, so without our own agent pairing just fails.
//! The app registers an agent and starts pairing itself (`Device1.Pair`), so BlueZ asks
//! this agent, which answers with the fixed passkey for the sensor being paired, and
//! rejects everything else. It does not become the system's default agent.
//!
//! Other operating systems show their own pairing prompt: there [`PairingAgent`] does nothing.

#[cfg(target_os = "linux")]
pub use linux::PairingAgent;

#[cfg(not(target_os = "linux"))]
pub struct PairingAgent;

#[cfg(not(target_os = "linux"))]
impl PairingAgent {
    pub async fn register() -> Result<Self, String> {
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
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use dbus::channel::{MatchingReceiver, Sender};
    use dbus::message::MatchRule;
    use dbus::nonblock::stdintf::org_freedesktop_dbus::Properties;
    use dbus::nonblock::{Proxy, SyncConnection};
    use dbus::{Message, Path};

    use crate::protocol;

    const AGENT_PATH: &str = "/org/espsensor/agent";
    /// Passkey entry: the sensor "displays" the passkey, we type it in.
    const CAPABILITY: &str = "KeyboardOnly";
    const CALL_TIMEOUT: Duration = Duration::from_secs(5);
    /// Pairing itself takes a second or two; this only stops a hung BlueZ call.
    const PAIR_TIMEOUT: Duration = Duration::from_secs(30);

    pub struct PairingAgent {
        conn: Arc<SyncConnection>,
        /// D-Bus path of the device being paired; the agent only answers for that one.
        target: Arc<Mutex<Option<String>>>,
    }

    impl PairingAgent {
        /// Connects to the system bus and registers the agent with BlueZ.
        pub async fn register() -> Result<Self, String> {
            let (resource, conn) =
                dbus_tokio::connection::new_system_sync().map_err(|e| format!("D-Bus: {e}"))?;
            tokio::spawn(async move {
                let _ = resource.await; // runs until the connection drops
            });

            let target: Arc<Mutex<Option<String>>> = Arc::default();
            let t = target.clone();
            conn.start_receive(
                MatchRule::new_method_call().with_path(AGENT_PATH),
                Box::new(move |msg, conn| {
                    let _ = conn.send(answer(&msg, &t));
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

        /// Pairs and bonds with the device, answering the passkey request ourselves.
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
            match result {
                Ok(()) => Ok(()),
                Err(e) if e.name() == Some("org.bluez.Error.AlreadyExists") => Ok(()),
                Err(e) => Err(e.message().unwrap_or("unknown error").to_owned()),
            }
        }
    }

    /// Replies to a call from BlueZ on the agent object (org.bluez.Agent1).
    fn answer(msg: &Message, target: &Mutex<Option<String>>) -> Message {
        let device = msg.get1::<Path>().map(|p| p.to_string());
        let ours = device.is_some() && *target.lock().unwrap() == device;
        let reject = || {
            msg.error(
                &"org.bluez.Error.Rejected".into(),
                c"Not an ESP Sensor being paired",
            )
        };
        match msg.member().as_deref() {
            Some("RequestPasskey") if ours => match protocol::PASSKEY.parse::<u32>() {
                Ok(passkey) => msg.method_return().append1(passkey),
                Err(_) => reject(),
            },
            Some("RequestPinCode") if ours => msg.method_return().append1(protocol::PASSKEY),
            // Nothing to show, and nothing to clean up: return without an error.
            Some("Release" | "Cancel" | "DisplayPasskey" | "DisplayPinCode") => msg.method_return(),
            _ => reject(),
        }
    }
}
