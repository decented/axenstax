//! The lobby's people surface: your own address, and (Task 17) the "Friends &
//! servers" column.
//!
//! This is **not** a social surface. It shows people you already know, so you
//! can build with them. There is no browsing, no discovery, and nothing here
//! lists a world (CLAUDE.md red lines 1 and 4). Lives in its own file because
//! `menu.rs` is already 6k lines.
//!
//! Spec: `docs/superpowers/specs/2026-09-06-online-play-by-contact-design.md`
//! §5.3.
#![cfg(not(target_arch = "wasm32"))]

use crate::contacts::Contact;
use crate::menu::MenuAction;
use crate::rendezvous::payload::Candidate;

/// The one-line explanation under "Your address". Pinned by a unit test so the
/// wording cannot drift into discovery or social-network framing.
pub fn your_address_lead() -> &'static str {
    "Give this to a friend so they can invite you."
}

/// An npub abbreviated for a narrow column, keeping the `npub1` prefix (so it
/// still reads as an npub) and the last four characters (so two are
/// distinguishable at a glance). Never hex — [[feedback_npub_only_display]].
pub fn short_npub(npub: &str) -> String {
    if npub.len() > 22 {
        format!("{}…{}", &npub[..14], &npub[npub.len() - 4..])
    } else {
        npub.to_string()
    }
}

/// Draw the "Your address" block: heading, QR, the abbreviated npub, a Copy
/// button, and the lead line. `npub` is `None` for a guest, in which case the
/// block explains what signing in unlocks rather than showing nothing.
pub fn draw_your_address(ui: &mut egui::Ui, npub: Option<&str>) {
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new("Your address")
            .size(16.0)
            .color(egui::Color32::from_rgb(226, 214, 178))
            .strong(),
    );
    ui.add_space(4.0);

    let Some(npub) = npub else {
        ui.label(
            egui::RichText::new(
                "Sign in with your Signet persona to get an address friends can invite.",
            )
            .size(11.0)
            .color(egui::Color32::from_rgb(150, 150, 160)),
        );
        return;
    };

    crate::menu::draw_qr(ui, npub, 132.0);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(short_npub(npub))
                .size(11.0)
                .monospace()
                .color(egui::Color32::from_rgb(200, 200, 210)),
        );
        if ui.small_button("Copy").clicked() {
            ui.ctx().copy_text(npub.to_string());
        }
    });
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(your_address_lead())
            .size(11.0)
            .color(egui::Color32::from_rgb(150, 150, 160)),
    );
}

// ---------------------------------------------------------------------------
// Friends & servers — pure helpers
// ---------------------------------------------------------------------------

/// What the player typed into "Add a friend".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddFriendInput {
    /// A full invite link — adds a pending contact and starts a join at once.
    Invite(Box<crate::invite::Invite>),
    /// A bare npub — adds a contact you can call later, once they are hosting.
    Npub(String),
}

/// Parse the paste box. Returns a message written for a person, not a debug
/// dump, because whatever comes back goes straight under the field.
pub fn parse_add_friend(text: &str, now: u64) -> Result<AddFriendInput, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("Paste an invite link or a friend's npub.".to_string());
    }
    if t.starts_with("axenstax://") {
        return crate::invite::Invite::parse(t, now)
            .map(|i| AddFriendInput::Invite(Box::new(i)))
            .map_err(|e| e.to_string());
    }
    // `PublicKey::parse` also accepts bare hex, so the `npub1` prefix check is
    // what actually refuses hex — deliberately, not incidentally
    // ([[feedback_npub_only_display]]).
    if t.starts_with("npub1") && nostr::PublicKey::parse(t).is_ok() {
        return Ok(AddFriendInput::Npub(t.to_string()));
    }
    Err("That doesn't look like an invite link or an npub. An address starts with \
         'npub1'."
        .to_string())
}

/// Whether this contact is close enough to offer a Join button. Exactly the
/// host's rule (`online_admission::admits_play`), so the button is never
/// offered for a join that would be refused.
pub fn can_join(c: &Contact) -> bool {
    crate::online_admission::admits_play(c.tier)
}

