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
//! answer arrives (a menu field does both through [`text_field_prompt`]):
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
//! Broker rules (each has a test in `text_prompt_tests.rs`):
//! 1. One prompt at a time: a request while one is open is ignored.
//! 2. An answer is delivered at most once, and only to the key that asked.
//! 3. An answer nobody collected (its screen went away) is dropped when the next
//!    prompt is requested. It never fills another field.
//! 4. No dead keyboard: a backend that cannot show a prompt, or reports its prompt
//!    gone (the activity no longer knows it), frees the broker with a cancel to the
//!    key that asked, so the next tap opens a new prompt.
//! 5. A prompt belongs to the screen that asked. Every frame the lobby
//!    (`menu::draw_main_menu`, naming its page or dialog with `menu::prompt_screen`)
//!    or the world (the game loop's touch Chat block) says which screen is on show
//!    ([`enter_screen`]). Another screen is another owner: an answer for the old one,
//!    held or still to come, is dropped, even when a screen of the same kind opens
//!    again. A screen replaced by another of the same kind, which only the lobby's
//!    external link handler does, is told by [`note_dialog_replaced`]: it changes the
//!    screen's name, so the old answer is dropped even when the new dialog holds the
//!    very same text. And a menu field takes an answer only while it still holds the
//!    text its prompt opened with.
//!
//! The broker is a plain struct over a pluggable [`PromptBackend`]. Tests build
//! their own instance; the process-wide one behind the free functions is never
//! touched by a test (the whole suite shares one process).

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
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub fn code(self) -> i32 {
        match self {
            PromptKind::Name => 0,
            PromptKind::Chat => 1,
            PromptKind::Plain => 2,
        }
    }

    /// The kind a menu text field asks for, from its caption: the two name fields
    /// get capitalised words, everything else (seed, room code, npub, a Minecraft
    /// username) plain text.
    pub fn for_field(caption: &str) -> PromptKind {
        match caption {
            "World name" | "Rename skin" => PromptKind::Name,
            _ => PromptKind::Plain,
        }
    }
}

/// The cap of a name field ("World name", "Rename skin"). The desktop `TextEdit`s set
/// none; the handover wire refuses a world name over 128 bytes
/// (`handover/live/wire.rs`), which is 128 characters of the ASCII most names are.
pub const NAME_MAX_CHARS: usize = 128;

/// The cap of the Minecraft username field: a Java username is 1 to 16 characters
/// (`mc_import::validate_username`).
pub const MC_USERNAME_MAX_CHARS: usize = 16;

/// The one shared cap of every other menu field: a room code, a server address, a Keep
/// link, an npub or a seed. The join box
/// has no fixed longest input: a Keep link is 145 characters, alone or inside a pasted
/// message of several hundred (`keeps::route_join` finds it there), and a `ws://` or
/// `wss://` address may carry a path and query of any length. 4096 holds all of them
/// with wide headroom (a prompt must never cut real join text).
pub const TEXT_MAX_CHARS: usize = 4096;

/// The most a menu field's prompt lets you type (UTF-16 units on Android; the web
/// prompt has no limit, as before), from its caption.
pub fn field_max_chars(caption: &str) -> usize {
    match caption {
        "World name" | "Rename skin" => NAME_MAX_CHARS,
        "Minecraft username" => MC_USERNAME_MAX_CHARS,
        _ => TEXT_MAX_CHARS,
    }
}

/// The touch Chat button's key.
pub const CHAT_KEY: &str = "chat";

/// A menu field's key: its caption. Which screen it is on is the broker's owner
/// (rule 5), so the same caption on two screens never shares an answer.
fn field_key(caption: &str) -> String {
    format!("field:{caption}")
}

/// Names a screen for [`enter_screen`]: equal for the same screen, different for
/// another (the lobby hashes its page or its dialog's kind, the world a tag).
pub fn screen_id(screen: impl std::hash::Hash) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    screen.hash(&mut hasher);
    hasher.finish()
}

thread_local! {
    /// How many times a lobby dialog was opened or replaced from outside the menu's own
    /// frame (an external link). Part of the lobby's screen name (`menu::prompt_screen`).
    static DIALOG_EPOCH: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// A dialog was opened or replaced by something other than the player's own taps and
/// typing (`LaunchLinks::offer`), possibly by one of the same kind with the same text.
/// The next frame names a new screen, so a prompt the old dialog asked for is dropped.
/// Never call it for an ordinary per-frame redraw or a field edit.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // the link handler is native-only
pub fn note_dialog_replaced() {
    DIALOG_EPOCH.with(|e| e.set(e.get().wrapping_add(1)));
}

/// The count [`note_dialog_replaced`] keeps; the lobby hashes it into its screen name.
pub fn dialog_epoch() -> u64 {
    DIALOG_EPOCH.with(std::cell::Cell::get)
}

/// What a backend says when asked about the prompt it is showing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptPoll {
    /// Still open, no answer yet.
    #[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
    Waiting,
    /// Closed: `Some(text)` from OK / Done / Enter, `None` from Cancel, Back or a
    /// tap outside.
    #[cfg_attr(not(any(target_arch = "wasm32", target_os = "android", test)), allow(dead_code))]
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

/// Who a prompt belongs to: the screen on show when it was asked for, and which
/// entry into that screen (a screen that closes and opens again is a new owner).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Owner {
    screen: u64,
    entry: u64,
}

/// One prompt asked for: its owner, its key and the text it opened with.
#[derive(Clone, Debug)]
struct Ticket {
    owner: Owner,
    key: String,
    initial: String,
}

