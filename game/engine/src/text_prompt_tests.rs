//! Tests for `text_prompt.rs`: the broker's rules on a scripted backend and the
//! lobby's screens as prompt owners. Every test builds its own broker; the
//! process-wide one is never touched.

use super::*;

/// Any cap, for tests where it does not matter.
const ANY_MAX: usize = 64;

/// A scripted backend: counts the prompts it showed and answers what the test
/// tells it to.
struct Fake {
    shown: Vec<String>,
    can_show: bool,
    next: PromptPoll,
    polls: usize,
}

impl Fake {
    fn new() -> Self {
        Self { shown: Vec::new(), can_show: true, next: PromptPoll::Waiting, polls: 0 }
    }
}

impl PromptBackend for Fake {
    fn show(&mut self, label: &str, _initial: &str, _kind: PromptKind, _max: usize) -> bool {
        if self.can_show {
            self.shown.push(label.to_string());
            self.next = PromptPoll::Waiting;
        }
        self.can_show
    }
    fn poll(&mut self) -> PromptPoll {
        self.polls += 1;
        // A closed prompt answers once; after that the backend knows nothing.
        match &self.next {
            PromptPoll::Waiting => PromptPoll::Waiting,
            _ => std::mem::replace(&mut self.next, PromptPoll::Gone),
        }
    }
}

fn ask(b: &mut PromptBroker<Fake>, key: &str) -> bool {
    b.request(key, key, "", PromptKind::Plain, ANY_MAX)
}

#[test]
fn a_request_is_answered_once_to_its_key() {
    let mut b = PromptBroker::new(Fake::new());
    assert!(ask(&mut b, "name"));
    assert_eq!(b.backend.shown, vec!["name"]);
    assert_eq!(b.take("name"), None, "nothing typed yet");
    b.backend.next = PromptPoll::Answered(Some("Phone Test".into()));
    assert_eq!(b.take("name"), Some(Some("Phone Test".into())));
    assert_eq!(b.take("name"), None, "delivered at most once");
    assert!(!b.is_open());
}

#[test]
fn a_second_request_while_one_is_open_is_ignored() {
    let mut b = PromptBroker::new(Fake::new());
    assert!(ask(&mut b, "name"));
    assert!(!ask(&mut b, "seed"), "one prompt at a time");
    assert!(!ask(&mut b, "name"), "not even the same field twice");
    assert_eq!(b.backend.shown, vec!["name"], "only the first dialog was shown");
    b.backend.next = PromptPoll::Answered(Some("x".into()));
    assert_eq!(b.take("seed"), None, "the ignored request gets nothing");
    assert_eq!(b.take("name"), Some(Some("x".into())));
}

#[test]
fn an_answer_is_never_delivered_to_another_key() {
    let mut b = PromptBroker::new(Fake::new());
    assert!(ask(&mut b, "name"));
    b.backend.next = PromptPoll::Answered(Some("Castle".into()));
    assert_eq!(b.take(CHAT_KEY), None, "chat polls every frame; it must not get the name");
    assert_eq!(b.take("seed"), None);
    assert_eq!(b.take("name"), Some(Some("Castle".into())));
}

#[test]
fn a_stale_answer_is_dropped_by_the_next_request() {
    let mut b = PromptBroker::new(Fake::new());
    assert!(ask(&mut b, "rename"));
    b.backend.next = PromptPoll::Answered(Some("Old".into()));
    // Its screen closed before collecting: another screen polls its own key...
    assert_eq!(b.take("seed"), None);
    // ...and then asks for a prompt of its own.
    assert!(ask(&mut b, "seed"), "a closed prompt frees the broker");
    assert_eq!(b.take("rename"), None, "the stale answer was dropped");
    b.backend.next = PromptPoll::Answered(Some("12345".into()));
    assert_eq!(b.take("seed"), Some(Some("12345".into())), "the new field gets only its own answer");
    assert_eq!(b.take("rename"), None);
}

#[test]
fn a_stale_answer_nobody_polled_is_dropped_too() {
    // The answer arrives after its screen stopped polling and no other
    // screen polled either: the next request must still clear it.
    let mut b = PromptBroker::new(Fake::new());
    assert!(ask(&mut b, "rename"));
    b.backend.next = PromptPoll::Answered(Some("Old".into()));
    assert!(ask(&mut b, "seed"), "an answered prompt is not an open one");
    assert_eq!(b.take("rename"), None);
    assert_eq!(b.backend.shown, vec!["rename", "seed"]);
}

