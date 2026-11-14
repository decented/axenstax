//! The text prompt: how a touch screen types a world name, a seed or a chat line.
//!
//! winit + egui cannot raise a soft keyboard on a touch screen: Android's
//! `NativeActivity` has no text-editor view for the IME to serve, and we don't run
//! eframe's hidden text agent on the web. So a touch text field (`menu.rs`
//! `menu_text_field_tall`) and the touch Chat button (`game_loop.rs`) ask the
//! platform for a native text prompt instead.
//!
//! The ask is ASYNC on every platform. A screen calls [`request_text_prompt`] once
//! (on a tap) and [`take_text_prompt_result`] every frame it is drawn, until the
//! answer arrives:
//!
//! - **Web:** `window.prompt()` at request time. It blocks the page, so the answer is
//!   there on the very next poll (the same frame, since callers poll right after
//!   asking): the same behaviour as before this broker existed.
//! - **Android:** a dark AlertDialog with a one-line EditText, shown by
//!   `AxenstaxActivity.showTextPrompt` over JNI (`android_jni.rs`). The game thread
//!   never waits for it: it asks, then polls a synchronized field on the activity,
//!   and only while a prompt is open.
//! - **Desktop:** no prompt (fields are egui `TextEdit`s there); a request is
//!   answered "cancelled" at once.
//!
//! Broker rules (each has a test below):
//! 1. One prompt at a time: a request while one is open is ignored.
//! 2. An answer is delivered at most once, and only to the key that asked.
//! 3. An answer nobody collected (its screen went away) is dropped when the next
//!    prompt is requested. It never fills another field.
//! 4. No dead keyboard: a backend that cannot show a prompt, or reports its prompt
//!    gone (the activity no longer knows it), frees the broker with a cancel to the
//!    key that asked, so the next tap opens a new prompt.
//!
//! The broker is a plain struct over a pluggable [`PromptBackend`]. Tests build
//! their own instance; the process-wide one behind the two free functions is
//! never touched by a test (the whole suite shares one process).

use std::cell::RefCell;

/// What a prompt is for: it picks the keyboard (Android `inputType`). The web's
/// `window.prompt()` has one kind of keyboard and ignores this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptKind {
    /// A name: each word capitalised.
    Name,
    /// A chat line or a `/command`: no suggestions, no autocorrect, no capitals,
    /// so `/give torch` arrives exactly as typed.
    Chat,
    /// Plain text: a seed, a room code, an npub. No capitals, no suggestions.
    Plain,
}

impl PromptKind {
    /// The number `AxenstaxActivity.showTextPrompt` takes (its `KIND_*` constants).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn code(self) -> i32 {
        match self {
            PromptKind::Name => 0,
            PromptKind::Chat => 1,
            PromptKind::Plain => 2,
        }
    }

    /// The kind a menu text field asks for, from its caption: the two name fields
    /// get capitalised words, everything else (seed, room code, npub, the delete
    /// confirmation, a Minecraft username) plain text.
    pub fn for_field(caption: &str) -> PromptKind {
        match caption {
            "World name" | "Rename skin" => PromptKind::Name,
            _ => PromptKind::Plain,
        }
    }
}

/// The most a menu field's prompt lets you type (UTF-16 units on Android; the web
/// prompt has no limit, as before).
pub const FIELD_MAX_CHARS: usize = 128;

/// The touch Chat button's key.
pub const CHAT_KEY: &str = "chat";

/// What a backend says when asked about the prompt it is showing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptPoll {
    /// Still open, no answer yet.
    #[cfg_attr(not(test), allow(dead_code))]
    Waiting,
    /// Closed: `Some(text)` from OK / Done / Enter, `None` from Cancel, Back or a
    /// tap outside.
    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
    Answered(Option<String>),
    /// The backend no longer knows this prompt (no dialog open, no answer held:
    /// e.g. a new activity instance, or the dialog never got shown). Taken as a
    /// cancel, so the keyboard is never left dead.
    Gone,
}

/// A platform's prompt: show one, then say how it is going.
pub trait PromptBackend {
    /// Show a prompt. `false` = it could not be shown (the request is then
    /// answered "cancelled" at once). Must not block on another thread.
    fn show(&mut self, label: &str, initial: &str, kind: PromptKind, max_chars: usize) -> bool;
    /// How the prompt last shown is going. Called only while one is open.
    fn poll(&mut self) -> PromptPoll;
}

impl<B: PromptBackend + ?Sized> PromptBackend for Box<B> {
    fn show(&mut self, label: &str, initial: &str, kind: PromptKind, max_chars: usize) -> bool {
        (**self).show(label, initial, kind, max_chars)
    }
    fn poll(&mut self) -> PromptPoll {
        (**self).poll()
    }
}

/// One prompt at a time, each answer to the key that asked (see the module doc).
pub struct PromptBroker<B> {
    backend: B,
    /// The key whose prompt is open.
    open: Option<String>,
    /// An answer not yet collected, with the key it belongs to.
    answer: Option<(String, Option<String>)>,
}