/// One prompt at a time, each answer to the key and screen that asked (see the
/// module doc).
pub struct PromptBroker<B> {
    backend: B,
    /// The screen on show now (rule 5).
    owner: Owner,
    /// The prompt that is open.
    open: Option<Ticket>,
    /// An answer not yet collected, with the prompt it answers.
    answer: Option<(Ticket, Option<String>)>,
}

impl<B: PromptBackend> PromptBroker<B> {
    pub fn new(backend: B) -> Self {
        Self { backend, owner: Owner { screen: 0, entry: 0 }, open: None, answer: None }
    }

    /// Say which screen is on show (rule 5). Called every frame; only a change
    /// counts. A new screen is a new owner: an answer held for the old one is
    /// dropped now, and one still to come (its dialog is still up) when it
    /// arrives. Asks the backend nothing.
    pub fn enter_screen(&mut self, screen: u64) {
        if screen == self.owner.screen {
            return;
        }
        self.owner = Owner { screen, entry: self.owner.entry.wrapping_add(1) };
        self.answer = None;
    }

    /// Ask for a prompt for `key`, owned by the screen on show. Returns whether
    /// one was asked for: `false` when a prompt is already open (rule 1).
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
        let ticket = Ticket { owner: self.owner, key: key.to_string(), initial: initial.to_string() };
        if self.backend.show(label, initial, kind, max_chars) {
            self.open = Some(ticket);
        } else {
            // Rule 4: nothing could be shown, so the asker hears "cancelled" now.
            self.answer = Some((ticket, None));
        }
        true
    }

    /// The answer for `key` on the screen on show, once: outer `None` = not ready
    /// (or not this key's), `Some(None)` = cancelled, `Some(Some(text))` = typed.
    pub fn take(&mut self, key: &str) -> Option<Option<String>> {
        self.take_if(key, |_| true)
    }

    /// [`take`](Self::take), and the answer is delivered only if `still_wanted`
    /// agrees; if not, it is dropped (it was for a field that has since changed).
    fn take_if(&mut self, key: &str, still_wanted: impl FnOnce(&Ticket) -> bool) -> Option<Option<String>> {
        self.collect();
        if !self.answer.as_ref().is_some_and(|(t, _)| t.owner == self.owner && t.key == key) {
            return None;
        }
        let (ticket, answer) = self.answer.take()?;
        still_wanted(&ticket).then_some(answer)
    }

    /// A menu text field's turn, every frame it is drawn (`menu.rs`
    /// `menu_text_field_tall`): a tap asks for a prompt with the field's own kind
    /// and cap ([`field_max_chars`], never below what the field already holds, so OK
    /// on an untouched prefill cannot cut it); a typed answer is written into `buf`,
    /// but only while `buf` still holds the text the prompt opened with (rule 5).
    pub fn field(&mut self, caption: &str, buf: &mut String, tapped: bool) {
        let key = field_key(caption);
        if tapped {
            let max_chars = field_max_chars(caption).max(buf.encode_utf16().count());
            self.request(&key, caption, buf, PromptKind::for_field(caption), max_chars);
        }
        if let Some(Some(text)) = self.take_if(&key, |ticket| ticket.initial == *buf) {
            *buf = text;
        }
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
        let Some(ticket) = self.open.take() else { return };
        // Rule 5: an answer for a screen that has gone is dropped as it arrives.
        self.answer = (ticket.owner == self.owner).then_some((ticket, answer));
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

/// Desktop: no prompt (its fields are egui `TextEdit`s, typed on a real keyboard).
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
struct NoPrompt;

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
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

#[cfg(target_os = "android")]
fn platform_backend() -> Box<dyn PromptBackend> {
    Box::new(crate::android_jni::ActivityPrompt)
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn platform_backend() -> Box<dyn PromptBackend> {
    Box::new(NoPrompt)
}

thread_local! {
    /// The process's broker. Only the game thread asks for prompts (menu drawing
    /// and the frame loop), so one per thread is one per game.
    static BROKER: RefCell<PromptBroker<Box<dyn PromptBackend>>> =
        RefCell::new(PromptBroker::new(platform_backend()));
}

/// Run `f` on the process's broker. `try_borrow_mut`: if a browser ever ran a frame
/// inside `window.prompt()`, the inner call is simply skipped rather than panicking.
fn with_broker<T>(f: impl FnOnce(&mut PromptBroker<Box<dyn PromptBackend>>) -> T) -> Option<T> {
    BROKER.with(|b| b.try_borrow_mut().ok().map(|mut b| f(&mut b)))
}

/// Say which screen is on show (rule 5; see [`PromptBroker::enter_screen`]).
pub fn enter_screen(screen: u64) {
    with_broker(|b| b.enter_screen(screen));
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
    with_broker(|b| b.request(key, label, initial, kind, max_chars)).unwrap_or(false)
}

/// Collect `key`'s answer, once: `None` = not ready, `Some(None)` = cancelled,
/// `Some(Some(text))` = typed. Poll every frame the asking screen is drawn.
pub fn take_text_prompt_result(key: &str) -> Option<Option<String>> {
    with_broker(|b| b.take(key)).flatten()
}

/// A menu text field's turn: ask on a tap, fill `buf` when its answer comes (see
/// [`PromptBroker::field`]).
pub fn text_field_prompt(caption: &str, buf: &mut String, tapped: bool) {
    with_broker(|b| b.field(caption, buf, tapped));
}

#[cfg(test)]
#[path = "text_prompt_tests.rs"]
mod tests;