#[test]
fn a_backend_that_never_answers_does_not_leave_a_dead_keyboard() {
    let mut b = PromptBroker::new(Fake::new());
    assert!(ask(&mut b, "name"));
    // The dialog never posts a result (the activity was recreated under it)...
    for _ in 0..100 {
        assert_eq!(b.take("name"), None);
    }
    assert!(!ask(&mut b, "name"), "still open as far as the backend says");
    // ...and the activity reports it no longer knows the prompt.
    b.backend.next = PromptPoll::Gone;
    assert_eq!(b.take("name"), Some(None), "the asker hears a cancel");
    assert!(ask(&mut b, "name"), "the next tap opens a new prompt");
    assert_eq!(b.backend.shown.len(), 2);
    b.backend.next = PromptPoll::Answered(Some("Back".into()));
    assert_eq!(b.take("name"), Some(Some("Back".into())));
}

#[test]
fn a_backend_that_cannot_show_answers_cancel_at_once() {
    let mut b = PromptBroker::new(Fake::new());
    b.backend.can_show = false;
    assert!(ask(&mut b, "name"), "asked, even though nothing could be shown");
    assert!(!b.is_open());
    assert_eq!(b.take("name"), Some(None));
    b.backend.can_show = true;
    assert!(ask(&mut b, "name"), "and the next tap still works");
    assert!(b.is_open());
}

#[test]
fn cancel_is_delivered_as_none_once() {
    let mut b = PromptBroker::new(Fake::new());
    assert!(ask(&mut b, CHAT_KEY));
    b.backend.next = PromptPoll::Answered(None);
    assert_eq!(b.take(CHAT_KEY), Some(None), "Cancel / Back / outside tap");
    assert_eq!(b.take(CHAT_KEY), None, "and only once: nothing re-opens it");
    assert_eq!(b.backend.shown.len(), 1);
}

#[test]
fn the_backend_is_polled_only_while_a_prompt_is_open() {
    let mut b = PromptBroker::new(Fake::new());
    for _ in 0..10 {
        assert_eq!(b.take(CHAT_KEY), None);
    }
    assert_eq!(b.backend.polls, 0, "no JNI call per frame when nothing is open");
    assert!(ask(&mut b, CHAT_KEY));
    b.backend.next = PromptPoll::Answered(Some("hi".into()));
    assert_eq!(b.take(CHAT_KEY), Some(Some("hi".into())));
    let after = b.backend.polls;
    for _ in 0..10 {
        assert_eq!(b.take(CHAT_KEY), None);
    }
    assert_eq!(b.backend.polls, after);
}

#[test]
fn a_web_style_backend_answers_in_the_same_frame() {
    // window.prompt() answers inside `show`; the caller polls right after
    // asking, so the field fills on the tap's own frame, as it always did.
    struct Blocking(Option<Option<String>>);
    impl PromptBackend for Blocking {
        fn show(&mut self, _l: &str, initial: &str, _k: PromptKind, _m: usize) -> bool {
            self.0 = Some(Some(format!("{initial}!")));
            true
        }
        fn poll(&mut self) -> PromptPoll {
            self.0.take().map_or(PromptPoll::Gone, PromptPoll::Answered)
        }
    }
    let mut b = PromptBroker::new(Blocking(None));
    assert!(b.request("name", "World name", "My World", PromptKind::Name, ANY_MAX));
    assert_eq!(b.take("name"), Some(Some("My World!".into())));
    // And through a menu field, on the tap's own frame too.
    let mut field = "My World".to_string();
    b.field("World name", &mut field, true);
    assert_eq!(field, "My World!");
}

#[test]
fn field_kinds_follow_their_captions() {
    assert_eq!(PromptKind::for_field("World name"), PromptKind::Name);
    assert_eq!(PromptKind::for_field("Rename skin"), PromptKind::Name);
    assert_eq!(PromptKind::for_field("Seed (optional)"), PromptKind::Plain);
    assert_eq!(PromptKind::for_field("Type the world name to confirm"), PromptKind::Plain);
    assert_eq!(PromptKind::for_field("Minecraft username"), PromptKind::Plain);
    let codes = [PromptKind::Name.code(), PromptKind::Chat.code(), PromptKind::Plain.code()];
    assert_eq!(codes, [0, 1, 2], "AxenstaxActivity's KIND_* constants");
}

// ── Rule 5: a prompt belongs to the screen that asked ──────────────────────────

