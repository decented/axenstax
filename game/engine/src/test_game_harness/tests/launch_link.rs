//! A tapped `axenstax:` link through the real frame: held while the player is in a world (it is
//! never dropped out of it), and handled the first time they are back at the menu, by the same
//! Join Game arm a paste reaches (`keep_join.rs` covers that arm's own cases).
//!
//! The Android read is replaced by handing the link to `launch_links` as the JNI reader would.
use super::*;
use crate::keeps::{self, Invite};
use crate::launch_links::IntentRead;
use crate::menu::MenuDialog;
use crate::signet::native_signer::{cache_identity_to, clear_identity_at, clear_session_at, identity_path, save_session_to, session_path};

const HOST: [u8; 32] = [0xab; 32];
const SECRET: [u8; 32] = [0x01; 32];

/// Signed in (a cached npub and a stored session), removed when the test ends.
struct SignedIn;

impl SignedIn {
    fn new() -> Self {
        cache_identity_to(&identity_path(), &"1".repeat(64)).unwrap();
        let stored = signet_nip46_client::PersistedSession {
            app_secret_key: "a".repeat(64),
            bunker_uri: "bunker://79dff8f82963424e0bb02708a22e44b4980893e3a4be0fa3cb60a43b946764e3?relay=wss://relay.example.org".to_string(),
        };
        save_session_to(&session_path(), &stored).unwrap();
        SignedIn
    }
}

impl Drop for SignedIn {
    fn drop(&mut self) {
        let _ = clear_identity_at(&identity_path());
        let _ = clear_session_at(&session_path());
        let _ = std::fs::remove_file(keeps::path());
    }
}

fn pump(state: &mut crate::GameState, frames: usize) {
    for _ in 0..frames {
        state.update_and_render();
        std::thread::sleep(std::time::Duration::from_millis(60));
    }
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_a_tapped_keep_link_waits_for_the_menu_then_asks() {
    let _signed_in = SignedIn::new();
    let _ = std::fs::remove_file(keeps::path());
    let link = fray_keep::format_invite_link(keeps::LINK_PREFIX, &HOST, &SECRET);
    let mut state = pollster::block_on(crate::GameState::new_headless(1280, 720));
    state.graphics.online_relays = Vec::new();

    // In a world: the link arrives (the poll holds it) and nothing happens to the player.
    state.mode = GameMode::Playing;
    state.launch_links.poll(std::time::Instant::now(), || IntentRead::Link { text: link.clone(), consumed: true });
    assert!(state.launch_links.has_held());
    pump(&mut state, 5);
    assert!(matches!(state.mode, GameMode::Playing), "still in the world");
    assert!(state.launch_links.has_held(), "held, not dropped, while in a world");
    assert!(state.fray_join.is_none() && state.pending_keep.is_none(), "nothing dialled");

    // Paused is a world too.
    state.mode = GameMode::Paused { confirm_quit: false, confirm_creative: false };
    pump(&mut state, 3);
    assert!(state.launch_links.has_held(), "held while paused");

    // Back at the menu (a fresh menu, as leaving a world builds one): the first frame that draws
    // it hands the link to Join Game's arm. The Keep is not saved, so it asks.
    state.mode = GameMode::Menu(Box::new(crate::menu::MenuState::new()));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while state.launch_links.has_held() {
        state.update_and_render();
        assert!(std::time::Instant::now() < deadline, "the held link was never handled at the menu");
    }
    let GameMode::Menu(menu) = &state.mode else { panic!("still at the menu") };
    assert!(
        matches!(menu.dialog, MenuDialog::JoinKeep { invite } if invite == Invite { host_key: HOST, secret: SECRET }),
        "the first-time confirm is up"
    );
    assert!(state.fray_join.is_none(), "nothing dialled before Join");
}
