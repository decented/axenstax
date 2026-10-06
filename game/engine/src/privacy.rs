//! Privacy level taxonomy + encoding (Spec C). The single source of truth for
//! the `none` / `sessions:<days>` levels and their encoding — bridging the Spec A
//! Card `privacy` tag, the Spec B `ConsoleSettings`, and the player-facing badge.
//! Cross-platform (the badge runs on the client; the gating on the server).

/// What a server persists about players. Default = `None` (Spec C §2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrivacyLevel {
    /// No tracking — nothing about players is persisted.
    #[default]
    None,
    /// Session history kept for `retention_days`, then auto-purged.
    Sessions { retention_days: u32 },
}

/// Encode to the Card `privacy` tag value: `"none"` / `"sessions:<days>"`.
pub fn encode(level: &PrivacyLevel) -> String {
    match level {
        PrivacyLevel::None => "none".to_string(),
        PrivacyLevel::Sessions { retention_days } => format!("sessions:{retention_days}"),
    }
}

/// Decode a Card `privacy` tag value. Unknown/malformed ⇒ `None` (safe default).
#[allow(dead_code)] // Spec C client-side badge / notice — no UI shows it yet; tested only
pub fn decode(s: &str) -> PrivacyLevel {
    if let Some(days) = s.strip_prefix("sessions:")
        && let Ok(d) = days.parse::<u32>()
    {
        return PrivacyLevel::Sessions { retention_days: d };
    }
    PrivacyLevel::None
}

/// Bridge from `ConsoleSettings` (its `privacy_level` string + retention days).
pub fn from_settings(privacy_level: &str, retention_days: u32) -> PrivacyLevel {
    match privacy_level {
        "sessions" => PrivacyLevel::Sessions { retention_days },
        _ => PrivacyLevel::None,
    }
}

/// The player-facing badge shown at server selection (Spec C §5).
#[allow(dead_code)] // Spec C client-side badge / notice — no UI shows it yet; tested only
pub fn badge(level: &PrivacyLevel) -> String {
    match level {
        PrivacyLevel::None => "🔒 No tracking".to_string(),
        PrivacyLevel::Sessions { retention_days } => {
            format!("📋 Keeps session history · {retention_days} days")
        }
    }
}

/// Whether the server persists session history at this level (Spec C §4) —
/// `None` ⇒ nothing is written to disk.
pub fn should_persist(level: &PrivacyLevel) -> bool {
    matches!(level, PrivacyLevel::Sessions { .. })
}

/// The retention cutoff: sessions that connected before this unix time are
/// purged (Spec C §6). `None` when not tracking.
pub fn retention_cutoff(level: &PrivacyLevel, now_unix: u64) -> Option<u64> {
    match level {
        PrivacyLevel::None => None,
        PrivacyLevel::Sessions { retention_days } => {
            Some(now_unix.saturating_sub((*retention_days as u64) * 86_400))
        }
    }
}

/// Whether to show the player a pre-join privacy notice (Spec C §5). Never for
/// `None` (nothing to consent to); for `Sessions`, only until the player has
/// acknowledged THIS posture — re-prompts if the level/retention changed.
#[allow(dead_code)] // Spec C client-side badge / notice — no UI shows it yet; tested only
pub fn should_show_notice(level: &PrivacyLevel, acked: Option<&str>) -> bool {
    match level {
        PrivacyLevel::None => false,
        PrivacyLevel::Sessions { .. } => {
            let tag = encode(level);
            acked != Some(tag.as_str())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_round_trips() {
        assert_eq!(encode(&PrivacyLevel::None), "none");
        assert_eq!(
            encode(&PrivacyLevel::Sessions { retention_days: 30 }),
            "sessions:30"
        );
        assert_eq!(decode("none"), PrivacyLevel::None);
        assert_eq!(
            decode("sessions:30"),
            PrivacyLevel::Sessions { retention_days: 30 }
        );
    }

    #[test]
    fn decode_unknown_is_none() {
        assert_eq!(decode("garbage"), PrivacyLevel::None);
        assert_eq!(decode(""), PrivacyLevel::None);
        assert_eq!(decode("sessions:notanumber"), PrivacyLevel::None);
    }

    #[test]
    fn from_settings_bridges() {
        assert_eq!(
            from_settings("sessions", 30),
            PrivacyLevel::Sessions { retention_days: 30 }
        );
        assert_eq!(from_settings("none", 0), PrivacyLevel::None);
        assert_eq!(from_settings("", 0), PrivacyLevel::None);
    }

    #[test]
    fn badges() {
        assert!(badge(&PrivacyLevel::None).contains("No tracking"));
        assert!(badge(&PrivacyLevel::Sessions { retention_days: 30 }).contains("30"));
    }

    #[test]
    fn should_persist_only_for_sessions() {
        assert!(!should_persist(&PrivacyLevel::None));
        assert!(should_persist(&PrivacyLevel::Sessions { retention_days: 30 }));
    }

    #[test]
    fn retention_cutoff_subtracts_days_saturating() {
        assert_eq!(retention_cutoff(&PrivacyLevel::None, 1_000_000), None);
        let now = 10_000_000u64;
        assert_eq!(
            retention_cutoff(&PrivacyLevel::Sessions { retention_days: 1 }, now),
            Some(now - 86_400)
        );
        assert_eq!(
            retention_cutoff(&PrivacyLevel::Sessions { retention_days: u32::MAX }, 100),
            Some(0),
            "huge retention saturates to 0, no underflow"
        );
    }

    #[test]
    fn notice_only_for_unacked_tracking() {
        assert!(!should_show_notice(&PrivacyLevel::None, None));
        assert!(!should_show_notice(&PrivacyLevel::None, Some("whatever")));
        let s30 = PrivacyLevel::Sessions { retention_days: 30 };
        assert!(should_show_notice(&s30, None));
        assert!(!should_show_notice(&s30, Some("sessions:30")));
        assert!(
            should_show_notice(&s30, Some("sessions:7")),
            "posture changed → re-prompt"
        );
    }
}
