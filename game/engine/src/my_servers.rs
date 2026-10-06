//! "My Servers" — the player's local list of servers they've joined (Spec A).
//!
//! Directed resolution + local memory, **no public directory**. The operator
//! npub is the stable key; the cached endpoint/name are a convenience (the live
//! address is re-resolved on join). Persisted cross-platform with the same
//! backend shape as `graphics_settings` (a JSON file natively; localStorage on
//! web).

use serde::{Deserialize, Serialize};

/// One remembered server, keyed by its operator npub.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MyServerEntry {
    pub operator_npub: String,
    pub name: String,
    pub last_endpoint: String,
    pub last_joined_unix: u64,
    #[serde(default)]
    pub favourite: bool,
    /// The Card `privacy` tag value the player acknowledged for this server
    /// (Spec C §5); `None` = not yet acknowledged. Preserved across re-joins.
    #[serde(default)]
    pub privacy_ack: Option<String>,
}

/// The player's saved server list.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MyServers {
    #[serde(default)]
    pub entries: Vec<MyServerEntry>,
}

impl MyServers {
    /// Load the saved list (empty if absent/unparseable).
    pub fn load() -> Self {
        load_raw()
            .and_then(|json| serde_json::from_str::<MyServers>(&json).ok())
            .unwrap_or_default()
    }

    /// Persist (best-effort — failure never breaks the game).
    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            save_raw(&json);
        }
    }

    /// Record a successful join: insert or update in place (dedup by npub),
    /// preserving an existing `favourite` flag.
    pub fn record_join(&mut self, operator_npub: &str, name: &str, endpoint: &str, now_unix: u64) {
        if let Some(e) = self
            .entries
            .iter_mut()
            .find(|e| e.operator_npub == operator_npub)
        {
            e.name = name.to_string();
            e.last_endpoint = endpoint.to_string();
            e.last_joined_unix = now_unix;
        } else {
            self.entries.push(MyServerEntry {
                operator_npub: operator_npub.to_string(),
                name: name.to_string(),
                last_endpoint: endpoint.to_string(),
                last_joined_unix: now_unix,
                favourite: false,
                privacy_ack: None,
            });
        }
    }

    /// Forget a server.
    pub fn remove(&mut self, operator_npub: &str) {
        self.entries.retain(|e| e.operator_npub != operator_npub);
    }

    /// Toggle the favourite flag (no-op if the npub isn't present).
    #[allow(dead_code)] // My Servers UI (Spec A task 10) is not built; tested only
    pub fn set_favourite(&mut self, operator_npub: &str, favourite: bool) {
        if let Some(e) = self
            .entries
            .iter_mut()
            .find(|e| e.operator_npub == operator_npub)
        {
            e.favourite = favourite;
        }
    }

    /// Record the player's acknowledgement of a server's privacy posture
    /// (the Card `privacy` tag value); no-op if the npub isn't present.
    #[allow(dead_code)] // My Servers UI (Spec A task 10) is not built; tested only
    pub fn set_privacy_ack(&mut self, operator_npub: &str, tag_value: &str) {
        if let Some(e) = self
            .entries
            .iter_mut()
            .find(|e| e.operator_npub == operator_npub)
        {
            e.privacy_ack = Some(tag_value.to_string());
        }
    }

    /// The privacy tag value the player acknowledged for this server, if any.
    #[allow(dead_code)] // My Servers UI (Spec A task 10) is not built; tested only
    pub fn acked(&self, operator_npub: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.operator_npub == operator_npub)
            .and_then(|e| e.privacy_ack.as_deref())
    }

    /// Display order: favourites first, then most-recently-joined.
    pub fn sorted(&self) -> Vec<&MyServerEntry> {
        let mut v: Vec<&MyServerEntry> = self.entries.iter().collect();
        v.sort_by(|a, b| {
            b.favourite
                .cmp(&a.favourite)
                .then(b.last_joined_unix.cmp(&a.last_joined_unix))
        });
        v
    }
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // the web localStorage backend is dead — the web build is a login-free taster with no My Servers
const STORAGE_KEY: &str = "axenstax_servers";

