//! "Your relays" — the native relay manager (Spec 04 §1.9).
//!
//! One list, `GraphicsSettings.online_relays`, is everything the app connects
//! to on the player's behalf: the sign-in QR, online-play setup, Server Card
//! discovery, Signet contacts pairing and the release feed. This module is the
//! one editor for it, shown in three places — the settings panel's "Relays"
//! section, and behind a "Relays" button on the sign-in dialog and in the
//! lobby — so a household can pick relays it trusts before it ever connects.
//!
//! The list logic ([`validate_new`], [`add`], [`remove`], [`reset`]) is pure
//! and unit-tested; the egui half only calls it. Nothing here re-sanitises on a
//! keystroke (P6 audit): the add field is a free-typed draft, validated only
//! when the player presses Add.
//!
//! NATIVE-ONLY: the web taster connects to no relay on the player's behalf.
#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::graphics_settings::{default_online_relays, GraphicsSettings, MAX_RELAYS};

pub const HEADING: &str = "Your relays";
pub const LEAD: &str = "The app uses these to sign in, find friends, pair contacts and check for \
                        updates. Your game itself never runs through them.";
/// Added under [`LEAD`] only while alpha-tester feedback is switched on.
pub const LEAD_FEEDBACK: &str = " Bug reports go to the project's own inbox relays.";
/// How long "Check" waits for each relay's websocket to open.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(5);

const OK_GREEN: egui::Color32 = egui::Color32::from_rgb(150, 220, 150);
const BAD_RED: egui::Color32 = egui::Color32::from_rgb(230, 150, 150);
const DIM: egui::Color32 = egui::Color32::from_rgb(150, 150, 160);

/// Why a typed relay was not added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddError {
    Empty,
    NotWss,
    Duplicate,
    Full,
}

impl AddError {
    /// The short line shown under the add field.
    pub fn message(self) -> &'static str {
        match self {
            AddError::Empty => "Type a relay address first.",
            AddError::NotWss => "A relay address starts with wss://",
            AddError::Duplicate => "That relay is already on your list.",
            AddError::Full => "You can have up to 8 relays. Remove one first.",
        }
    }
}

/// Two relay addresses name the same relay (case and a trailing `/` aside).
fn same_relay(a: &str, b: &str) -> bool {
    a.trim_end_matches('/').eq_ignore_ascii_case(b.trim_end_matches('/'))
}

/// Check a typed relay against the current list. `Ok` carries the trimmed
/// address to store. Same rules as `graphics_settings::sanitise_relays`
/// (wss-only, no duplicates, at most [`MAX_RELAYS`]), but with a reason.
pub fn validate_new(list: &[String], input: &str) -> Result<String, AddError> {
    let r = input.trim();
    if r.is_empty() {
        return Err(AddError::Empty);
    }
    let host = r.strip_prefix("wss://").ok_or(AddError::NotWss)?;
    if host.is_empty() || r.chars().any(char::is_whitespace) {
        return Err(AddError::NotWss);
    }
    // The signer's own parser: one address it can't read fails the whole
    // nostrconnect URI, so refuse it here rather than at sign-in.
    if nostr::RelayUrl::parse(r).is_err() {
        return Err(AddError::NotWss);
    }
    if list.iter().any(|x| same_relay(x, r)) {
        return Err(AddError::Duplicate);
    }
    if list.len() >= MAX_RELAYS {
        return Err(AddError::Full);
    }
    Ok(r.to_string())
}

/// Append a typed relay if it passes [`validate_new`].
pub fn add(list: &mut Vec<String>, input: &str) -> Result<(), AddError> {
    let r = validate_new(list, input)?;
    list.push(r);
    Ok(())
}

/// Whether a row's remove button is live: the list may never become empty.
pub fn can_remove(list: &[String]) -> bool {
    list.len() > 1
}

/// Remove the relay at `idx`. Refuses (returns `false`) on the last remaining
/// relay or an out-of-range index.
pub fn remove(list: &mut Vec<String>, idx: usize) -> bool {
    if !can_remove(list) || idx >= list.len() {
        return false;
    }
    list.remove(idx);
    true
}

/// The shipped list (`server_resolve::PUBLIC_DEFAULT_RELAYS`).
pub fn reset() -> Vec<String> {
    default_online_relays()
}

/// One relay's "Check" result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckState {
    Checking,
    Reachable,
    Unreachable,
}

#[derive(Default)]
struct Checks {
    /// Bumped on every "Check" so a slow earlier run can't overwrite a newer one.
    generation: u64,
    by_relay: HashMap<String, CheckState>,
}

/// UI state for one relay-manager instance (the settings panel and the lobby
/// each own one). The relay list itself lives in `GraphicsSettings`.
#[derive(Default)]
pub struct RelaysUiState {
    /// The add field's free-typed draft.
    pub input: String,
    pub error: Option<AddError>,
    checks: Arc<Mutex<Checks>>,
}

