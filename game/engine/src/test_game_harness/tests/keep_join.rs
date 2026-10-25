//! Join Game and My Keeps for a Keep invite link, through the real lobby frame: the arms of
//! `update_and_render` that `keeps_tests.rs` can only test as pure functions. Signed out or
//! without a stored session nothing is dialled; a Keep not in My Keeps asks first; a saved
//! Keep and a My Keeps row dial straight away; and `pending_keep` is cleared when the join
//! fails and when a code join starts.
//!
//! No relay is configured, so a join that is started (`fray_join` is `Some`) reaches nobody.
use super::*;
use crate::keeps::{self, Invite};
use crate::menu::{MenuAction, MenuDialog};
use crate::signet::native_signer::{cache_identity_to, clear_identity_at, clear_session_at, identity_path, save_session_to, session_path};

const HOST: [u8; 32] = [0xab; 32];
const SECRET: [u8; 32] = [0x01; 32];

/// The player's Signet files in the test data root, removed when the test ends.
struct SignetFiles;

impl SignetFiles {
    fn cache_identity(&self) {
        cache_identity_to(&identity_path(), &"1".repeat(64)).unwrap();
    }

    fn store_session(&self) {
        let stored = signet_nip46_client::PersistedSession {
            app_secret_key: "a".repeat(64),
            bunker_uri: "bunker://79dff8f82963424e0bb02708a22e44b4980893e3a4be0fa3cb60a43b946764e3?relay=wss://relay.example.org".to_string(),
        };
        save_session_to(&session_path(), &stored).unwrap();
    }

    fn sign_out(&self) {
        clear_identity_at(&identity_path()).unwrap();
        clear_session_at(&session_path()).unwrap();
    }
}

impl Drop for SignetFiles {
    fn drop(&mut self) {
        self.sign_out();
        let _ = std::fs::remove_file(keeps::path());
    }
}

fn lobby() -> crate::GameState {
    let mut state = pollster::block_on(crate::GameState::new_headless(1280, 720));
    state.graphics.online_relays = Vec::new();
    state.mode = GameMode::Menu(Box::new(crate::menu::MenuState::new()));
    state
}

/// The player does `action` in the lobby: it is the action of the next frame the lobby draws
/// (the lobby is throttled to about 20 frames a second, so a call may pump a few).
fn act(state: &mut crate::GameState, action: MenuAction) {
    state.test_menu_action = Some(action);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while state.test_menu_action.is_some() {
        state.update_and_render();
        assert!(std::time::Instant::now() < deadline, "the lobby never drew a frame");
    }
}

fn menu(state: &crate::GameState) -> &crate::menu::MenuState {
    let GameMode::Menu(menu) = &state.mode else { panic!("still in the lobby") };
    menu
}

/// Nothing dialled, nothing pending, no confirm up.
fn assert_idle(state: &crate::GameState, why: &str) {
    assert!(state.fray_join.is_none(), "{why}: nothing dialled");
    assert!(state.pending_keep.is_none(), "{why}: no Keep pending");
    assert!(matches!(menu(state).dialog, MenuDialog::None), "{why}: no dialog");
}

/// A join was started for the Keep: dialled, pending, no dialog; then dropped.
fn assert_dialled_and_drop(state: &mut crate::GameState, invite: Invite, why: &str) {
    let join = state.fray_join.take().unwrap_or_else(|| panic!("{why}: the Keep was dialled"));
    assert_eq!(join.label(), keeps::label(&invite.host_key), "{why}");
    assert_eq!(state.pending_keep.take(), Some(invite), "{why}: the Keep is pending until seated");
    assert!(matches!(menu(state).dialog, MenuDialog::None), "{why}: no confirm");
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_a_keep_link_signs_in_first_asks_once_and_dials_a_saved_keep() {
    let signet = SignetFiles;
    signet.sign_out();
    let _ = std::fs::remove_file(keeps::path());
    let invite = Invite { host_key: HOST, secret: SECRET };
    let link = fray_keep::format_invite_link(keeps::LINK_PREFIX, &HOST, &SECRET);
    let mut state = lobby();

    // Signed out: told to sign in, nothing dialled. A Keep link never falls through to the
    // room-code or server arms.
    act(&mut state, MenuAction::JoinGame(link.clone()));
    assert_idle(&state, "signed out");
    assert_eq!(menu(&state).notice.as_deref(), Some(keeps::SIGN_IN_FIRST));
    // The same from a My Keeps row.
    act(&mut state, MenuAction::JoinKeep(invite));
    assert_idle(&state, "signed out, My Keeps");
    assert_eq!(menu(&state).notice.as_deref(), Some(keeps::SIGN_IN_FIRST));

    // The cached npub without a stored session is not signed in.
    signet.cache_identity();
    act(&mut state, MenuAction::JoinGame(link.clone()));
    assert_idle(&state, "cached npub only");
    assert_eq!(menu(&state).notice.as_deref(), Some(keeps::SIGN_IN_FIRST));
    act(&mut state, MenuAction::JoinKeep(invite));
    assert_idle(&state, "cached npub only, My Keeps");

    // Signed in and the Keep not in My Keeps: the confirm, and nothing dialled until Join.
    signet.store_session();
    act(&mut state, MenuAction::JoinGame(link.clone()));
    assert!(state.fray_join.is_none() && state.pending_keep.is_none(), "unsaved: nothing dialled before Join");
    assert!(matches!(menu(&state).dialog, MenuDialog::JoinKeep { invite: asked } if asked == invite), "unsaved: the confirm");
    assert!(menu(&state).notice.is_none());
    // Join in the confirm closes it and sends the same action My Keeps' row sends: it dials,
    // with no second confirm.
    if let GameMode::Menu(menu) = &mut state.mode {
        menu.dialog = MenuDialog::None;
    }
    act(&mut state, MenuAction::JoinKeep(invite));
    assert_dialled_and_drop(&mut state, invite, "after the confirm");

    // The Keep is now saved: the pasted link dials with no confirm.
    assert!(keeps::save_invite_join(&keeps::path(), &invite, Some("Ada's Keep"), 100));
    act(&mut state, MenuAction::JoinGame(link.clone()));
    assert_dialled_and_drop(&mut state, invite, "saved Keep, pasted link");
    // A signed-out player with it saved is still told to sign in first.
    signet.sign_out();
    act(&mut state, MenuAction::JoinGame(link.clone()));
    assert_idle(&state, "signed out, saved Keep");
    signet.cache_identity();
    signet.store_session();

    // A damaged link says so and dials nothing.
    act(&mut state, MenuAction::JoinGame(link[..link.len() - 1].to_string()));
    assert_idle(&state, "damaged link");
    assert_eq!(menu(&state).notice.as_deref(), Some(keeps::NOT_A_KEEP_LINK));

    // A code join clears a Keep that was pending (the player moved on to another join).
    state.pending_keep = Some(invite);
    act(&mut state, MenuAction::JoinGame("KMT RWX".to_string()));
    assert!(state.fray_join.take().is_some(), "the code join started");
    assert!(state.pending_keep.is_none(), "a code join clears the pending Keep");

    // A join that fails clears it too, and says why in the lobby.
    act(&mut state, MenuAction::JoinKeep(invite));
    assert_eq!(state.pending_keep, Some(invite));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while state.fray_join.is_some() {
        state.update_and_render();
        assert!(std::time::Instant::now() < deadline, "the join with no relay never failed");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(state.pending_keep.is_none(), "a failed join clears the pending Keep");
    assert!(menu(&state).notice.as_deref().is_some_and(|n| !n.is_empty()), "the lobby says why");
}