/// The row label: their name if the book has one, otherwise a short npub.
/// Never hex — [[feedback_npub_only_display]].
pub fn contact_row_label(c: &Contact) -> String {
    if let Some(n) = &c.display_name
        && !n.trim().is_empty()
    {
        return n.clone();
    }
    let npub = nostr::PublicKey::from_slice(&c.pubkey)
        .ok()
        .and_then(|pk| nostr::ToBech32::to_bech32(&pk).ok())
        .unwrap_or_else(|| "npub1\u{2026}".to_string());
    short_npub(&npub)
}

/// The one-line explanation under the "Friends" heading. Pinned by a test so it
/// cannot drift into discovery or social-network framing (CLAUDE.md red lines
/// 1 and 4).
pub fn friends_lead() -> &'static str {
    "People you know, to build with."
}

/// What an empty friends list says. It points at the two things that actually
/// add somebody — a link they sent you, or your address sent to them — because
/// there is nowhere to go looking.
pub fn friends_empty_line() -> &'static str {
    "Nobody yet. Paste a friend's invite link below, or send them your address."
}

/// Whether the world card's "Host online" button can be pressed, and the line
/// to show beside it.
///
/// Spec §5.1 step 1 asks for the button to be greyed "with the reason". Only
/// the *signed-out* case actually greys: the runtime attestation is minted **by
/// pressing the button** (step 2 — one phone tap), so greying on a missing
/// attestation would be a dead end you could never leave. Unattested therefore
/// stays pressable and carries a heads-up instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostOnlineState {
    pub enabled: bool,
    pub note: Option<&'static str>,
}

/// See [`HostOnlineState`]. `attested` is "this computer already carries a
/// persona-signed attestation over its runtime key".
pub fn host_online_state(signed_in: bool, attested: bool) -> HostOnlineState {
    if !signed_in {
        return HostOnlineState {
            enabled: false,
            note: Some("Sign in with your Signet persona first — hosting needs an address."),
        };
    }
    if !attested {
        return HostOnlineState {
            enabled: true,
            note: Some("First time: one tap on your phone links this computer to you."),
        };
    }
    HostOnlineState { enabled: true, note: None }
}

/// A one-line, address-free summary of how reachable this host is.
///
/// Places, not protocols: "home network" rather than "LAN", "router mapping"
/// rather than "UPnP", "internet" rather than "STUN reflexive". And no
/// addresses at all — this line is on screen while somebody streams.
pub fn reachability_summary(candidates: &[Candidate]) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for (kind, label) in [
        ("lan", "home network"),
        ("v6", "direct internet address"),
        ("upnp", "router mapping"),
        ("stun", "internet"),
    ] {
        if candidates.iter().any(|c| c.kind == kind) {
            parts.push(label);
        }
    }
    if parts.is_empty() {
        return "Reachable: nothing yet".to_string();
    }
    format!("Reachable: {}", parts.join(", "))
}

/// The line above the "Add a friend" paste box. Pinned here rather than left
/// inline in `menu.rs` so the copy sweep below can see it.
pub fn add_friend_lead() -> &'static str {
    "Paste their invite link, or their address (npub)."
}

/// Whether this contact belongs in the Friends column at all.
///
/// Kin, Kith and Ken are people the player (or their guardian) has actually
/// written down. A `Stranger` row is not a person you know — it is the absence
/// of one — and listing it would turn a column of friends into a list of
/// everyone who has ever turned up, which is the browsing surface this feature
/// must not have (CLAUDE.md red lines 1 and 4).
pub fn shows_in_friends(c: &Contact) -> bool {
    !matches!(c.tier, crate::comms::Tier::Stranger)
}

/// One row of the Online panel's connected list.
///
/// What the player calls them wins; failing that their npub, abbreviated. The
/// name the *server* holds is used only when there is no verified persona at
/// all, because it can be client-asserted — a name is never allowed to stand in
/// front of a key here. Never hex ([[feedback_npub_only_display]]).
pub fn connected_label(display_name: &str, persona: Option<&[u8; 32]>, book: &[Contact]) -> String {
    if let Some(pk) = persona {
        if let Some(c) = crate::contacts::find(book, pk) {
            return contact_row_label(c);
        }
        if let Some(npub) = nostr::PublicKey::from_slice(pk)
            .ok()
            .and_then(|k| nostr::ToBech32::to_bech32(&k).ok())
        {
            return short_npub(&npub);
        }
    }
    let named = display_name.trim();
    if named.is_empty() {
        "Someone".to_string()
    } else {
        named.to_string()
    }
}

