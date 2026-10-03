//! Permanent guard (CLAUDE.md red line 2): AxeNStax's own relay,
//! `relay.trotters.cc`, is never a DEFAULT anywhere in the native app. A player
//! may add it to "Your relays"; the build never ships it.
//!
//! Walks every default / const relay list the engine connects with on anyone's
//! behalf — the player's default list and everything derived from it (sign-in
//! QR, contacts pairing, discovery, release feed), the feedback inbox, and the
//! operator's pairing relay — and fails on any `trotters` host. Extends the
//! `default_relays_are_public_only` pattern in `graphics_settings`.
//!
//! The ONLY sanctioned appearance is [`ALLOWED`]: places where trotters is
//! named in order to PROHIBIT it, never to use it. Adding to that list needs a
//! red-line reason in the comment beside the entry.

const OURS: &str = "trotters";

/// Explicit allow-list — trotters named as a prohibition, not a default.
const ALLOWED: &[(&str, &str)] = &[
    // world_room's lint refuses trotters in any room relay list it is given;
    // the constant names the host it forbids (red-line enforcement, D6).
    ("world_room::FORBIDDEN_RELAY_HOST", crate::world_room::FORBIDDEN_RELAY_HOST),
];

fn assert_public(name: &str, relays: &[String]) {
    assert!(!relays.is_empty(), "{name}: a relay default must never be empty");
    for r in relays {
        assert!(
            !r.to_ascii_lowercase().contains(OURS),
            "red line 2: {name} ships AxeNStax's own relay `{r}` as a default"
        );
        assert!(r.starts_with("wss://"), "{name}: `{r}` is not wss://");
    }
}

fn owned(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| s.to_string()).collect()
}

#[test]
fn no_relay_default_names_our_own_relay() {
    let player_default = crate::graphics_settings::default_online_relays();

    let lists: Vec<(&str, Vec<String>)> = vec![
        ("server_resolve::PUBLIC_DEFAULT_RELAYS", owned(&crate::server_resolve::PUBLIC_DEFAULT_RELAYS)),
        ("graphics_settings::default_online_relays (Your relays)", player_default.clone()),
        ("GraphicsSettings::default().online_relays", crate::graphics_settings::GraphicsSettings::default().online_relays),
        ("server_resolve_native::default_relays (discovery)", crate::server_resolve_native::default_relays()),
        ("native_mailbox::FEEDBACK_INBOX_RELAYS", owned(&crate::native_mailbox::FEEDBACK_INBOX_RELAYS)),
        ("server_identity::DEFAULT_PAIR_RELAY (operator pairing + admin)", owned(&[crate::server_identity::DEFAULT_PAIR_RELAY])),
        ("native_signin::signin_relays(default) (sign-in QR)", crate::native_signin::signin_relays(&player_default)),
        ("native_signin::signin_relays(empty) (sign-in fallback)", crate::native_signin::signin_relays(&[])),
        ("signet_contacts::pairing_relay(default) (contacts pairing)", vec![crate::signet_contacts::pairing_relay(&player_default)]),
        ("signet_contacts::pairing_relay(empty) (pairing fallback)", vec![crate::signet_contacts::pairing_relay(&[])]),
        // The release feed has no list of its own: `update_check::start_once`
        // is handed the player's list, so its default IS `player_default`.
        ("relays_ui::reset (Reset to defaults)", crate::relays_ui::reset()),
    ];
    for (name, relays) in &lists {
        assert_public(name, relays);
    }
}

#[test]
fn the_allow_list_only_holds_prohibitions() {
    // Each allowed entry must actually be the forbidden host — if it ever
    // stops naming trotters, it no longer needs to be on this list.
    for (name, value) in ALLOWED {
        assert!(value.contains(OURS), "{name} is allow-listed but no longer names trotters");
    }
}

#[test]
fn the_feedback_inbox_is_independent_of_the_player_list() {
    // D3: a player who customises "Your relays" must still reach the project,
    // so the inbox is a fixed set — and must not silently equal (and so drift
    // with) the player defaults.
    let inbox = owned(&crate::native_mailbox::FEEDBACK_INBOX_RELAYS);
    assert!((2..=3).contains(&inbox.len()), "2–3 inbox relays");
    assert_ne!(inbox, crate::graphics_settings::default_online_relays());
}
