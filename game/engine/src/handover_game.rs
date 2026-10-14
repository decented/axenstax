//! GameState callers of the live world handover (Spec 04 "World handover").
//!
//! - The host action `Hand this world to …`: offered only for a present,
//!   authenticated seat (`HostedServer::handover_seat`), re-read at the click;
//!   the session binds that exact slot, attach generation and verified key.
//! - The recipient's explicit consent while joined: the joined `RemoteClient`
//!   routes Handover frames here; a Propose is checked against the host this
//!   client verified at join (`joined_host`) before anything is shown.
//! - One control pump per frame, in every mode (`poll_handover`, next to
//!   `poll_online`), never from the game tick. The recipient session keeps its
//!   connection across leaving the joined world; a source session is aborted
//!   before its world is torn down (everything here is pre-retirement).
//! - The play freeze: while this machine's world is frozen, or the host it
//!   joined paused play, or a session holds its connection, no game tick, no
//!   frame-level gameplay input, no input sends, no commands and no saves run.
use crate::handover::{
    Id,
    context::RootContext,
    live::{
        capture, copy,
        eligibility::{Refusal, Seat},
        recipient::{JoinedHost, RecipientConsent, RecipientIdentity, RecipientSession, RecipientStage},
        source::{Capture, HostAction, Live, SourceConfig, SourceSession, SourceStage},
        wire::{DeclineReason, Frame, root_lineage},
    },
    phase::Authority,
};
use std::path::PathBuf;
use web_time::{Duration, Instant};

/// How long a certificate stays valid, clipped to the invitation's expiry.
pub(crate) const CERTIFICATE_SECS: u64 = 3600;
/// L7: the consent buttons arm this long after the window first shows, so a
/// click already under way (a chest open mid-play) can't land on Accept.
pub(crate) const CONSENT_ARM: Duration = Duration::from_millis(750);

/// The one handover this machine is part of.
pub(crate) enum Handover {
    Source { session: Box<SourceSession>, world: String },
    Recipient(Box<RecipientSession>),
}

/// What the source needs from this machine besides the world: the runtime
/// key and the live invitation it hosts under (`OnlineHost`), and where the
/// packed copy is staged.
pub(crate) struct SourceEnv {
    pub runtime: nostr::Keys,
    pub root: RootContext,
    pub scratch: PathBuf,
    /// H1: the live invitation's bearer (`OnlineHost::live_bearer`).
    pub bearer: crate::handover::live::bearer::Bearer,
    /// This runtime's player attestation (the one online play uses).
    pub attestation: nostr::Event,
    pub continuation: Option<crate::handover::live::continuation::Resolved>,
}

/// What a recipient needs: its runtime key and that key's player
/// attestation, and where an inactive copy and its markers are kept.
pub(crate) struct RecipientEnv {
    pub identity: RecipientIdentity,
    pub imports: PathBuf,
}

