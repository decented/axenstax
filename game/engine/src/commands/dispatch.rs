//! Command dispatch + ChatLine output channel.
//!
//! `CommandContext` is the API surface commands see. It holds a slim subset
//! of game state — bounded so the trait stays portable across games.

use super::parser::{parse, ParseError};
use super::registry::{CommandRegistry, OpLevel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatLineKind {
    Info,
    Echo,
    Success,
    Error,
    /// Game-emitted system messages (e.g. "World saved", "Autosave failed",
    /// and — world chat Phase 2 — a chat refusal or rate-limit notice).
    System,
    /// A line from another player in the world (world chat, Phase 2).
    /// Ordinary readable text — this is what most chat looks like.
    Player,
    /// A line relayed from the attached room (world chat §4 — the room plug
    /// that reaches a guardian's phone). Tinted distinctly so it visibly
    /// reads as "from outside the world", not from another player in-game.
    Room,
}

#[derive(Clone, Debug)]
pub struct ChatLine {
    pub text: String,
    pub kind: ChatLineKind,
    pub at_tick: u64,
}

impl ChatLine {
    pub fn info(text: impl Into<String>, tick: u64) -> Self {
        Self { text: text.into(), kind: ChatLineKind::Info, at_tick: tick }
    }
    pub fn echo(text: impl Into<String>, tick: u64) -> Self {
        Self { text: text.into(), kind: ChatLineKind::Echo, at_tick: tick }
    }
    pub fn success(text: impl Into<String>, tick: u64) -> Self {
        Self { text: text.into(), kind: ChatLineKind::Success, at_tick: tick }
    }
    pub fn error(text: impl Into<String>, tick: u64) -> Self {
        Self { text: text.into(), kind: ChatLineKind::Error, at_tick: tick }
    }
    pub fn system(text: impl Into<String>, tick: u64) -> Self {
        Self { text: text.into(), kind: ChatLineKind::System, at_tick: tick }
    }
    pub fn player(text: impl Into<String>, tick: u64) -> Self {
        Self { text: text.into(), kind: ChatLineKind::Player, at_tick: tick }
    }
    pub fn room(text: impl Into<String>, tick: u64) -> Self {
        Self { text: text.into(), kind: ChatLineKind::Room, at_tick: tick }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CommandResult {
    Success,
    Error(String),
    Silent,
    /// Side-effect: the game loop should zero `Health` on every mob in the
    /// ECS after dispatch returns. Treated as `Success` for cheat-marker
    /// purposes. Adding more ECS-touching side effects = add more variants.
    KillAllMobs,
    /// Side-effect: spawn one mob of the given kind at the given world
    /// position. Treated as `Success` for cheat-marker purposes.
    SpawnMob(crate::mob::MobType, glam::Vec3),
    /// #129 side-effect: PLACE an authored wild mob — spawn it with the
    /// `Authored` ECS marker so it persists in the `.axeworld` (via `saved_mobs`)
    /// and respawns on load. The game loop owns the ECS, so it does the spawn.
    PlaceAuthoredMob(crate::mob::MobType, glam::Vec3),
    /// Rail freight (Phase 1) side-effect: spawn a cart on the TRACK cell the
    /// dispatching player is aiming at. The game loop raycasts from the
    /// player's eye/look (the command has no ray access) and refuses if the
    /// targeted block isn't track. Treated as `Success` for cheat-marker
    /// purposes (debug spawn).
    SpawnCart,
    /// Side-effect: despawn every dropped ItemEntity in the world. Treated
    /// as `Success` for cheat-marker purposes.
    ClearItems,
    /// #7 (WorldEdit) side-effect: the command mutated world blocks inside the
    /// inclusive cuboid `min..=max`; the game loop must re-mesh every chunk the
    /// cuboid touches (`worldedit::affected_chunks`). Treated as `Success` for
    /// cheat-marker purposes (creative-power editing).
    /// `changed` lists every cell the edit actually changed, so the host can
    /// mirror them into its hosted server's world and broadcast them.
    RebuildRegion { min: [i32; 3], max: [i32; 3], changed: Vec<[i32; 3]> },
    /// #9 (build-guide) side-effect: project the named plan as a build-guide
    /// anchored at `origin`. The game loop looks the plan up in
    /// `world.plan_registry`, clones its cells, and sets `GameState.build_guide`.
    StartBuildGuide {
        name: String,
        origin: [i32; 3],
        /// Guided build-along step mode the guide is laid in.
        mode: crate::build_steps::StepMode,
    },
    /// #9 side-effect: clear the active build-guide.
    StopBuildGuide,
    /// Guided build-along: switch the *active* guide's step mode without
    /// re-laying it (the mode picker from chat).
    SetBuildGuideMode(crate::build_steps::StepMode),
    /// Side-effect: start the given scenario (Goal 1 runner). The game loop
    /// clears the player's inventory, provisions the def's kit, sets
    /// `self.scenario`, and resets tick timing. Unboxed per the Goal 1 spec.
    StartScenario { def: crate::scenario::ScenarioDef },
    /// Spec 40 (The Workshop) side-effect: the override registry on the World was
    /// mutated (a reskin was pinned, or overrides were reset). The game loop must
    /// rebuild the block texture array from `world.override_registry.appended_layers()`
    /// and re-mesh loaded chunks so block faces pick up the new layers. Treated as
    /// `Success` for cheat-marker purposes (creative-only authoring, render-only).
    ApplyWorkshopOverrides,
    /// Spec 40 blow-up Phase 4 side-effect: pin the locked ×4 working copy of the
    /// given project id onto every instance of its block type (paint →
    /// `AuthoredFaces`, shape → coloured micro-model) and collapse it ×4→×1. The
    /// game loop calls `commit_locked_balloon(id)`. Treated as `Success` for
    /// cheat-marker purposes (creative-only authoring, render-only).
    CommitLockedBalloon(u32),
    /// Spec 40 Phase F (Mode B reshape) side-effect: bake the captured `plan` into a
    /// micro-model and register it as `block_id`'s shape override
    /// (`World::micro_registry`), then re-mesh loaded chunks so the block renders as
    /// the new 3D form. The bake needs the `BlockRegistry` (on the game loop), hence
    /// the side-effect rather than baking inside the command.
    ApplyWorkshopReshape {
        block_id: crate::block::BlockId,
        plan: crate::plan::PlanData,
    },
    /// Spec 40 (The Workshop) side-effect: open the face-painter panel for the
    /// given asset (the game loop seeds it from the asset's current textures).
    OpenWorkshopPainter {
        target: crate::workshop_painter::PaintTarget,
    },
    /// Spec 40 (The Workshop) side-effect: set the mannequin "play" (animate)
    /// preview flag.
    SetWorkshopPlay(bool),
    /// `/ws publish confirm` side-effect (PWA-only): the game loop spawn_locals the
    /// async Beacon publish of these already-serialised, version-prefixed bytes under
    /// the player's npub. Cross-platform variant; the live publish is wasm-gated in the
    /// loop. Treated as a cheat-marking success (authoring → World Integrity Ledger).
    PublishOverrideSet { name: String, bytes: Vec<u8> },
    /// Async Beacon follow toggle (PWA-only); the game loop spawn_locals it. Not a cheat.
    BeaconFollow { pubkey_hex: String, follow: bool },
    /// List who you follow (PWA-only). Not a cheat.
    BeaconFollowing,
    /// Browse published override sets — all followed creators, or one npub (PWA-only). Not a cheat.
    BeaconBrowse { pubkey_hex: Option<String> },
    /// Adopt a browsed override set by its blob hash (PWA-only). A cheat (changes your art).
    BeaconAdopt { blob_hash: String },
    /// Workshop Phase 5 Task 6 — `/ws gallery` side-effect: open the Wardrobe panel.
    /// The game loop sets `wardrobe_open = true` and releases the cursor.
    OpenWardrobe,
    /// Trials (⚡ Race) side-effect: launch the named trial. The game loop snaps
    /// the start/finish to the surface, teleports the player to the start, places
    /// the finish marker, loads the personal-best ghost to chase, and arms the
    /// run. Not a cheat (it's a self-contained race in your own world).
    StartTrial(String),
    /// Trials side-effect: cancel the active trial (clear `GameState.active_trial`).
    StopTrial,
    /// World chat §4.5 — `/room join <link>` side-effect. The room lives on
    /// `HostedServer`, which `CommandContext` has no access to (kept narrow on
    /// purpose); the game loop applies this against `self.hosted_server` after
    /// dispatch, the pattern every other server-reaching command already uses.
    /// Native only in effect (no-op on web — there is no chat there at all).
    RoomJoin(String),
    /// World chat §4.5 — `/room leave` side-effect: detach the room, if any.
    RoomLeave,
    /// World chat §4.5 — `/room` (no args) side-effect: print status (attached
    /// or not, the link, member count, relays).
    RoomStatus,
    /// World chat §4.5 — `/room invite` side-effect: print the current room
    /// link so it can be copied.
    RoomInvite,
    /// `/online` — the game loop prints the invite link, the reachability
    /// summary and the relay count into chat, and opens the Online panel.
    OnlineStatus,
    /// `/online copy` — the game loop puts the invite link on the clipboard.
    OnlineCopyInvite,
    /// Wind, Copper & Electricity wave §4 side-effect: a `/bug` or `/idea`
    /// report was queued for the makers. The report itself is already handled
    /// (native outbox) — this only tells the game loop to fire
    /// `ChallengeEvent::SendFeedback` at the running trial, which the commands
    /// module has no access to. Not a cheat. Native only: the browser build has
    /// no `/bug` or `/idea`.
    #[cfg(not(target_arch = "wasm32"))]
    FeedbackQueued,
}

/// Bounded view of game state that commands may touch.
///
/// Kept narrow so the trait stays portable. Anything that needs deeper access
/// gets staged through here explicitly rather than handing commands a full
/// `&mut GameState` (which would couple every command to the entire engine).
pub struct CommandContext<'a> {
    /// Reserved for future commands that need to mutate terrain (e.g.
    /// `/setblock`, `/fill`). No v1 builtin uses it; the field is kept so
    /// adding such a command doesn't churn the trait surface.
    #[allow(dead_code)]
    pub world: &'a mut crate::world::World,
    pub world_time: &'a mut u32,
    /// Default world-time advance per tick (alpha=4 for 5-min day; 1 = 20-min day).
    pub world_time_step: &'a mut u32,
    pub is_creative: &'a mut bool,
    /// Source of truth for the play mode. `/gamemode` mutates this AND the
    /// `is_creative` projection above together (both borrow the same GameState
    /// fields, so they stay consistent post-dispatch). Other commands read
    /// `is_creative` and ignore this.
    pub play_mode: &'a mut crate::play_mode::PlayMode,
    /// World seed for /seed.
    pub seed: u32,
    /// Reserved for future commands that need to identify the world (e.g.
    /// future `/save` or per-world flag toggles). v1 builtins don't read it;
    /// the game loop persists `WorldMeta` directly via the marker out-params.
    #[allow(dead_code)]
    pub world_name: &'a str,
    pub players: &'a mut Vec<crate::player_slot::PlayerSlot>,
    pub player_idx: usize,
    pub op_level: OpLevel,
    pub current_tick: u64,
    pub log: &'a mut Vec<ChatLine>,
    pub registry: &'a CommandRegistry,
    /// Out-param: set to `true` if the command was a successful cheat. The
    /// caller (game loop) is responsible for persisting `WorldMeta.cheats_used`.
    /// This indirection keeps the commands module free of save-format
    /// dependencies and lifts cleanly to other games.
    pub cheats_used_marker: &'a mut bool,
    /// Out-param: set to `true` if the command marked the world as having
    /// been creative (one-way ledger flag). Caller persists.
    pub ever_creative_marker: &'a mut bool,
    /// Out-param: set to `true` if the command broke pure-survival.
    pub pure_survival_broken_marker: &'a mut bool,
}

impl<'a> CommandContext<'a> {
    pub fn echo(&mut self, text: impl Into<String>) {
        self.log.push(ChatLine::echo(text, self.current_tick));
    }
    pub fn info(&mut self, text: impl Into<String>) {
        self.log.push(ChatLine::info(text, self.current_tick));
    }
    pub fn success(&mut self, text: impl Into<String>) {
        self.log.push(ChatLine::success(text, self.current_tick));
    }
    pub fn error(&mut self, text: impl Into<String>) {
        self.log.push(ChatLine::error(text, self.current_tick));
    }
}

/// Parse `input`, look up the command, check permissions, execute. Returns
/// the command's `CommandResult` and pushes user-facing lines to `ctx.log`.
pub fn dispatch(
    input: &str,
    ctx: &mut CommandContext,
    registry: &CommandRegistry,
) -> CommandResult {
    // Echo the user's input verbatim into the log (so they see what they typed
    // in the rolling chat output).
    ctx.echo(input.to_string());

    let parsed = match parse(input) {
        Ok(p) => p,
        Err(e) => {
            let msg = match e {
                ParseError::Empty => "empty input".to_string(),
                ParseError::NotACommand => "not a command (must start with /)".to_string(),
                ParseError::OnlySlash => "missing command name".to_string(),
                ParseError::UnclosedQuote => "unclosed quote".to_string(),
                ParseError::BadEscape => "bad escape sequence".to_string(),
            };
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        }
    };

    let cmd = match registry.lookup(&parsed.name) {
        Some(c) => c,
        None => {
            let msg = format!("unknown command: /{}", parsed.name);
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        }
    };

    if ctx.op_level < cmd.min_op_level() {
        let msg = format!("permission denied: /{}", parsed.name);
        ctx.error(msg.clone());
        return CommandResult::Error(msg);
    }

    let result = cmd.execute(ctx, &parsed.args);

    // Cheat flag is set iff the command claims to be a cheat AND it succeeded.
    // KillAllMobs / SpawnMob are side-effect-bearing successes.
    if cmd.is_cheat() && matches!(
        result,
        CommandResult::Success
            | CommandResult::KillAllMobs
            | CommandResult::SpawnMob(_, _)
            | CommandResult::PlaceAuthoredMob(_, _)
            | CommandResult::SpawnCart
            | CommandResult::ClearItems
            | CommandResult::PublishOverrideSet { .. }
            | CommandResult::BeaconAdopt { .. }
    ) {
        *ctx.cheats_used_marker = true;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::Command;
    use crate::world::World;
    use crate::player_slot::PlayerSlot;

    struct EchoCmd {
        cheat: bool,
        op: OpLevel,
    }
    impl Command for EchoCmd {
        fn name(&self) -> &'static str { "echo" }
        fn help(&self) -> &'static str { "echo" }
        fn usage(&self) -> &'static str { "/echo" }
        fn min_op_level(&self) -> OpLevel { self.op }
        fn is_cheat(&self) -> bool { self.cheat }
        fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
            ctx.success(args.join(" "));
            CommandResult::Success
        }
    }

    fn make_ctx_state() -> (
        World,
        u32,
        u32,
        bool,
        crate::play_mode::PlayMode,
        Vec<PlayerSlot>,
        Vec<ChatLine>,
        bool,
        bool,
        bool,
        CommandRegistry,
    ) {
        let world = World::new();
        let players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        let registry = CommandRegistry::new();
        (world, 0u32, 4u32, false, crate::play_mode::PlayMode::Survival, players, Vec::new(), false, false, false, registry)
    }

    macro_rules! make_ctx {
        ($world:expr, $time:expr, $step:expr, $creative:expr, $pm:expr, $players:expr, $log:expr,
         $cheats:expr, $ever:expr, $pure_broken:expr, $reg:expr, $op:expr) => {
            CommandContext {
                world: &mut $world,
                world_time: &mut $time,
                world_time_step: &mut $step,
                is_creative: &mut $creative,
                play_mode: &mut $pm,
                seed: 42,
                world_name: "test",
                players: &mut $players,
                player_idx: 0,
                op_level: $op,
                current_tick: 0,
                log: &mut $log,
                registry: &$reg,
                cheats_used_marker: &mut $cheats,
                ever_creative_marker: &mut $ever,
                pure_survival_broken_marker: &mut $pure_broken,
            }
        };
    }

    #[test]
    fn unknown_command_errors() {
        let (mut w, mut t, mut s, mut cr, mut pm, mut p, mut log, mut ch, mut ev, mut ps, mut reg) =
            make_ctx_state();
        reg.register(Box::new(EchoCmd { cheat: false, op: OpLevel::None }));
        let mut ctx = make_ctx!(w, t, s, cr, pm, p, log, ch, ev, ps, reg, OpLevel::Op);
        let r = dispatch("/nope", &mut ctx, &reg);
        assert!(matches!(r, CommandResult::Error(_)));
        assert!(!ch);
    }

    #[test]
    fn parse_error_doesnt_set_cheat() {
        let (mut w, mut t, mut s, mut cr, mut pm, mut p, mut log, mut ch, mut ev, mut ps, reg) =
            make_ctx_state();
        let mut ctx = make_ctx!(w, t, s, cr, pm, p, log, ch, ev, ps, reg, OpLevel::Op);
        let r = dispatch("not a command", &mut ctx, &reg);
        assert!(matches!(r, CommandResult::Error(_)));
        assert!(!ch);
    }

    #[test]
    fn op_required_blocks_unprivileged() {
        let (mut w, mut t, mut s, mut cr, mut pm, mut p, mut log, mut ch, mut ev, mut ps, mut reg) =
            make_ctx_state();
        reg.register(Box::new(EchoCmd { cheat: true, op: OpLevel::Op }));
        let mut ctx = make_ctx!(w, t, s, cr, pm, p, log, ch, ev, ps, reg, OpLevel::None);
        let r = dispatch("/echo hi", &mut ctx, &reg);
        assert!(matches!(r, CommandResult::Error(_)));
        assert!(!ch); // cheat NOT set on permission denial
    }

    #[test]
    fn successful_cheat_sets_marker() {
        let (mut w, mut t, mut s, mut cr, mut pm, mut p, mut log, mut ch, mut ev, mut ps, mut reg) =
            make_ctx_state();
        reg.register(Box::new(EchoCmd { cheat: true, op: OpLevel::Op }));
        let mut ctx = make_ctx!(w, t, s, cr, pm, p, log, ch, ev, ps, reg, OpLevel::Op);
        let r = dispatch("/echo hi", &mut ctx, &reg);
        assert_eq!(r, CommandResult::Success);
        assert!(ch); // cheat marker now set
    }

    #[test]
    fn successful_non_cheat_doesnt_set_marker() {
        let (mut w, mut t, mut s, mut cr, mut pm, mut p, mut log, mut ch, mut ev, mut ps, mut reg) =
            make_ctx_state();
        reg.register(Box::new(EchoCmd { cheat: false, op: OpLevel::None }));
        let mut ctx = make_ctx!(w, t, s, cr, pm, p, log, ch, ev, ps, reg, OpLevel::None);
        let r = dispatch("/echo hi", &mut ctx, &reg);
        assert_eq!(r, CommandResult::Success);
        assert!(!ch);
    }

    /// FIX 3 regression guard — `/gamemode adventure` through the real `dispatch()`
    /// entry point must set `cheats_used_marker`, which is what triggers
    /// `WorldMeta.game_mode` persistence in the game loop.
    #[test]
    fn gamemode_adventure_via_dispatch_sets_cheats_used() {
        let (mut w, mut t, mut s, mut cr, mut pm, mut p, mut log, mut ch, mut ev, mut ps, mut reg) =
            make_ctx_state();
        crate::commands::builtins::register_all(&mut reg);
        let r = {
            let mut ctx = make_ctx!(w, t, s, cr, pm, p, log, ch, ev, ps, reg, OpLevel::Op);
            dispatch("/gamemode adventure", &mut ctx, &reg)
        }; // ctx (and its mut borrow of ch/pm) dropped here
        assert_eq!(r, CommandResult::Success);
        assert!(ch, "cheats_used_marker must be set so game_mode persists");
        assert_eq!(pm, crate::play_mode::PlayMode::Adventure);
    }
}