#[cfg(not(target_arch = "wasm32"))]
fn servers_path() -> std::path::PathBuf {
    crate::data_dir::data_root().join("my_servers.json")
}

#[cfg(not(target_arch = "wasm32"))]
fn load_raw() -> Option<String> {
    std::fs::read_to_string(servers_path()).ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn save_raw(json: &str) {
    let _ = std::fs::write(servers_path(), json);
}

#[cfg(target_arch = "wasm32")]
fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

#[cfg(target_arch = "wasm32")]
fn load_raw() -> Option<String> {
    local_storage()?.get_item(STORAGE_KEY).ok()?
}

#[cfg(target_arch = "wasm32")]
fn save_raw(json: &str) {
    if let Some(store) = local_storage() {
        let _ = store.set_item(STORAGE_KEY, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_join_inserts_then_updates_no_dup() {
        let mut m = MyServers::default();
        m.record_join("npub1a", "Alpha", "wss://a/ws", 100);
        assert_eq!(m.entries.len(), 1);
        m.record_join("npub1a", "Alpha2", "wss://a2/ws", 200);
        assert_eq!(m.entries.len(), 1, "same npub must not duplicate");
        assert_eq!(m.entries[0].name, "Alpha2");
        assert_eq!(m.entries[0].last_endpoint, "wss://a2/ws");
        assert_eq!(m.entries[0].last_joined_unix, 200);
    }

    #[test]
    fn record_join_preserves_favourite() {
        let mut m = MyServers::default();
        m.record_join("npub1a", "Alpha", "wss://a/ws", 100);
        m.set_favourite("npub1a", true);
        m.record_join("npub1a", "Alpha", "wss://a/ws", 300);
        assert!(m.entries[0].favourite, "re-join must keep the favourite flag");
    }

    #[test]
    fn remove_drops_entry() {
        let mut m = MyServers::default();
        m.record_join("npub1a", "A", "wss://a", 1);
        m.record_join("npub1b", "B", "wss://b", 2);
        m.remove("npub1a");
        assert_eq!(m.entries.len(), 1);
        assert_eq!(m.entries[0].operator_npub, "npub1b");
    }

    #[test]
    fn set_favourite_toggles() {
        let mut m = MyServers::default();
        m.record_join("npub1a", "A", "wss://a", 1);
        m.set_favourite("npub1a", true);
        assert!(m.entries[0].favourite);
        m.set_favourite("npub1a", false);
        assert!(!m.entries[0].favourite);
    }

    #[test]
    fn sorted_favourites_first_then_recent() {
        let mut m = MyServers::default();
        m.record_join("npub1a", "A", "wss://a", 100);
        m.record_join("npub1b", "B", "wss://b", 200);
        m.record_join("npub1c", "C", "wss://c", 50);
        m.set_favourite("npub1c", true);
        let s = m.sorted();
        assert_eq!(s[0].operator_npub, "npub1c", "favourite first");
        assert_eq!(s[1].operator_npub, "npub1b", "then most recent");
        assert_eq!(s[2].operator_npub, "npub1a");
    }

    #[test]
    fn privacy_ack_set_get_and_preserved_on_rejoin() {
        let mut m = MyServers::default();
        m.record_join("npub1a", "A", "wss://a", 10);
        assert_eq!(m.acked("npub1a"), None);
        m.set_privacy_ack("npub1a", "sessions:30");
        assert_eq!(m.acked("npub1a"), Some("sessions:30"));
        // A re-join must preserve the acknowledgement (like favourite).
        m.record_join("npub1a", "A", "wss://a2", 20);
        assert_eq!(m.acked("npub1a"), Some("sessions:30"));
        assert_eq!(m.entries.len(), 1);
    }
}