/// L5: the joined session's router lease, held by its recipient session.
impl crate::handover::live::recipient::HeldLease for crate::online_prep::LeaseKeeper {
    fn renew(&mut self) {
        self.tick(Instant::now());
    }
    fn give_back(self: Box<Self>) {
        self.release();
    }
    fn give_back_within(self: Box<Self>, grace: std::time::Duration) {
        self.release_within(grace);
    }
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

/// Test stand-ins for the signer and the data directory.
#[cfg(test)]
pub(crate) struct Fixture {
    pub signed_in: Option<[u8; 32]>,
    pub recipient: Option<Box<dyn Fn() -> Result<RecipientEnv, String>>>,
}

/// A HUD click a test injects in place of an egui click (the harness has no
/// pointer): consumed only by the branch that would draw that dialog.
#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) enum TestHudClick {
    Chest(crate::container_window::ContainerClick),
    Bounty(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UiAction {
    Accept,
    Decline,
    Cancel,
}

fn random_id() -> Id {
    let mut id = [0; 16];
    getrandom::fill(&mut id).expect("randomness unavailable");
    id
}

/// `<data dir>/handover/<leaf>`, created.
fn handover_dir(leaf: &str) -> Result<PathBuf, String> {
    let dir = crate::data_dir::data_root().join("handover").join(leaf);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// The production recipient environment: this install's runtime key and its
/// attestation (the one online play uses), and the data directory.
fn recipient_env() -> Result<RecipientEnv, String> {
    let identity = crate::runtime_identity::RuntimeIdentity::load()?.ok_or("no runtime identity")?;
    let attestation = identity.attestation().cloned().ok_or("no attestation")?;
    // CP10: the recipient's records live in each import's own working
    // world `handover/`, not in a shared markers folder.
    Ok(RecipientEnv {
        identity: RecipientIdentity { runtime: identity.keys().clone(), attestation },
        imports: handover_dir("imports")?,
    })
}

/// CP11: the world list's status lines for imports still awaiting their
/// Committed (read-only: never creates `<data>/handover/imports`). CP13:
/// plus one line for each handover record the install's ledger doesn't list.
pub(crate) fn awaiting_import_statuses() -> Vec<String> {
    let mut lines = crate::handover::live::resumer::awaiting_statuses(
        &crate::save::worlds_root(),
        &crate::data_dir::data_root().join("handover").join("imports"),
        &crate::handover::ledger::Ledger::install(),
    );
    // CP13b (D2): e.g. "This world may now exist in two places."
    lines.extend(crate::handover::live::resumer::Resumer::global().notices());
    // CP18: an Activated import the last promotion couldn't move in.
    lines.extend(PROMOTE_LINES.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned());
    lines
}

/// CP18: the last promotion's lines (`promote_activated_imports`).
static PROMOTE_LINES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// CP18: move every Activated import into the worlds folder, so it lists
/// as an ordinary world (`handover::live::promote`). Main thread only: at
/// start-up, on lobby entry and every lobby refresh, and (CP20) from the
/// lobby's per-frame `MenuState::poll_handovers` once an import activates.
/// Never creates `<data>/handover/imports`.
pub(crate) fn promote_activated_imports() -> crate::handover::live::promote::Promoted {
    let imports = crate::data_dir::data_root().join("handover").join("imports");
    let mut promoted = crate::handover::live::promote::promote_activated_imports(&imports, &crate::save::worlds_root());
    *PROMOTE_LINES.lock().unwrap_or_else(|e| e.into_inner()) = std::mem::take(&mut promoted.lines);
    promoted
}

/// P3: discard every marked inert import under `<data>/handover/imports`
/// except `live`'s (bounded; at startup and before a new recipient session).
pub(crate) fn sweep_inert_imports(live: Option<Id>) {
    let Ok(imports) = handover_dir("imports") else { return };
    match crate::handover::live::inert::sweep(&imports, live) {
        Ok(0) => {}
        Ok(n) => log::info!("handover: discarded {n} inert import(s)"),
        Err(e) => log::warn!("handover: inert import sweep: {e}"),
    }
    // Review LOW-7: at start-up no source session runs either.
    if live.is_none() {
        sweep_pack_scratch();
    }
}

/// Review LOW-7: orphaned `.handover-pack-*` scratch (no source session).
fn sweep_pack_scratch() {
    let Ok(scratch) = handover_dir("scratch") else { return };
    if let Ok(n @ 1..) = crate::handover::live::inert::sweep_pack_scratch(&scratch) {
        log::info!("handover: removed {n} orphaned pack folder(s)");
    }
}

/// The handover window: the consent screen, the status of a running
/// handover with its Cancel, or the paused banner. Returns the click.
pub(crate) fn draw(ctx: &egui::Context, handover: Option<&Handover>, host_paused: bool, interactable: bool, armed: bool) -> Option<UiAction> {
    let mut action = None;
    let consent = matches!(handover, Some(Handover::Recipient(s)) if s.stage() == RecipientStage::AwaitingConsent);
    let (title, lines, buttons): (&str, Vec<String>, &[(&str, UiAction)]) = match handover {
        Some(Handover::Recipient(s)) => {
            let p = s.proposal();
            match s.stage() {
                RecipientStage::AwaitingConsent => (
                    copy::CONSENT_TITLE,
                    copy::consent_lines(&p.world_name, &p.source_npub).to_vec(),
                    &[(copy::ACCEPT, UiAction::Accept), (copy::DECLINE, UiAction::Decline)],
                ),
                RecipientStage::Ready => {
                    ("World handover", vec![copy::recipient_stored(&p.world_name)], &[(copy::CANCEL, UiAction::Cancel)])
                }
                RecipientStage::Prepared => ("World handover", vec![copy::recipient_finishing(&p.world_name)], &[]),
                RecipientStage::Aborted(_) | RecipientStage::Quarantined(_) | RecipientStage::Activated => return None,
                _ => (
                    "World handover",
                    vec![copy::recipient_copying(&p.world_name, &p.source_npub)],
                    &[(copy::CANCEL, UiAction::Cancel)],
                ),
            }
        }
        Some(Handover::Source { session, world }) => {
            let who = session.recipient_npub();
            let cancel: &[(&str, UiAction)] = &[(copy::CANCEL, UiAction::Cancel)];
            let (line, buttons) = match session.stage() {
                SourceStage::Proposed => (copy::source_asking(world, &who), cancel),
                SourceStage::ReadyVerified => (copy::source_stored(world, &who), cancel),
                SourceStage::Publishing | SourceStage::Prepared => (copy::source_finishing(world, &who), cancel),
                // Past retirement nothing resumes: no Cancel.
                SourceStage::Retired => {
                    let mut line = copy::source_retired(world, &who);
                    line.push_str(if session.retained_folder().is_some() { " Your separate retained copy is in the lobby." } else { " Your retained copy is stored safely and awaits a move retry from the lobby." });
                    (line, &[][..])
                },
                SourceStage::RetirementUncertain => (copy::RETIREMENT_UNCERTAIN.to_string(), &[][..]),
                SourceStage::Aborted(_) => return None,
                _ => (copy::source_copying(world, &who), cancel),
            };
            ("World handover", vec![line], buttons)
        }
        None if host_paused => ("World handover", vec![copy::PAUSED_BANNER.to_string()], &[]),
        None => return None,
    };
    egui::Window::new(title)
        .id(egui::Id::new("world-handover"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_TOP, [0.0, 48.0])
        .interactable(interactable)
        .show(ctx, |ui| {
            ui.set_max_width(420.0);
            for l in &lines {
                ui.label(l);
                ui.add_space(4.0);
            }
            ui.horizontal(|ui| {
                for (label, a) in buttons {
                    if ui.add_enabled(armed || !consent, egui::Button::new(*label)).clicked() {
                        action = Some(*a);
                    }
                }
            });
        });
    action
}

impl crate::GameState {
    /// This machine's world is frozen for a handover.
    pub(crate) fn handover_freezes_world(&self) -> bool {
        self.hosted_server.as_ref().is_some_and(|hs| hs.handover_frozen())
    }

    /// Play waits: this world is frozen, the host this client joined paused
    /// play, or a handover session holds this client's connection.
    pub(crate) fn handover_holds_play(&self) -> bool {
        self.handover_freezes_world()
            || self.host_paused
            || self.remote_client.as_ref().is_some_and(|rc| rc.is_parked())
            || self.remote_client.as_ref().is_some_and(|rc| rc.personal_inbox.as_ref().is_some_and(|inbox| !inbox.initialised()))
    }

    /// M-A: the one gate for pause-menu world settings while play is held
    /// for a handover. A world-changing click is dropped (with a notice)
    /// and `-1` (no click) returned; anything else passes unchanged.
    pub(crate) fn handover_gate_pause_click(&mut self, clicked: i32) -> i32 {
        use crate::menu::*;
        let world_setting = matches!(
            clicked,
            PAUSE_STASH_TOGGLED | PAUSE_DIFFICULTY_CHANGED | PAUSE_SWITCH_CREATIVE | PAUSE_TRY_AGAIN
        ) || clicked >= PAUSE_PLAYER_LEAVE_BASE;
        if world_setting && self.handover_holds_play() {
            self.handover_toast(copy::SETTINGS_PAUSED);
            return -1;
        }
        clicked
    }

    /// Review LOW-5: the one gate for the world- or inventory-changing
    /// actions that sit outside the HUD's frozen branch: the trial and
    /// scenario "Try again" (teleport, lanterns, provisioning, a direct
    /// world_meta write) and closing the crafting screen (it moves grid and
    /// cursor items into the inventory). While play is held the action is
    /// dropped with a notice; `true` = blocked.
    pub(crate) fn handover_blocks_world_action(&mut self) -> bool {
        if self.handover_holds_play() {
            self.handover_toast(copy::SETTINGS_PAUSED);
            return true;
        }
        false
    }


    /// The identity signed in now (re-read at every boundary).
    pub(crate) fn handover_signed_in(&self) -> Option<[u8; 32]> {
        #[cfg(test)]
        if let Some(f) = &self.handover_fixture {
            return f.signed_in;
        }
        Self::owner_persona_bytes()
    }

    fn handover_live(&self) -> Live {
        Live { signed_in: self.handover_signed_in(), local_seats: self.players.len() }
    }

    fn handover_toast(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        if let crate::GameMode::Menu(menu) = &mut self.mode {
            menu.notice = Some(msg);
        } else {
            self.toast = Some((msg, Instant::now() + Duration::from_secs(8)));
        }
    }

    /// Present, authenticated players this host may hand the world to, by
    /// seat: `(slot, verified key)`. Never a name typed in, never a seat
    /// that has gone.
    /// Each row carries the seat's attach generation and key as drawn (L6):
    /// the click is checked against them, not just the slot.
    pub(crate) fn handover_candidates(&self) -> Vec<(usize, u32, [u8; 32])> {
        let Some(hs) = &self.hosted_server else { return Vec::new() };
        if self.handover.is_some() {
            return Vec::new();
        }
        (0..hs.server.players.len())
            .filter_map(|slot| hs.handover_seat(slot).map(|(generation, key)| (slot, generation, key)))
            .collect()
    }

    /// L7: the consent window has been up long enough for its buttons.
    pub(crate) fn handover_consent_armed(&self) -> bool {
        self.handover_consent_shown.is_some_and(|t| t.elapsed() >= CONSENT_ARM)
    }

    /// The host action from the Online panel. Only an online-hosted world
    /// has an invitation to bind the handover to.
    pub(crate) fn begin_handover(&mut self, drawn: (usize, u32, [u8; 32])) {
        if let Some(slot) = self.players.first() && let Err(r) = crate::handover::live::eligibility::local_progress(slot, self.tick_counter) {
            self.handover_toast(r.0); return;
        }
        let now = crate::game_loop::wardrobe_now();
        let env = self.online_host.as_ref().map(|host| (host.runtime_keys().clone(), host.root_context(now).ok(), host.live_bearer(now), host.continuation().cloned()));
        let Some((runtime, root, bearer, continuation)) = env else {
            self.handover_toast(copy::NEEDS_ONLINE);
            return;
        };
        // H1: the live invitation capability, or a typed refusal.
        let (Some(root), Some(bearer)) = (root, bearer) else {
            self.handover_toast(copy::NO_LIVE_INVITE);
            return;
        };
        let attestation = crate::runtime_identity::RuntimeIdentity::load()
            .ok()
            .flatten()
            .filter(|id| id.keys().public_key() == runtime.public_key())
            .and_then(|id| id.attestation().cloned());
        let Some(attestation) = attestation else {
            self.handover_toast(copy::NEEDS_ONLINE);
            return;
        };
        let scratch = match handover_dir("scratch") {
            Ok(d) => d,
            Err(e) => {
                self.handover_toast(format!("The handover couldn't start: {e}"));
                return;
            }
        };
        let bearer = crate::handover::live::bearer::Bearer::new(bearer);
        if let Err(e) = self.begin_handover_with(drawn, SourceEnv { runtime, root, scratch, bearer, attestation, continuation }, now) {
            self.handover_toast(e);
        }
    }

    /// Start a handover to the authenticated player in `slot`: re-read its
    /// seat now, derive lineage and authority from the invitation, propose.
    pub(crate) fn begin_handover_with(&mut self, drawn: (usize, u32, [u8; 32]), env: SourceEnv, now: u64) -> Result<(), String> {
        let slot = drawn.0;
        if self.handover.is_some() {
            return Err(copy::BUSY.into());
        }
        let live = self.handover_live();
        let source_key = live.signed_in.ok_or("Sign in before handing this world over.")?;
        if let Some(slot) = self.players.first() {
            crate::handover::live::eligibility::local_progress(slot, self.tick_counter).map_err(|r| r.0.to_string())?;
            let context = crate::handover::personal::RecordContext { participant: source_key, lineage: root_lineage(&env.root), generation: 1, transfer: [1; 16] };
            let seed = capture::local0_sidecar(slot, [0; 16]).map_err(|r| r.0.to_string())?;
            let progress = capture::local0_progress(slot, context).map_err(|r| r.0.to_string())?;
            crate::handover::live::personal_adapter::preflight(&seed, &progress, context).map_err(|r| r.0.to_string())?;
        }
        let meta = crate::save::load_world_meta(&self.world_name);
        let world = if meta.display_name.trim().is_empty() { self.world_name.clone() } else { meta.display_name };
        let Some(hs) = self.hosted_server.as_mut() else { return Err(copy::NEEDS_ONLINE.into()) };
        let (generation, key) = hs.handover_seat(slot).ok_or("That player is no longer signed in to this world.")?;
        if (generation, key) != (drawn.1, drawn.2) {
            return Err(copy::SEAT_CHANGED.into());
        }
        // The lineage is derived from the invitation (the recipient derives
        // the same from the host it verified); the authority is the root's.
        let lineage = root_lineage(&env.root);
        let world_dir = crate::save::world_dir(&self.world_name);
        let installed_proof = crate::handover::live::personal_adapter::proof(&world_dir)?;
        let (authority, checkpoint_generation, validity) = if let Some(resolved) = &env.continuation {
            if resolved.root != env.root || resolved.bearer != env.bearer { return Err("The original invitation capability changed.".into()); }
            let proof = installed_proof.as_ref().ok_or("The continued world has no installed authority.")?;
            let store = crate::handover::durable::MarkerStore::open_existing(crate::handover::live::authority::dir(&world_dir)).map_err(|e| e.to_string())?;
            let installed = crate::handover::phase::Installed::read(&store, proof.core.transfer).map_err(|e| e.to_string())?.ok_or("The installed authority is missing.")?;
            resolved.matches_installation(&installed).map_err(|e| e.to_string())?;
            (resolved.authority().map_err(|e| e.to_string())?, resolved.next_generation().map_err(|e| e.to_string())?, CERTIFICATE_SECS.min(resolved.expiry().saturating_sub(now)))
        } else {
            if installed_proof.as_ref().is_some_and(|proof| !proof.is_retained()) { return Err("This world requires its verified original invitation authority.".into()); }
            (Authority::root(&env.root, lineage).map_err(|_| "This world's invitation can't carry a handover.".to_string())?, 1, CERTIFICATE_SECS)
        };
        let cfg = SourceConfig {
            runtime: env.runtime,
            root: env.root,
            authority,
            lineage,
            // A fresh identity for this checkpoint's copy (worlds carry no
            // UUID at this base).
            world: random_id(),
            generation: checkpoint_generation,
            scratch: env.scratch,
            validity,
            world_dir: Some(crate::save::world_dir(&self.world_name)),
            bearer: env.bearer,
            attestation: env.attestation,
        };
        let action = HostAction { seat: Seat { slot, generation, key }, source_key, world_name: world.clone() };
        let _ = crate::handover::live::inert::sweep_pack_scratch(&cfg.scratch);
        let session = SourceSession::begin(hs, action, cfg, live, now).map_err(|r| r.0.to_string())?;
        self.handover = Some(Handover::Source { session: Box::new(session), world });
        Ok(())
    }

    /// The independent control pump. Every frame, every mode: the joined
    /// client's frames, the source session, the recipient session, and any
    /// button clicked last frame.
    pub(crate) fn poll_handover(&mut self) {
        let now = crate::game_loop::wardrobe_now();
        let was_frozen = self.handover_holds_play();
        if let Some(a) = self.handover_click.take() {
            self.handover_action(a, now);
        }
        self.route_joined_handover_frames(now);
        self.pump_handover_source(now);
        self.pump_handover_recipient(now);
        if was_frozen && !self.handover_holds_play() {
            // A long pause must not burst ten catch-up ticks.
            self.reset_tick_timing();
        }
    }

    /// Frames the host sent this joined client: pause notices, a proposal,
    /// or the host's cancel of a proposal not yet answered.
    fn route_joined_handover_frames(&mut self, now: u64) {
        let Some(rc) = self.remote_client.as_mut() else {
            self.host_paused = false;
            return;
        };
        for payload in rc.take_handover_frames() {
            match Frame::decode(&payload) {
                Ok(Frame::Notice { paused, .. }) => {
                    if self.host_paused && !paused {
                        self.handover_toast(copy::RESUMED);
                    }
                    self.host_paused = paused;
                }
                Ok(Frame::Propose { transfer, .. }) => self.on_handover_proposal(&payload, transfer, now),
                Ok(Frame::Abort { transfer, .. }) => {
                    if let Some(Handover::Recipient(s)) = &self.handover
                        && s.proposal().transfer == transfer
                        && s.stage() == RecipientStage::AwaitingConsent
                    {
                        self.handover = None;
                        self.handover_toast("The host cancelled the handover.");
                    }
                }
                _ => {}
            }
        }
    }

    fn decline_over_joined(&self, transfer: Id, reason: DeclineReason) {
        if let (Some(rc), Ok(bytes)) = (self.remote_client.as_ref(), (Frame::Decline { transfer, reason }).encode()) {
            let mut pkt = vec![crate::protocol::PacketType::Handover as u8];
            pkt.extend_from_slice(&bytes);
            rc.send_raw(&pkt);
        }
    }

    fn on_handover_proposal(&mut self, payload: &[u8], transfer: Id, now: u64) {
        if self.handover.is_some() {
            self.decline_over_joined(transfer, DeclineReason::Busy);
            return;
        }
        let Some(host) = self.joined_host else {
            self.decline_over_joined(transfer, DeclineReason::UnverifiedHost);
            self.handover_toast(copy::UNVERIFIED_HOST);
            return;
        };
        #[cfg(test)]
        let env = match self.handover_fixture.as_ref().and_then(|f| f.recipient.as_ref()) {
            Some(make) => make(),
            None => recipient_env(),
        };
        #[cfg(not(test))]
        let env = recipient_env();
        let Ok(env) = env else {
            self.decline_over_joined(transfer, DeclineReason::NotReady);
            self.handover_toast(copy::NOT_READY_HERE);
            return;
        };
        // P3: no session is live here (a running one answers Busy above).
        let _ = crate::handover::live::inert::sweep(&env.imports, None);
        match RecipientSession::propose_with_authority(payload, host, env.identity, env.imports, now, self.joined_continuation.as_ref()) {
            Ok(mut s) => {
                // CP12 (CP10 review LOW-2): every end path hands its wait here.
                s.hand_waits_to(crate::handover::live::resumer::Resumer::global().clone());
                self.handover = Some(Handover::Recipient(Box::new(s)));
                self.handover_consent_shown = Some(Instant::now());
            }
            Err(Refusal(why)) => {
                self.decline_over_joined(transfer, DeclineReason::Invalid);
                self.handover_toast(why);
            }
        }
    }

    fn pump_handover_source(&mut self, now: u64) {
        let live = self.handover_live();
        let Some(Handover::Source { session, .. }) = self.handover.as_mut() else { return };
        let Some(hs) = self.hosted_server.as_mut() else {
            // The server (and the link with it) went: the session ends the
            // one way every source handover ends (rotating a disclosed
            // bearer). A retired session's publication was already handed
            // to the resumer at Retired, so nothing is lost by dropping it.
            // CP14b (CP13b review LOW-1): before Retired it aborts as every
            // other end does (the abort record first, handed over below, and
            // the world's late-save fence lifted), with no link to tell.
            session.abort_without_server(copy::LEFT);
            let line = self.end_source_handover(copy::LEFT);
            log::info!("handover: {line}");
            return;
        };
        let mut result = session.pump(hs, live, now);
        if result.is_ok() && session.wants_capture() {
            // M-A: the freeze closes every HUD dialog before the capture, so
            // nothing a dialog does can land after the cut (the central gate
            // in the HUD loop holds any surface still open).
            for p in self.players.iter_mut() {
                p.close_hud_dialogs_for_freeze();
            }
            result = capture_and_offer(
                session.as_mut(),
                hs,
                live,
                now,
                Frozen {
                    world_name: &self.world_name,
                    world: &self.world,
                    ecs: &self.ecs,
                    players: &self.players,
                    seed: self.biome_gen.seed,
                    tick: self.tick_counter,
                },
            );
        }
        // CP9: at Retired the Committed publication goes to the one
        // background resumer, which outlives this world: leaving, closing
        // the window or dropping GameState never stops it.
        if matches!(session.stage(), SourceStage::Retired | SourceStage::RetirementUncertain) && !session.publication_handed() {
            session.hand_publication(crate::handover::live::resumer::Resumer::global());
        }
        if let SourceStage::Aborted(why) = session.stage() {
            let msg = match result {
                Err(Refusal(r)) => r,
                Ok(()) => why,
            };
            let line = self.end_source_handover(msg);
            self.handover_toast(line);
        }
    }

    /// P2: the disclosed bearer is retired: only the bearer changes, so the
    /// invitation's root context (expiry, relays, lineage) stays as it was.
    pub(crate) fn rotate_handover_invite(&mut self) {
        if let Some(host) = self.online_host.as_mut() {
            host.rotate_bearer();
        }
    }

    /// CP7 review HIGH-1: this machine's source session retired the world
    /// (or may have): leaving skips the save and tears the world down.
    pub(crate) fn source_handover_retired(&self) -> bool {
        matches!(&self.handover, Some(Handover::Source { session, .. }) if session.past_retirement())
    }

    /// CP7 review MEDIUM-1 + LOW-7: the ONLY way any caller (Cancel, leave,
    /// the pump) clears a Source handover. A session that aborted after its
    /// bearer left this machine rotates the invitation (`rotate_bearer`:
    /// only the bearer changes, the root stays) and the line says so.
    pub(crate) fn end_source_handover(&mut self, msg: &str) -> String {
        // CP9 review LOW-1: a last chance to hand a retired publication over
        // before the session goes (the start-up scan stays the backstop).
        // CP13b (D2): an abort record's Aborted is handed the same way.
        if let Some(Handover::Source { session, .. }) = self.handover.as_mut()
            && (session.past_retirement() || session.abort_recorded())
            && !session.publication_handed()
        {
            session.hand_publication(crate::handover::live::resumer::Resumer::global());
        }
        let (rotate, mut line) = match &self.handover {
            Some(Handover::Source { session, .. }) => (session.needs_invite_rotation(), session.end_line(msg)),
            _ => (false, msg.to_string()),
        };
        self.handover = None;
        if rotate {
            if self.online_host.as_ref().is_some_and(|host| host.continuation().is_some()) {
                line = line.replace(crate::handover::live::copy::INVITE_REISSUED,
                    "The original invite is now unavailable; contacts can still join directly.");
            }
            self.rotate_handover_invite();
        }
        line
    }

    fn pump_handover_recipient(&mut self, now: u64) {
        let Some(Handover::Recipient(s)) = self.handover.as_mut() else { return };
        let result = s.pump(now);
        // CP10 (HIGH-2): past the ack the wait for Committed goes to the one
        // background resumer, which outlives this session, the joined world
        // and the window (and resumes at start-up from the durable receipt).
        if s.stage() == RecipientStage::Prepared && !s.activation_handed() {
            s.hand_activation(now);
        }
        if s.stage() == RecipientStage::Activated {
            // The source retired its copy: the joined world is gone for good.
            let msg = copy::recipient_activated(&s.proposal().world_name);
            self.handover = None;
            if self.remote_client.as_ref().is_some_and(|rc| rc.is_parked()) {
                self.leave_world_with_notice(crate::world_exit::SaveChoice::Discard, msg);
            } else {
                self.handover_toast(msg);
            }
            return;
        }
        if let RecipientStage::Aborted(why) = s.stage() {
            let msg = match result {
                Err(Refusal(r)) => r,
                Ok(()) => why,
            };
            self.end_recipient(msg);
        } else if let RecipientStage::Quarantined(_) = s.stage() {
            // CP14b (CP13b review LOW-2): past the ack the wait goes on in
            // the background unless a verified Aborted ended it.
            let line = if s.wait_continues() { copy::waiting_after_ack(&s.proposal().source_npub) } else { copy::QUARANTINED.to_owned() };
            self.end_recipient(&line);
        }
    }

    /// A recipient session that ended before activation: the connection and
    /// the gameplay that waited go back to the joined client if it is still
    /// here; otherwise the connection closes with the session.
    fn end_recipient(&mut self, msg: &str) {
        self.end_recipient_inner(msg, false);
    }
    fn end_recipient_inner(&mut self, msg: &str, leaving: bool) {
        // CP8 review LOW-1: never take (and drop) a Source handover here.
        if !matches!(self.handover, Some(Handover::Recipient(_))) {
            return;
        }
        let Some(Handover::Recipient(mut s)) = self.handover.take() else { return };
        // CP12 (CP10 review LOW-2): past the ack, every end of the session
        // hands its wait for Committed to the resumer first (once: the pump
        // or `abort` may have already).
        s.hand_off_wait();
        // CP16 (CP14 review LOW-5, decided): only while the wait goes on.
        // After a verified Aborted quarantined the import, the source has
        // already run `handover_release` (it unfroze), so the link goes back
        // and the player keeps playing (below). An unreadable quarantine
        // check is not proof the source resumed: the player leaves.
        if (s.stage() == RecipientStage::Prepared || s.ack_started()) && !s.import_quarantined() {
            // Leaving the joined world (`handover_on_leave`), or a Cancel or
            // failure past the ack while the wait goes on (CP14b, CP13b
            // review LOW-2): the resumer owns the wait, so the session ends
            // here. Its router lease goes back (`Drop`) and its link closes
            // with it; no Abort is sent (the ack has left). The link never
            // goes back to the joined world: its host is frozen (or retired)
            // while the transfer may still commit, so the player leaves it,
            // with the line in the lobby.
            drop(s);
            if !leaving && self.remote_client.as_ref().is_some_and(|rc| rc.is_parked()) {
                self.leave_world_with_notice(crate::world_exit::SaveChoice::Discard, msg.to_owned());
            } else {
                self.handover_toast(msg);
            }
            return;
        }
        let back = s.return_link();
        let parked = self.remote_client.as_ref().is_some_and(|rc| rc.is_parked());
        match (parked, back) {
            (true, Some((link, waiting))) => {
                if let Some(rc) = self.remote_client.as_mut() {
                    rc.restore_transport(link, waiting);
                }
                // The joined session carries on: its router lease comes back.
                if let Some(lease) = s.take_lease().and_then(|l| l.into_any().downcast::<crate::online_prep::LeaseKeeper>().ok()) {
                    self.upnp_lease = Some(*lease);
                }
                self.handover_toast(msg);
            }
            (true, None) => self.leave_world_with_notice(crate::world_exit::SaveChoice::Discard, copy::RESUME_FAILED.into()),
            (false, _) => self.handover_toast(msg),
        }
    }

    fn handover_action(&mut self, action: UiAction, now: u64) {
        if matches!(action, UiAction::Accept | UiAction::Decline) && !self.handover_consent_armed() {
            return;
        }
        match (action, self.handover.as_mut()) {
            (UiAction::Accept, Some(Handover::Recipient(s))) if s.stage() == RecipientStage::AwaitingConsent => {
                let p = s.proposal().clone();
                if let Some(refusal) = self.players.first().and_then(|slot| crate::handover::live::eligibility::local_progress(slot, self.tick_counter).err()) {
                    self.handover = None;
                    self.decline_over_joined(p.transfer, DeclineReason::NotReady);
                    self.handover_toast(refusal.0);
                    return;
                }
                if !self.pending_block_changes.is_empty() || !self.window_inbox.is_empty()
                    || self.players.first().is_some_and(|slot| slot.crafting_ui.ops.first_stamp().is_some())
                    || self.remote_client.as_ref().is_some_and(|client| !client.handover_authority_settled()) {
                    self.handover_toast("Your inventory is still settling. Try again in a moment.");
                    return;
                }
                let Some(rc) = self.remote_client.as_mut() else {
                    self.handover = None;
                    return;
                };
                let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") else {
                    self.decline_over_joined(p.transfer, DeclineReason::NotReady);
                    self.handover = None;
                    self.handover_toast("This machine couldn't open a port for the handover, so it was declined.");
                    return;
                };
                let link = rc.park_transport();
                let consent = RecipientConsent { transfer: p.transfer, world_name: p.world_name, source_npub: p.source_npub };
                match s.consent(consent, socket, link, now) {
                    // L5: the router lease goes with the link it serves, so
                    // leaving the joined world can't give it back early.
                    Ok(()) => {
                        if let Some(lease) = self.upnp_lease.take() {
                            s.hold_lease(Box::new(lease));
                        }
                    }
                    Err(Refusal(why)) => {
                        // The refused consent took the connection with it.
                        self.handover = None;
                        self.leave_world_with_notice(crate::world_exit::SaveChoice::Discard, why.into());
                    }
                }
            }
            (UiAction::Decline, Some(Handover::Recipient(s))) if s.stage() == RecipientStage::AwaitingConsent => {
                if let Some(pkt) = s.decline()
                    && let Some(rc) = self.remote_client.as_ref()
                {
                    rc.send_raw(&pkt);
                }
                self.handover = None;
                self.handover_toast(copy::DECLINED);
            }
            (UiAction::Cancel, Some(Handover::Recipient(s))) => {
                // CP13b (CP13 review LOW-1): after the ack nothing is
                // cancelled; the wait goes on in the background.
                let line = if s.ack_started() { copy::waiting_after_ack(&s.proposal().source_npub) } else { copy::CANCELLED.to_owned() };
                s.abort("You cancelled the handover.");
                self.end_recipient(&line);
            }
            (UiAction::Cancel, Some(Handover::Source { session, .. })) => {
                // CP7 review HIGH-1: Cancel is not drawn past retirement, and
                // a click that slipped through cancels nothing.
                if session.past_retirement() {
                    return;
                }
                if let Some(hs) = self.hosted_server.as_mut() {
                    session.abort(hs, "cancelled");
                }
                let line = self.end_source_handover(copy::CANCELLED);
                self.handover_toast(line);
            }
            _ => {}
        }
    }

    /// Leaving the world: a source handover is aborted while its world and
    /// link still exist (before any save, which the freeze refuses); an
    /// unanswered proposal is declined. A recipient past consent holds its
    /// own connection and carries on, until Prepared: past the ack the
    /// resumer owns the wait, so the session ends (CP12, CP10 review LOW-2).
    /// Returns the line the lobby shows once the player is there (a source
    /// handover was cancelled by leaving): the caller puts it in the lobby's
    /// `menu.notice` after the leave, where it is actually drawn.
    pub(crate) fn handover_on_leave(&mut self) -> Option<String> {
        let mut notice = None;
        match self.handover.as_mut() {
            Some(Handover::Source { session, .. }) => {
                if let Some(hs) = self.hosted_server.as_mut() {
                    session.abort(hs, "left");
                }
                // Past retirement leaving cancels nothing (the world is retired).
                let cancelled = !session.past_retirement();
                // MEDIUM-1: rotates the invitation if the bearer had left.
                // CP14b (CP13b review LOW-5): the composed line (with the
                // re-issued invite and an unsaved cancellation) is the notice.
                let line = self.end_source_handover(copy::LEFT);
                if cancelled {
                    notice = Some(line);
                }
            }
            Some(Handover::Recipient(s)) if s.stage() == RecipientStage::AwaitingConsent => {
                let transfer = s.proposal().transfer;
                self.handover = None;
                self.decline_over_joined(transfer, DeclineReason::Player);
            }
            // CP12 (CP10 review LOW-2): `end_recipient` hands the wait over,
            // gives the lease back and closes the link.
            Some(Handover::Recipient(s)) if s.stage() == RecipientStage::Prepared => {
                let line = copy::awaiting_handover(&s.proposal().source_npub);
                self.end_recipient_inner(&line, true);
            }
            _ => {}
        }
        self.host_paused = false;
        self.joined_host = None;
        self.joined_continuation = None;
        notice
    }

    /// The handover window has something to click (consent, Cancel): the
    /// cursor must be free for it, mid-play too.
    pub(crate) fn handover_wants_pointer(&self) -> bool {
        match &self.handover {
            Some(Handover::Recipient(s)) => {
                !matches!(s.stage(), RecipientStage::Aborted(_) | RecipientStage::Quarantined(_) | RecipientStage::Activated)
            }
            Some(Handover::Source { session, .. }) => source_wants_pointer(session.stage()),
            None => false,
        }
    }

    /// Review LOW-7: at exit a recipient session's router lease goes back
    /// within the same bound as the online host's own.
    pub(crate) fn handover_release_lease_on_exit(&mut self, grace: std::time::Duration) {
        if let Some(Handover::Recipient(s)) = self.handover.as_mut() {
            s.release_lease_within(grace);
        }
    }

    /// A joined session that verified its host: online play's verified
    /// answer and the runtime it was signed by.
    pub(crate) fn set_joined_host(&mut self, persona: [u8; 32], runtime: [u8; 32]) {
        self.joined_continuation = None;
        self.joined_host = (persona != [0; 32]).then_some(JoinedHost { persona, runtime });
    }
}

/// L4: the transfer copy of the world's settings carries the LIVE world's
/// Proof-of-Play total and clock (the saved meta can be an autosave behind),
/// then is sanitised.
/// Review LOW-6: the source host keeps mouselook, break/place and attack
/// while the recipient decides (its world still runs); only a frozen world
/// frees the pointer for the handover window. In `Proposed` the window's
/// Cancel is reached through the pause menu (Esc), where it is drawn too.
pub(crate) fn source_wants_pointer(stage: SourceStage) -> bool {
    !matches!(stage, SourceStage::Proposed | SourceStage::Aborted(_))
}

pub(crate) fn live_transfer_metadata(
    mut saved: crate::save::WorldMeta,
    world: &crate::world::World,
) -> crate::handover::Result<crate::save::WorldMeta> {
    saved.total_work = world.total_work;
    saved.total_ticks = world.total_ticks;
    crate::handover::checkpoint::transfer_metadata(&saved)
}

/// The real frozen world and seats the capture reads.
struct Frozen<'a> {
    world_name: &'a str,
    world: &'a crate::world::World,
    ecs: &'a hecs::World,
    players: &'a [crate::player_slot::PlayerSlot],
    seed: u32,
    tick: u64,
}

/// The frozen capture: the RAW save from the real lent world, ECS and local
/// seats (the session neutralises it), the world's own chunk and exhibit
/// folders, and seat 0's own records from the real `PlayerSlot`.
fn capture_and_offer(
    session: &mut SourceSession,
    hs: &mut crate::hosted_server::HostedServer,
    live: Live,
    now: u64,
    f: Frozen<'_>,
) -> Result<(), Refusal> {
    let fail = |session: &mut SourceSession, hs: &mut crate::hosted_server::HostedServer, r: Refusal| {
        session.abort(hs, r.0);
        Err(r)
    };
    let Some(seat0) = f.players.first() else {
        return fail(session, hs, Refusal("This world has no local player to hand over from."));
    };
    if let Err(r) = crate::handover::live::eligibility::local_progress(seat0, f.tick) {
        return fail(session, hs, r);
    }
    let source_metadata = crate::handover::live::personal_adapter::read_regular(
        &crate::save::world_dir(f.world_name).join("world_meta.json"), 2 * 1024 * 1024,
    ).map_err(|_| crate::handover::Error::Io)
        .and_then(|bytes| crate::handover::checkpoint::source_metadata(&bytes));
    let meta = match source_metadata.and_then(|saved| live_transfer_metadata(saved, f.world)) {
        Ok(m) => m,
        Err(_) => return fail(session, hs, Refusal("This world's saved history and settings couldn't be read. Nothing moved; play has resumed.")),
    };
    let carts = crate::save::carts_to_saved(f.ecs);
    let kept = match crate::save::tamed_mobs_to_saved(f.ecs) {
        Ok(kept) => kept,
        Err(error) => {
            log::warn!("Handover pet capture refused: {error}");
            return fail(session, hs, Refusal("This world's kept animals couldn't be saved; the handover was stopped."));
        }
    };
    let Some(save) = crate::save::build_world_save(f.world, f.players, f.seed, &carts, &kept) else {
        return fail(session, hs, Refusal("This world has no local player to hand over from."));
    };
    let sidecar = match capture::local0_sidecar(seat0, hs.server.run_id) {
        Ok(b) => b,
        Err(r) => return fail(session, hs, r),
    };
    let progress = match capture::local0_progress(seat0, session.record_context(session.source_key())) {
        Ok(b) => b,
        Err(r) => return fail(session, hs, r),
    };
    let dir = crate::save::world_dir(f.world_name);
    let (chunks, exhibits) = (dir.join("chunks"), dir.join("exhibits"));
    let capture = Capture {
        meta: &meta,
        save,
        world: f.world,
        disk_chunks: chunks.is_dir().then_some(chunks.as_path()),
        exhibit_dir: exhibits.is_dir().then_some(exhibits.as_path()),
        source_sidecar: &sidecar,
        source_progress: &progress,
    };
    session.offer(hs, live, capture, now)
}