#[test]
fn an_answer_for_a_screen_that_has_gone_is_dropped() {
    let mut b = PromptBroker::new(Fake::new());
    b.enter_screen(screen_id("play together"));
    assert!(ask(&mut b, "field:Room code"));
    // The screen changes while the prompt is still open...
    b.enter_screen(screen_id("join game"));
    // ...and then the old dialog is answered.
    b.backend.next = PromptPoll::Answered(Some("ABCD-1234".into()));
    assert_eq!(b.take("field:Room code"), None, "the new screen's field of the same name never gets it");
    assert!(!b.is_open(), "the closed dialog still frees the broker");
    b.enter_screen(screen_id("play together"));
    assert_eq!(b.take("field:Room code"), None, "and it is gone, not waiting for the old screen");
}

#[test]
fn a_held_answer_is_dropped_when_its_screen_goes_even_if_that_screen_reopens() {
    let mut b = PromptBroker::new(Fake::new());
    b.enter_screen(screen_id("create"));
    assert!(ask(&mut b, "field:World name"));
    b.backend.next = PromptPoll::Answered(Some("Castle".into()));
    // Collected by another key's poll and held, uncollected by its own field.
    assert_eq!(b.take(CHAT_KEY), None);
    // Create closes and a new Create opens.
    b.enter_screen(screen_id("page"));
    b.enter_screen(screen_id("create"));
    assert_eq!(b.take("field:World name"), None, "a screen opened again is a new owner");
}

#[test]
fn an_invalidated_prompt_frees_the_broker_when_its_dialog_closes() {
    let mut b = PromptBroker::new(Fake::new());
    b.enter_screen(screen_id("create"));
    assert!(ask(&mut b, "field:Seed (optional)"));
    b.enter_screen(screen_id("join game"));
    assert!(!ask(&mut b, "field:Room code"), "the old dialog is still up");
    // The old dialog closes; nobody ever polls its key again.
    b.backend.next = PromptPoll::Answered(Some("12345".into()));
    assert!(ask(&mut b, "field:Room code"), "the next tap opens a prompt");
    assert_eq!(b.take("field:Room code"), None, "the seed did not land here");
    b.backend.next = PromptPoll::Answered(Some("ROOM-42".into()));
    assert_eq!(b.take("field:Room code"), Some(Some("ROOM-42".into())));
}

#[test]
fn naming_the_same_screen_every_frame_keeps_its_prompt() {
    let mut b = PromptBroker::new(Fake::new());
    let create = screen_id("create");
    b.enter_screen(create);
    assert!(ask(&mut b, "field:World name"));
    for _ in 0..30 {
        b.enter_screen(create);
        assert_eq!(b.take("field:World name"), None);
    }
    let polls = b.backend.polls;
    b.enter_screen(create);
    assert_eq!(b.backend.polls, polls, "naming the screen asks the backend nothing");
    b.backend.next = PromptPoll::Answered(Some("Castle".into()));
    b.enter_screen(create);
    assert_eq!(b.take("field:World name"), Some(Some("Castle".into())));
}

#[test]
fn a_field_takes_an_answer_only_while_it_holds_what_the_prompt_opened_with() {
    let mut b = PromptBroker::new(Fake::new());
    let mut field = "old".to_string();
    b.field("Room code", &mut field, true);
    // Something else rewrote the field while the prompt was open.
    field = "replaced".to_string();
    b.backend.next = PromptPoll::Answered(Some("typed".into()));
    b.field("Room code", &mut field, false);
    assert_eq!(field, "replaced");
    b.field("Room code", &mut field, false);
    assert_eq!(field, "replaced", "and the answer is gone, not held for later");
}

#[cfg(not(target_arch = "wasm32"))]
mod lobby {
    //! The reviewer's cases, through the lobby's own screen names and the real
    //! link handler (`menu.rs`'s touch field calls `text_field_prompt`, which is
    //! `PromptBroker::field` on the process's broker).
    use super::*;
    use crate::launch_links::{IntentRead, LaunchLinks};
    use crate::menu::{LobbyPage, MenuDialog, MenuState, new_create_dialog, prompt_screen};
    use crate::worlds_ui::words::ROOM_CODE;
    use std::time::Instant;

    /// Join Game's own field caption (`menu.rs` `draw_join_dialog`, native).
    const JOIN_GAME_FIELD: &str = "Room code";

    fn tap_link(menu: &mut MenuState, link: &str) {
        let mut links = LaunchLinks::new();
        links.poll(Instant::now(), || IntentRead::Link { text: link.to_string(), consumed: true });
        let _ = links.offer(&mut menu.dialog, true);
    }