/// The success toast (spec §5.2 step 3).
///
/// When the only name we have for the host IS the world's name — a link pasted
/// from somebody who isn't in the book yet — saying it twice ("Joined Ivy's
/// Hollow at Ivy's Hollow's.") reads as a bug, so it is said once.
pub fn joined_toast(world_name: &str, host_name: &str) -> String {
    let world = if world_name.trim().is_empty() { "the world" } else { world_name.trim() };
    let host = host_name.trim();
    if host.is_empty() || host == world {
        format!("Joined {world}.")
    } else {
        format!("Joined {world} at {host}\u{2019}s.")
    }
}

// ---------------------------------------------------------------------------
// Friends & servers — panels
// ---------------------------------------------------------------------------

/// Everything the Online panel needs to draw. A view struct so the panel can be
/// rendered from the lobby or from the in-game `/online` overlay without either
/// reaching into `OnlineHost`.
pub struct OnlinePanelView<'a> {
    pub invite_link: &'a str,
    pub candidates: &'a [Candidate],
    pub relays: (usize, usize),
    /// Setup events the relay layer had to throw away because the game loop
    /// was behind. Zero in every ordinary session; shown when it is not,
    /// because "your friend called and we dropped it" is otherwise invisible.
    pub dropped: usize,
    pub warning: Option<&'a str>,
    /// Who is in the world right now, already rendered by
    /// [`connected_label`] — names or npubs, never hex, and never a list of
    /// anybody who is not actually here.
    pub connected: &'a [String],
}

/// QR + copy button + reachability + relay count + who is here + any warning.
pub fn draw_online_panel(ui: &mut egui::Ui, view: &OnlinePanelView) {
    ui.label(egui::RichText::new("Playing online").size(16.0).strong());
    ui.add_space(4.0);
    crate::menu::draw_qr(ui, view.invite_link, 148.0);
    ui.add_space(4.0);
    if ui.button("Copy invite link").clicked() {
        ui.ctx().copy_text(view.invite_link.to_string());
    }
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(reachability_summary(view.candidates))
            .size(11.0)
            .color(egui::Color32::from_rgb(160, 175, 160)),
    );
    let (up, total) = view.relays;
    ui.label(
        egui::RichText::new(format!("Relays connected: {up}/{total}"))
            .size(11.0)
            .color(egui::Color32::from_rgb(150, 150, 160)),
    );
    if view.dropped > 0 {
        ui.label(
            egui::RichText::new(format!(
                "Busy — {} call{} were dropped. Ask your friend to try again.",
                view.dropped,
                if view.dropped == 1 { "" } else { "s" }
            ))
            .size(11.0)
            .color(egui::Color32::from_rgb(230, 190, 120)),
        );
    }
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(if view.connected.is_empty() {
            "Nobody here yet.".to_string()
        } else {
            format!("Here now: {}", view.connected.len())
        })
        .size(11.0)
        .color(egui::Color32::from_rgb(200, 200, 210)),
    );
    for who in view.connected {
        ui.label(egui::RichText::new(format!("\u{2022} {who}")).size(11.0));
    }
    if let Some(w) = view.warning {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(w)
                .size(11.0)
                .color(egui::Color32::from_rgb(230, 190, 120)),
        );
    }
}