impl<B: PromptBackend> PromptBroker<B> {
    pub fn new(backend: B) -> Self {
        Self { backend, open: None, answer: None }
    }

    /// Ask for a prompt for `key`. Returns whether one was asked for: `false`
    /// when a prompt is already open (rule 1).
    pub fn request(
        &mut self,
        key: &str,
        label: &str,
        initial: &str,
        kind: PromptKind,
        max_chars: usize,
    ) -> bool {
        // A prompt that closed since anyone last polled frees the broker first.
        self.collect();
        if self.open.is_some() {
            return false;
        }
        // Rule 3: an answer still here is stale (its screen never collected it).
        self.answer = None;
        if self.backend.show(label, initial, kind, max_chars) {
            self.open = Some(key.to_string());
        } else {
            // Rule 4: nothing could be shown, so the asker hears "cancelled" now.
            self.answer = Some((key.to_string(), None));
        }
        true
    }

    /// The answer for `key`, once: outer `None` = not ready (or not this key's),
    /// `Some(None)` = cancelled, `Some(Some(text))` = typed.
    pub fn take(&mut self, key: &str) -> Option<Option<String>> {
        self.collect();
        if self.answer.as_ref().is_some_and(|(k, _)| k == key) {
            return self.answer.take().map(|(_, answer)| answer);
        }
        None
    }

    /// Move a closed prompt's answer (or a gone prompt's cancel) out of the
    /// backend. The backend is asked only while a prompt is open, so an idle
    /// broker costs nothing per frame (no JNI call on Android).
    fn collect(&mut self) {
        if self.open.is_none() {
            return;
        }
        let answer = match self.backend.poll() {
            PromptPoll::Waiting => return,
            PromptPoll::Answered(answer) => answer,
            PromptPoll::Gone => None,
        };
        self.answer = self.open.take().map(|key| (key, answer));
    }

    /// Whether a prompt is open.
    #[cfg(test)]
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }
}

/// The web: `window.prompt()`, answered on the spot.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct WebPrompt {
    answer: Option<Option<String>>,
}

#[cfg(target_arch = "wasm32")]
impl PromptBackend for WebPrompt {
    fn show(&mut self, label: &str, initial: &str, _kind: PromptKind, _max_chars: usize) -> bool {
        // The one reliable way to summon the on-screen keyboard on a tablet or a
        // Chromebook with no hardware keyboard. It blocks the page until answered.
        let Some(window) = web_sys::window() else { return false };
        self.answer = Some(window.prompt_with_message_and_default(label, initial).ok().flatten());
        true
    }
    fn poll(&mut self) -> PromptPoll {
        match self.answer.take() {
            Some(answer) => PromptPoll::Answered(answer),
            None => PromptPoll::Gone,
        }
    }
}

/// Desktop (and Android until its dialog lands): no prompt; fields are egui
/// `TextEdit`s, typed on a real keyboard.
#[cfg(not(target_arch = "wasm32"))]
struct NoPrompt;

#[cfg(not(target_arch = "wasm32"))]
impl PromptBackend for NoPrompt {
    fn show(&mut self, _label: &str, _initial: &str, _kind: PromptKind, _max_chars: usize) -> bool {
        false
    }
    fn poll(&mut self) -> PromptPoll {
        PromptPoll::Gone
    }
}

#[cfg(target_arch = "wasm32")]
fn platform_backend() -> Box<dyn PromptBackend> {
    Box::new(WebPrompt::default())
}

#[cfg(not(target_arch = "wasm32"))]
fn platform_backend() -> Box<dyn PromptBackend> {
    Box::new(NoPrompt)
}

thread_local! {
    /// The process's broker. Only the game thread asks for prompts (menu drawing
    /// and the frame loop), so one per thread is one per game.
    static BROKER: RefCell<PromptBroker<Box<dyn PromptBackend>>> =
        RefCell::new(PromptBroker::new(platform_backend()));
}

/// Ask the platform for a text prompt for `key` (see the module doc). Ignored
/// (returns `false`) while another prompt is open.
pub fn request_text_prompt(
    key: &str,
    label: &str,
    initial: &str,
    kind: PromptKind,
    max_chars: usize,
) -> bool {
    // `try_borrow_mut`: if a browser ever ran a frame inside `window.prompt()`,
    // the inner call is simply ignored rather than panicking.
    BROKER.with(|b| {
        b.try_borrow_mut()
            .map(|mut b| b.request(key, label, initial, kind, max_chars))
            .unwrap_or(false)
    })
}

/// Collect `key`'s answer, once: `None` = not ready, `Some(None)` = cancelled,
/// `Some(Some(text))` = typed. Poll every frame the asking screen is drawn.
pub fn take_text_prompt_result(key: &str) -> Option<Option<String>> {
    BROKER.with(|b| b.try_borrow_mut().ok().and_then(|mut b| b.take(key)))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        b.request(key, key, "", PromptKind::Plain, FIELD_MAX_CHARS)
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
        assert!(b.request("name", "World name", "My World", PromptKind::Name, FIELD_MAX_CHARS));
        assert_eq!(b.take("name"), Some(Some("My World!".into())));
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
}
