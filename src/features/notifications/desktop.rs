//! Account-scoped unread state and privacy-preserving desktop alerts.
use megalodon::entities::Notification;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Default, Serialize, Deserialize)]
pub struct Activity {
    #[serde(default)]
    initialized: bool,
    seen: VecDeque<String>,
    unread: Vec<String>,
}
impl Activity {
    pub fn load(account: &str) -> Self {
        path(account)
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }
    pub fn record(&mut self, id: &str) -> bool {
        if self.seen.iter().any(|seen| seen == id) {
            return false;
        }
        self.seen.push_front(id.to_owned());
        self.seen.truncate(512);
        if !self.unread.iter().any(|unread| unread == id) {
            self.unread.push(id.to_owned());
        }
        self.unread.truncate(512);
        true
    }
    pub fn reconcile(&mut self, ids: impl IntoIterator<Item = String>) {
        for id in ids {
            if self.initialized {
                self.record(&id);
            } else if !self.seen.contains(&id) {
                self.seen.push_back(id);
            }
        }
        self.seen.truncate(512);
        self.initialized = true;
    }
    pub fn count(&self) -> usize {
        self.unread.len()
    }
    pub fn read(&mut self) {
        self.unread.clear();
    }
    pub fn save(&self, account: &str) {
        let Some(path) = path(account) else {
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };
        let result = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(parent)?;
            let temporary = path.with_extension("tmp");
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            use std::io::Write;
            options
                .open(&temporary)?
                .write_all(&serde_json::to_vec(self)?)?;
            std::fs::rename(temporary, &path)
        })();
        if result.is_err() {
            tracing::warn!("could not save unread activity state");
        }
    }
}
fn path(account: &str) -> Option<std::path::PathBuf> {
    let mut path = dirs::data_local_dir()?
        .join("dev.edfloreshz.Toot")
        .join("activity");
    for chunk in account.as_bytes().chunks(64) {
        path.push(
            chunk
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
        );
    }
    Some(path.join("unread.json"))
}

pub fn summary(notification: &Notification) -> String {
    let actor = notification
        .account
        .as_ref()
        .map(super::model::actor_name)
        .unwrap_or_else(|| crate::fl!("app-title"));
    format!(
        "{} — {}",
        actor,
        super::view::event_label(&notification.r#type)
    )
}

pub async fn show(summary: String) -> Result<(), ()> {
    // Do not include post content, content warnings, or private-message previews.
    let connection = zbus::Connection::session().await.map_err(|_| ())?;
    send(&connection, summary).await
}

async fn send(connection: &zbus::Connection, summary: String) -> Result<(), ()> {
    let proxy = zbus::Proxy::new(
        connection,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .await
    .map_err(|_| ())?;
    let mut hints = std::collections::HashMap::<&str, zbus::zvariant::Value<'_>>::new();
    hints.insert(
        "desktop-entry",
        zbus::zvariant::Value::from("dev.edfloreshz.Toot"),
    );
    let _: u32 = proxy
        .call(
            "Notify",
            &(
                "Toot",
                0u32,
                "dev.edfloreshz.Toot",
                summary,
                "",
                Vec::<String>::new(),
                hints,
                -1i32,
            ),
        )
        .await
        .map_err(|_| ())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unread_deduplicates_replays_and_survives_roundtrip() {
        let mut activity = Activity::default();
        assert!(activity.record("1"));
        assert!(!activity.record("1"));
        let mut restored: Activity =
            serde_json::from_slice(&serde_json::to_vec(&activity).unwrap()).unwrap();
        assert_eq!(restored.count(), 1);
        restored.read();
        assert_eq!(restored.count(), 0);
        assert!(!restored.record("1"));
        assert!(restored.record("2"));
    }
}

pub fn clear(account: &str) {
    if let Some(path) = path(account) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod reconciliation_tests {
    use super::*;
    #[test]
    fn initial_history_is_read_but_missed_arrivals_and_live_events_are_unread() {
        let mut activity = Activity::default();
        activity.record("live");
        activity.reconcile(["old".into(), "live".into()]);
        assert_eq!(activity.count(), 1);
        activity.read();
        activity.reconcile(["new".into(), "live".into(), "old".into()]);
        assert_eq!(activity.count(), 1);
        assert!(!activity.record("new"));
        let another = Activity::default();
        assert_eq!(another.count(), 0);
        assert_ne!(
            path("https://example.org#alice"),
            path("https://example.org#bob")
        );
    }
}

#[cfg(test)]
mod dbus_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    type Captured = Arc<Mutex<Option<(String, String, String)>>>;
    struct Service(Captured);
    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl Service {
        #[allow(clippy::too_many_arguments)]
        async fn notify(
            &self,
            _app_name: &str,
            _replaces_id: u32,
            _app_icon: &str,
            summary: &str,
            body: &str,
            _actions: Vec<String>,
            hints: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
            _expire_timeout: i32,
        ) -> u32 {
            let desktop = hints
                .get("desktop-entry")
                .and_then(|value| <&str>::try_from(value).ok())
                .unwrap_or_default()
                .to_owned();
            *self.0.lock().unwrap() = Some((summary.into(), body.into(), desktop));
            1
        }
    }
    struct Bus(std::process::Child);
    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    #[test]
    fn desktop_notification_payload_reaches_a_private_mock_bus_with_no_body() {
        use std::io::BufRead;
        let child = std::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("dbus-daemon is required for the notification integration test");
        let mut bus = Bus(child);
        let mut address = String::new();
        std::io::BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = address.trim();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let captured = Captured::default();
            let _service = zbus::connection::Builder::address(address)
                .unwrap()
                .name("org.freedesktop.Notifications")
                .unwrap()
                .serve_at("/org/freedesktop/Notifications", Service(captured.clone()))
                .unwrap()
                .build()
                .await
                .unwrap();
            let client = zbus::connection::Builder::address(address)
                .unwrap()
                .build()
                .await
                .unwrap();
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                send(&client, "Alice — Mention".into()),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(
                *captured.lock().unwrap(),
                Some((
                    "Alice — Mention".into(),
                    String::new(),
                    "dev.edfloreshz.Toot".into()
                ))
            );
        });
    }
}