/// The "Friends & servers" column: your address, the people you know, the paste
/// box, then the saved-servers list.
///
/// This lists **people you already know**. There is nothing to browse and
/// nobody new to find here — that is the point (CLAUDE.md red lines 1 and 4).
pub fn draw_friends_column(ui: &mut egui::Ui, state: &mut crate::menu::MenuState) -> MenuAction {
    let mut action = MenuAction::None;

    egui::ScrollArea::vertical()
        .id_salt("friends_column")
        .show(ui, |ui| {
            let identity = crate::signet::native_signer::load_identity();
            draw_your_address(ui, identity.npub().as_deref());
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);

            ui.label(egui::RichText::new("Friends").size(16.0).strong());
            ui.label(
                egui::RichText::new(friends_lead())
                    .size(11.0)
                    .color(egui::Color32::from_rgb(150, 150, 160)),
            );
            ui.add_space(4.0);

            if !state.contacts.iter().any(shows_in_friends) {
                ui.label(
                    egui::RichText::new(friends_empty_line())
                        .size(11.0)
                        .color(egui::Color32::from_rgb(150, 150, 160)),
                );
            }

            // Snapshot so the row loop can stage an action without holding a
            // borrow of `state`.
            let rows: Vec<(String, String, bool)> = state
                .contacts
                .iter()
                .filter(|c| shows_in_friends(c))
                .map(|c| (hex::encode(c.pubkey), contact_row_label(c), can_join(c)))
                .collect();
            for (persona_hex, label, joinable) in rows {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&label).size(13.0));
                    if joinable && ui.small_button("Join").clicked() {
                        action = MenuAction::JoinContact {
                            persona_hex: persona_hex.clone(),
                            display_name: label.clone(),
                        };
                    }
                });
            }

            ui.add_space(8.0);
            if ui.button("Add a friend").clicked() {
                state.dialog = crate::menu::MenuDialog::AddFriend {
                    input: String::new(),
                    error: None,
                };
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);
            crate::signet_contacts::ui::draw(ui, &state.relays);

            ui.add_space(10.0);
            ui.separator();
            let servers_action = crate::menu::draw_my_servers_column_inner(ui, state);
            if !matches!(servers_action, MenuAction::None) {
                action = servers_action;
            }
        });

    action
}

#[cfg(test)]
mod tests {
    use super::*;

    const NPUB: &str = "npub1sg6plzptd64u62a878hep2kev88swjh3tw00gjsfl8f237lmu63q0uf63m";

    #[test]
    fn the_lead_line_is_the_approved_copy() {
        // Pinned so the wording can't drift: it must explain what the address is
        // FOR (being invited), not invite anyone to find anyone. No discovery
        // language, no social-network framing (CLAUDE.md red line 4).
        assert_eq!(
            your_address_lead(),
            "Give this to a friend so they can invite you."
        );
    }

    #[test]
    fn the_lead_line_carries_no_discovery_or_money_words() {
        let lead = your_address_lead().to_lowercase();
        for banned in [
            "browse", "discover", "find players", "directory", "public",
            "earn", "sats", "bitcoin", "money", "social network",
        ] {
            assert!(!lead.contains(banned), "lead copy must not say {banned:?}");
        }
    }

    #[test]
    fn short_npub_keeps_the_npub_prefix_and_the_tail() {
        let s = short_npub(NPUB);
        assert!(s.starts_with("npub1"), "must still read as an npub: {s}");
        assert!(s.ends_with(&NPUB[NPUB.len() - 4..]), "tail preserved: {s}");
        assert!(s.contains('…'));
        assert!(s.len() < NPUB.len());
    }

    #[test]
    fn short_npub_leaves_a_short_string_alone() {
        assert_eq!(short_npub("npub1abc"), "npub1abc");
    }

    use crate::comms::Tier;
    use crate::contacts::{AddedVia, Contact};
    use crate::rendezvous::payload::Candidate;

    fn contact(tier: Tier, name: Option<&str>) -> Contact {
        Contact {
            pubkey: [3u8; 32],
            display_name: name.map(str::to_string),
            tier,
            is_child: false,
            runtime_pubkey: None,
            added_via: AddedVia::Kenspeckle,
            added_at: 0,
            last_joined: None,
        }
    }

    #[test]
    fn only_kin_and_kith_get_a_join_button() {
        // The column SHOWS ken (they are somebody you recognise) but does not
        // offer to take you into their world — the same rule the host applies.
        assert!(can_join(&contact(Tier::Kin, Some("Mum"))));
        assert!(can_join(&contact(Tier::Kith, Some("Rowan"))));
        assert!(!can_join(&contact(Tier::Ken, Some("Someone"))));
        assert!(!can_join(&contact(Tier::Stranger, None)));
    }

    #[test]
    fn a_nameless_contact_falls_back_to_a_short_npub_never_hex() {
        let label = contact_row_label(&contact(Tier::Kith, None));
        assert!(label.starts_with("npub1"), "must render as an npub: {label}");
        assert!(label.contains('\u{2026}'));
    }