    fn join_game_address(menu: &mut MenuState) -> &mut String {
        match &mut menu.dialog {
            MenuDialog::JoinDirect { address } => address,
            other => panic!("Join Game is not open: {other:?}"),
        }
    }

    #[test]
    fn a_link_that_opens_join_game_never_gets_the_play_together_room_code() {
        let mut menu = MenuState::new();
        menu.page = LobbyPage::PlayTogether;
        let mut b = PromptBroker::new(Fake::new());
        b.enter_screen(prompt_screen(&menu));
        let mut join_code = String::new();
        b.field(ROOM_CODE, &mut join_code, true);
        assert!(b.is_open(), "Play together's Room code prompt is up");

        // While it is up, a tapped server link opens Join Game with its address.
        let link = "axenstax://play.example.com:8080";
        tap_link(&mut menu, link);
        assert_eq!(join_game_address(&mut menu), link);

        // The next lobby frame: Join Game's own "Room code" field polls, and the
        // old prompt is answered.
        b.enter_screen(prompt_screen(&menu));
        b.backend.next = PromptPoll::Answered(Some("ABCD-1234".into()));
        b.field(JOIN_GAME_FIELD, join_game_address(&mut menu), false);
        assert_eq!(join_game_address(&mut menu), link, "the linked address is not overwritten");
        assert!(!b.is_open());

        // Back on Play together, the answer is not there either.
        menu.dialog = MenuDialog::None;
        b.enter_screen(prompt_screen(&menu));
        b.field(ROOM_CODE, &mut join_code, false);
        assert_eq!(join_code, "");
    }

    #[test]
    fn a_link_that_replaces_join_game_keeps_its_address() {
        let mut menu = MenuState::new();
        menu.dialog = MenuDialog::JoinDirect { address: String::new() };
        let mut b = PromptBroker::new(Fake::new());
        b.enter_screen(prompt_screen(&menu));
        b.field(JOIN_GAME_FIELD, join_game_address(&mut menu), true);
        assert!(b.is_open());

        // A link replaces Join Game with a new Join Game: the same kind of screen.
        let link = "axenstax://play.example.com:8080";
        tap_link(&mut menu, link);
        b.enter_screen(prompt_screen(&menu));
        b.backend.next = PromptPoll::Answered(Some("ABCD-1234".into()));
        b.field(JOIN_GAME_FIELD, join_game_address(&mut menu), false);
        assert_eq!(join_game_address(&mut menu), link, "the old prompt does not overwrite the link");
    }

    #[test]
    fn edit_world_never_gets_the_name_typed_for_create() {
        let mut menu = MenuState::new();
        menu.dialog = new_create_dialog();
        let mut b = PromptBroker::new(Fake::new());
        b.enter_screen(prompt_screen(&menu));
        // The player had typed "Castle" and taps the name again to change it.
        if let MenuDialog::Create { name, .. } = &mut menu.dialog {
            *name = "Castle".into();
            b.field("World name", name, true);
        }
        assert!(b.is_open());

        // The game takes the player out of Create, and Edit World opens on a world
        // with that same name.
        menu.dialog = MenuDialog::Edit { world_idx: 0, name: "Castle".into(), description: String::new() };
        b.enter_screen(prompt_screen(&menu));
        b.backend.next = PromptPoll::Answered(Some("Typed for Create".into()));
        let MenuDialog::Edit { name, .. } = &mut menu.dialog else { unreachable!() };
        b.field("World name", name, false);
        assert_eq!(name, "Castle", "Edit World keeps its own name");
    }

    #[test]
    fn the_lobby_names_its_screen_by_page_or_dialog_kind() {
        let mut menu = MenuState::new();
        let home = prompt_screen(&menu);
        menu.page = LobbyPage::PlayTogether;
        let play_together = prompt_screen(&menu);
        assert_ne!(home, play_together);
        menu.dialog = MenuDialog::JoinDirect { address: "a".into() };
        let join_game = prompt_screen(&menu);
        assert_ne!(join_game, play_together);
        menu.dialog = MenuDialog::JoinDirect { address: "ab".into() };
        assert_eq!(prompt_screen(&menu), join_game, "typing in a dialog is the same screen");
        menu.dialog = new_create_dialog();
        let create = prompt_screen(&menu);
        menu.dialog = MenuDialog::Edit { world_idx: 0, name: String::new(), description: String::new() };
        assert_ne!(prompt_screen(&menu), create);
        menu.dialog = MenuDialog::None;
        menu.show_settings = true;
        assert_ne!(prompt_screen(&menu), play_together, "Settings over a page is its own screen");
    }
}