impl RelaysUiState {
    fn check_state(&self, relay: &str) -> Option<CheckState> {
        self.checks.lock().ok().and_then(|c| c.by_relay.get(relay).copied())
    }

    fn checking(&self) -> bool {
        self.checks
            .lock()
            .map(|c| c.by_relay.values().any(|s| *s == CheckState::Checking))
            .unwrap_or(false)
    }

    /// Test every relay off the main thread: a websocket connect within
    /// [`CHECK_TIMEOUT`]. Results land in the shared map as they arrive.
    pub fn start_check(&mut self, relays: &[String]) {
        let generation = {
            let Ok(mut c) = self.checks.lock() else { return };
            c.generation += 1;
            c.by_relay = relays.iter().map(|r| (r.clone(), CheckState::Checking)).collect();
            c.generation
        };
        let checks = self.checks.clone();
        let owned = relays.to_vec();
        let spawned = std::thread::Builder::new().name("relay-check".into()).spawn(move || {
            let relays = owned;
            let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
                record_all(&checks, generation, &relays, CheckState::Unreachable);
                return;
            };
            rt.block_on(futures_util::future::join_all(relays.iter().map(|url| {
                let checks = checks.clone();
                async move {
                    let ok = matches!(
                        tokio::time::timeout(
                            CHECK_TIMEOUT,
                            tokio_tungstenite::connect_async(url.as_str())
                        )
                        .await,
                        Ok(Ok(_))
                    );
                    let state = if ok { CheckState::Reachable } else { CheckState::Unreachable };
                    record(&checks, generation, url, state);
                }
            })));
        });
        if spawned.is_err() {
            record_all(&self.checks, generation, relays, CheckState::Unreachable);
        }
    }
}

fn record(checks: &Mutex<Checks>, generation: u64, relay: &str, state: CheckState) {
    if let Ok(mut c) = checks.lock()
        && c.generation == generation
    {
        c.by_relay.insert(relay.to_string(), state);
    }
}

fn record_all(checks: &Mutex<Checks>, generation: u64, relays: &[String], state: CheckState) {
    for r in relays {
        record(checks, generation, r, state);
    }
}

/// Draw the manager inline. Returns `true` when the list changed (already
/// saved through `gfx.save()`).
pub fn draw(ui: &mut egui::Ui, state: &mut RelaysUiState, gfx: &mut GraphicsSettings) -> bool {
    let mut changed = false;
    ui.label(egui::RichText::new(HEADING).size(15.0).strong());
    let lead = if crate::native_mailbox::feedback_enabled(gfx) {
        format!("{LEAD}{LEAD_FEEDBACK}")
    } else {
        LEAD.to_string()
    };
    ui.label(egui::RichText::new(lead).size(11.0).color(DIM));
    ui.add_space(4.0);

    let removable = can_remove(&gfx.online_relays);
    let mut remove_idx = None;
    for (i, relay) in gfx.online_relays.iter().enumerate() {
        ui.horizontal(|ui| {
            let x = ui
                .add_enabled(removable, egui::Button::new("×").small())
                .on_hover_text("Remove this relay")
                .on_disabled_hover_text("Keep at least one relay");
            if x.clicked() {
                remove_idx = Some(i);
            }
            match state.check_state(relay) {
                Some(CheckState::Checking) => {
                    ui.spinner();
                }
                Some(CheckState::Reachable) => {
                    ui.colored_label(OK_GREEN, "✓");
                }
                Some(CheckState::Unreachable) => {
                    ui.colored_label(BAD_RED, "✗");
                }
                None => {}
            }
            ui.label(egui::RichText::new(relay).size(12.0));
        });
    }
    if let Some(i) = remove_idx
        && remove(&mut gfx.online_relays, i)
    {
        changed = true;
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(&mut state.input)
                .hint_text("wss://relay.example.com")
                .desired_width(220.0),
        );
        if field.changed() {
            state.error = None;
        }
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button("Add").clicked() || enter {
            match add(&mut gfx.online_relays, &state.input) {
                Ok(()) => {
                    state.input.clear();
                    state.error = None;
                    changed = true;
                }
                Err(e) => state.error = Some(e),
            }
        }
    });
    if let Some(e) = state.error {
        ui.label(egui::RichText::new(e.message()).size(11.0).color(BAD_RED));
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let defaults = reset();
        if ui
            .add_enabled(gfx.online_relays != defaults, egui::Button::new("Reset to defaults"))
            .clicked()
        {
            gfx.online_relays = defaults;
            state.error = None;
            changed = true;
        }
        let busy = state.checking();
        if ui
            .add_enabled(!busy, egui::Button::new("Check"))
            .on_hover_text("Try to reach each relay (up to 5 seconds)")
            .clicked()
        {
            state.start_check(&gfx.online_relays);
        }
        if busy {
            ui.ctx().request_repaint();
        }
    });

    if changed {
        gfx.save();
    }
    changed
}