    #[test]
    fn a_named_contact_shows_its_name() {
        assert_eq!(contact_row_label(&contact(Tier::Kin, Some("Mum"))), "Mum");
    }

    #[test]
    fn add_friend_accepts_an_invite_link() {
        let inv = crate::invite::Invite {
            host_persona: NPUB.to_string(),
            host_runtime: [0xab; 32],
            relays: vec!["wss://nos.lol".to_string()],
            bearer: [1u8; 16],
            expires_at: 2_000_000_000,
            world_name: "Ivy's Hollow".to_string(),
        };
        match parse_add_friend(&inv.to_link(), 1_000_000_000) {
            Ok(AddFriendInput::Invite(got)) => assert_eq!(*got, inv),
            other => panic!("expected an invite, got {other:?}"),
        }
    }

    #[test]
    fn add_friend_accepts_a_bare_npub() {
        match parse_add_friend(&format!("  {NPUB}  "), 0) {
            Ok(AddFriendInput::Npub(got)) => assert_eq!(got, NPUB),
            other => panic!("expected an npub, got {other:?}"),
        }
    }

    #[test]
    fn add_friend_refuses_hex_and_says_why() {
        // Hex is internal only ([[feedback_npub_only_display]]); accepting it
        // here would teach players to pass it around.
        let err = parse_add_friend(&"ab".repeat(32), 0).unwrap_err();
        assert!(err.contains("npub"), "{err}");
    }

    #[test]
    fn add_friend_explains_an_expired_invite_in_plain_words() {
        let inv = crate::invite::Invite {
            host_persona: NPUB.to_string(),
            host_runtime: [0xab; 32],
            relays: vec!["wss://nos.lol".to_string()],
            bearer: [1u8; 16],
            expires_at: 100,
            world_name: "W".to_string(),
        };
        let err = parse_add_friend(&inv.to_link(), 200).unwrap_err();
        assert!(err.contains("expired"), "{err}");
        assert!(!err.contains("InviteError"), "no debug formatting: {err}");
    }

    #[test]
    fn add_friend_refuses_empty_input() {
        assert!(parse_add_friend("   ", 0).is_err());
    }

    #[test]
    fn the_reachability_summary_names_places_not_protocols() {
        let s = reachability_summary(&[
            Candidate { kind: "lan".to_string(), addr: "192.168.1.2:7700".to_string() },
            Candidate { kind: "upnp".to_string(), addr: "198.51.100.7:7700".to_string() },
            Candidate { kind: "stun".to_string(), addr: "203.0.113.9:41000".to_string() },
        ]);
        assert_eq!(s, "Reachable: home network, router mapping, internet");
        assert!(!s.contains("192.168"), "a summary must not leak addresses: {s}");
    }

    #[test]
    fn the_reachability_summary_says_so_when_only_the_home_network_works() {
        let s = reachability_summary(&[Candidate {
            kind: "lan".to_string(),
            addr: "192.168.1.2:7700".to_string(),
        }]);
        assert_eq!(s, "Reachable: home network");
    }

    #[test]
    fn the_reachability_summary_handles_nothing_at_all() {
        assert_eq!(reachability_summary(&[]), "Reachable: nothing yet");
    }

    #[test]
    fn host_online_is_blocked_until_you_have_an_address_to_be_invited_by() {
        let s = host_online_state(false, false);
        assert!(!s.enabled, "nothing to host as");
        let note = s.note.expect("a greyed button must say why");
        assert!(note.contains("Sign in"), "{note}");
    }

    #[test]
    fn host_online_is_offered_before_the_first_phone_tap_and_says_it_is_coming() {
        // Greying here would be a dead end: the one tap is minted BY pressing
        // the button (spec 5.1 step 2), so the button must stay pressable.
        let s = host_online_state(true, false);
        assert!(s.enabled, "the phone tap happens after the press, not before");
        let note = s.note.expect("warn about the tap before it interrupts them");
        assert!(note.contains("phone"), "{note}");
    }

    #[test]
    fn host_online_is_plain_once_signed_in_and_attested() {
        let s = host_online_state(true, true);
        assert!(s.enabled);
        assert_eq!(s.note, None);
    }

