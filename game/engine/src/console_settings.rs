#![cfg(not(target_arch = "wasm32"))]
//! Operator console settings (Spec B §5) — the descriptor / capacity / announce /
//! privacy fields an operator sets via admin commands, persisted to `console.json`
//! in the identity dir. The running server reads these on its policy reload; the
//! Card publisher (Spec A) and the snapshot (Spec B task 5) read them too.

use serde::{Deserialize, Serialize};

use crate::server_identity::AdminCommand;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsoleSettings {
    /// Player cap override (`None` ⇒ use the CLI `--max-players`).
    #[serde(default)]
    pub max_players: Option<u16>,
    /// Whether the server announces a Server Card (Spec A).
    #[serde(default)]
    pub announce: bool,
    /// Display name override (`None` ⇒ use the CLI `--name`).
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub about: String,
    #[serde(default)]
    pub region: String,
    /// Privacy level (Spec C defines the values; "none"/"sessions"/…).
    #[serde(default)]
    pub privacy_level: String,
    #[serde(default)]
    pub privacy_retention_days: u32,
}

impl ConsoleSettings {
    /// Load from `<dir>/console.json` (defaults if absent/unparseable).
    pub fn load(dir: &std::path::Path) -> Self {
        std::fs::read_to_string(dir.join("console.json"))
            .ok()
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default()
    }

    /// Persist to `<dir>/console.json`.
    pub fn save(&self, dir: &std::path::Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let j = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("console.json"), j).map_err(|e| e.to_string())
    }

    /// Apply a settings command in place (no-op for non-settings commands).
    pub fn apply(&mut self, cmd: &AdminCommand) {
        match cmd {
            AdminCommand::SetMaxPlayers(n) => self.max_players = Some(*n),
            AdminCommand::SetAnnounce(b) => self.announce = *b,
            AdminCommand::SetServerName(s) => self.server_name = Some(s.clone()),
            AdminCommand::SetAbout(s) => self.about = s.clone(),
            AdminCommand::SetRegion(s) => self.region = s.clone(),
            AdminCommand::SetPrivacy(level, days) => {
                self.privacy_level = level.clone();
                self.privacy_retention_days = *days;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_sets_each_field() {
        let mut s = ConsoleSettings::default();
        s.apply(&AdminCommand::SetMaxPlayers(20));
        assert_eq!(s.max_players, Some(20));
        s.apply(&AdminCommand::SetAnnounce(true));
        assert!(s.announce);
        s.apply(&AdminCommand::SetServerName("Cool SMP".into()));
        assert_eq!(s.server_name.as_deref(), Some("Cool SMP"));
        s.apply(&AdminCommand::SetAbout("no griefing".into()));
        assert_eq!(s.about, "no griefing");
        s.apply(&AdminCommand::SetRegion("eu-west".into()));
        assert_eq!(s.region, "eu-west");
        s.apply(&AdminCommand::SetPrivacy("sessions".into(), 30));
        assert_eq!(s.privacy_level, "sessions");
        assert_eq!(s.privacy_retention_days, 30);
    }

    #[test]
    fn non_settings_command_is_noop() {
        let mut s = ConsoleSettings::default();
        s.apply(&AdminCommand::SetRequireSignin(true));
        assert_eq!(s, ConsoleSettings::default());
    }

    #[test]
    fn load_apply_save_round_trips() {
        let dir =
            std::env::temp_dir().join(format!("axe_console_settings_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = ConsoleSettings::load(&dir); // default (no file yet)
        s.apply(&AdminCommand::SetServerName("X".into()));
        s.apply(&AdminCommand::SetMaxPlayers(12));
        s.apply(&AdminCommand::SetPrivacy("none".into(), 0));
        s.save(&dir).unwrap();
        let loaded = ConsoleSettings::load(&dir);
        assert_eq!(loaded.server_name.as_deref(), Some("X"));
        assert_eq!(loaded.max_players, Some(12));
        assert_eq!(loaded.privacy_level, "none");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