/// What the modal window reports back to its caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowResult {
    /// The list changed (already saved).
    pub changed: bool,
    /// The player pressed Done.
    pub close: bool,
}

/// The manager as a floating window, for the sign-in dialog and the lobby.
pub fn draw_window(
    ctx: &egui::Context,
    state: &mut RelaysUiState,
    gfx: &mut GraphicsSettings,
) -> WindowResult {
    let mut out = WindowResult::default();
    egui::Window::new("Relays")
        .collapsible(false)
        .resizable(false)
        // A landscape phone is ~411 points tall: let the list scroll inside
        // the screen-constrained window rather than clip "Done" off the bottom.
        // Android only, so desktop/web windows size exactly as before.
        .vscroll(cfg!(target_os = "android"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_max_width(380.0);
            out.changed = draw(ui, state, gfx);
            ui.add_space(10.0);
            if ui.button("Done").clicked() {
                out.close = true;
            }
        });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn add_rejects_a_wss_address_the_signer_cannot_parse() {
        // One unparseable relay in the first three fails the whole
        // nostrconnect URI (`RelayUrl::parse` in the nip46 client), so the
        // manager must refuse it up front.
        let mut l = list(&["wss://a.example"]);
        assert_eq!(add(&mut l, "wss://bad host"), Err(AddError::NotWss));
        assert_eq!(add(&mut l, "wss://[::1"), Err(AddError::NotWss));
        assert_eq!(add(&mut l, "wss://exa mple.com"), Err(AddError::NotWss));
        assert_eq!(l, list(&["wss://a.example"]));
    }

    #[test]
    fn add_accepts_a_new_wss_relay_trimmed() {
        let mut l = list(&["wss://a.example"]);
        assert_eq!(add(&mut l, "  wss://b.example  "), Ok(()));
        assert_eq!(l, list(&["wss://a.example", "wss://b.example"]));
    }

    #[test]
    fn add_rejects_empty_non_wss_and_spaces() {
        let mut l = list(&["wss://a.example"]);
        assert_eq!(add(&mut l, "   "), Err(AddError::Empty));
        assert_eq!(add(&mut l, "ws://b.example"), Err(AddError::NotWss));
        assert_eq!(add(&mut l, "https://b.example"), Err(AddError::NotWss));
        assert_eq!(add(&mut l, "wss://"), Err(AddError::NotWss));
        assert_eq!(add(&mut l, "wss://b .example"), Err(AddError::NotWss));
        assert_eq!(l.len(), 1, "nothing added on error");
    }

    #[test]
    fn add_rejects_a_duplicate_ignoring_case_and_trailing_slash() {
        let mut l = list(&["wss://a.example"]);
        assert_eq!(add(&mut l, "wss://A.example/"), Err(AddError::Duplicate));
    }

    #[test]
    fn add_rejects_a_ninth_relay() {
        let mut l: Vec<String> = (0..MAX_RELAYS).map(|i| format!("wss://r{i}.example")).collect();
        assert_eq!(add(&mut l, "wss://one-more.example"), Err(AddError::Full));
        assert_eq!(l.len(), MAX_RELAYS);
    }

    #[test]
    fn remove_takes_the_named_row() {
        let mut l = list(&["wss://a.example", "wss://b.example"]);
        assert!(remove(&mut l, 0));
        assert_eq!(l, list(&["wss://b.example"]));
    }

    #[test]
    fn remove_refuses_the_last_relay() {
        let mut l = list(&["wss://only.example"]);
        assert!(!can_remove(&l));
        assert!(!remove(&mut l, 0));
        assert_eq!(l, list(&["wss://only.example"]));
    }

    #[test]
    fn remove_refuses_an_out_of_range_index() {
        let mut l = list(&["wss://a.example", "wss://b.example"]);
        assert!(!remove(&mut l, 5));
        assert_eq!(l.len(), 2);
    }

    #[test]
    fn reset_is_the_public_default_list() {
        assert_eq!(reset(), crate::server_resolve::public_default_relays());
        assert!(reset().iter().all(|r| !r.contains("trotters")));
    }

    #[test]
    fn every_error_has_a_short_message() {
        for e in [AddError::Empty, AddError::NotWss, AddError::Duplicate, AddError::Full] {
            assert!(!e.message().is_empty() && e.message().len() < 60, "{e:?}");
        }
    }

    #[test]
    fn a_stale_check_run_cannot_overwrite_a_newer_one() {
        let checks = Mutex::new(Checks { generation: 2, by_relay: HashMap::new() });
        record(&checks, 1, "wss://a.example", CheckState::Reachable);
        assert!(checks.lock().unwrap().by_relay.is_empty());
        record(&checks, 2, "wss://a.example", CheckState::Unreachable);
        assert_eq!(
            checks.lock().unwrap().by_relay.get("wss://a.example"),
            Some(&CheckState::Unreachable)
        );
    }
}