    #[test]
    fn a_stranger_is_never_a_row_in_the_friends_column() {
        // A Stranger is the absence of a contact, not a contact. Listing them
        // would turn a column of people you know into a list of everyone who
        // has ever turned up — the browsing surface this feature must not have.
        assert!(shows_in_friends(&contact(Tier::Kin, Some("Mum"))));
        assert!(shows_in_friends(&contact(Tier::Kith, Some("Rowan"))));
        assert!(
            shows_in_friends(&contact(Tier::Ken, Some("Someone"))),
            "ken is somebody you recognise — shown, but with no Join button"
        );
        assert!(!shows_in_friends(&contact(Tier::Stranger, Some("Nobody"))));
    }

    #[test]
    fn a_connected_player_is_named_by_what_you_call_them() {
        let book = vec![contact(Tier::Kin, Some("Mum"))];
        // `contact()` uses [3u8; 32] as the pubkey.
        assert_eq!(connected_label("whatever", Some(&[3u8; 32]), &book), "Mum");
    }

    #[test]
    fn a_connected_stranger_is_named_by_npub_not_by_a_name_they_chose() {
        // The server-side handle can be client-asserted. A name must never
        // stand in front of a key here.
        let pk = nostr::PublicKey::parse(NPUB).unwrap().to_bytes();
        let label = connected_label("Definitely Mum", Some(&pk), &[]);
        assert!(label.starts_with("npub1"), "{label}");
        assert!(!label.contains("Mum"));
    }

    #[test]
    fn a_connected_player_with_no_verified_key_at_all_still_reads_as_a_person() {
        assert_eq!(connected_label("Rowan", None, &[]), "Rowan");
        assert_eq!(connected_label("   ", None, &[]), "Someone");
    }

    #[test]
    fn the_joined_toast_is_the_spec_sentence() {
        assert_eq!(
            joined_toast("Ivy's Hollow", "Rowan"),
            "Joined Ivy's Hollow at Rowan\u{2019}s."
        );
    }

    #[test]
    fn the_joined_toast_does_not_say_the_same_name_twice() {
        // A pasted invite from somebody not in the book has only ONE human word
        // in it — the world's name. "Joined X at X's." reads as a bug.
        assert_eq!(joined_toast("Ivy's Hollow", "Ivy's Hollow"), "Joined Ivy's Hollow.");
        assert_eq!(joined_toast("Ivy's Hollow", "  "), "Joined Ivy's Hollow.");
        assert_eq!(joined_toast("", "Rowan"), "Joined the world at Rowan\u{2019}s.");
    }

    #[test]
    fn the_online_copy_carries_no_discovery_money_or_social_words() {
        let mut copy = vec![
            reachability_summary(&[]),
            friends_lead().to_string(),
            friends_empty_line().to_string(),
            your_address_lead().to_string(),
            add_friend_lead().to_string(),
            joined_toast("Ivy's Hollow", "Rowan"),
            connected_label("", None, &[]),
            "Nobody here yet.".to_string(),
        ];
        // Every message the Add-a-friend box can put under its field, including
        // the ones that come out of the invite parser.
        let inv = crate::invite::Invite {
            host_persona: NPUB.to_string(),
            host_runtime: [0xab; 32],
            relays: vec!["wss://nos.lol".to_string()],
            bearer: [1u8; 16],
            expires_at: 100,
            world_name: "W".to_string(),
        };
        for (text, now) in [
            ("", 0u64),
            ("   ", 0),
            ("not a link at all", 0),
            ("ab00cd", 0),
            ("axenstax://nonsense", 0),
            ("axenstax://invite/v9/whatever", 0),
            (inv.to_link().as_str(), 200),
        ] {
            if let Err(e) = parse_add_friend(text, now) {
                assert!(!e.contains("InviteError"), "no debug formatting: {e}");
                copy.push(e);
            }
        }
        for (a, b) in [(false, false), (true, false), (true, true)] {
            if let Some(n) = host_online_state(a, b).note {
                copy.push(n.to_string());
            }
        }
        for line in &copy {
            let lower = line.to_lowercase();
            for banned in [
                "browse", "discover", "directory", "server list", "public",
                "earn", "sats", "bitcoin", "money", "social network", "chat platform",
            ] {
                assert!(!lower.contains(banned), "copy must not say {banned:?}: {line}");
            }
        }
    }
}
