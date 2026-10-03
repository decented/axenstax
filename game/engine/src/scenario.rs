//! Scenario runner — the permanent in-engine capability to RUN a challenge
//! scenario from a declarative [`ScenarioDef`] (DATA), with **no specific game
//! hardcoded**. The games (Hash Dash, Satori Rush) are `ScenarioDef` values
//! delivered as official open-stash mods; this module is only the runner.
//!
//! Design: a [`ScenarioState`] lives as `pub(crate) scenario: Option<ScenarioState>`
//! on `GameState` (deliberately NOT a `GameMode` variant — avoids exhaustive
//! match surgery). The game loop drives it through three hooks:
//!   * [`ScenarioState::tick`] — once per sim tick, after `GameState::tick`.
//!   * [`ScenarioState::on_block_broken`] — on every block-break completion
//!     (survival AND creative paths), with the block's work value.
//!   * [`ScenarioState::on_material_gained`] — when a material enters the
//!     inventory (e.g. a Satori gem), arming objective completion.
//!
//! Scoring is **work** (the proof-of-play tally), computed from
//! `crafting::block_work` — see `docs/foundations/2026-06-03-work-based-hashing.md`
//! and `docs/superpowers/specs/2026-06-03-conference-demo-plan.md` §2/§2b.

use serde::{Deserialize, Serialize};

use crate::item::MaterialId;

/// Which built-in challenge a def describes. Defs are *data*, but the kind tags
/// the family so menus / official-content can label them. Open in spirit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScenarioKind {
    /// Timed work score-attack (Goal 3).
    HashDash,
    /// Untimed Genesis-Block speedrun (Goal 4).
    SatoriRush,
    /// A feature-coverage challenge (onboarding arc / explorer card) — steers a
    /// tester through a specific feature. No leaderboard.
    Challenge,
    /// Generic/test scenario — the runner's own coverage + `/scenario test`.
    Test,
}

impl ScenarioKind {
    /// Short player-facing tag for an Experience card ("Race", "Speedrun").
    /// "Experience" is a broad umbrella, so this keeps each one legible at a
    /// glance in the Stash Column. (`scenario` stays the code term throughout.)
    pub fn label(&self) -> &'static str {
        match self {
            ScenarioKind::HashDash => "Race",
            ScenarioKind::SatoriRush => "Speedrun",
            ScenarioKind::Challenge => "Challenge",
            ScenarioKind::Test => "Test",
        }
    }
}

/// How an experience's world is managed across repeat plays. Authorable per def
/// (the Stash Column "Play" path consumes it; the in-world `/scenario` command
/// does not — it runs in your current world).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ArenaMode {
    /// ONE world per experience, regenerated FRESH every play (the prior run is
    /// wiped). The "quick throwaway attempt" model — Hash Dash. Default.
    #[default]
    Reuse,
    /// ONE world per experience, RESUMED if it already exists (fresh only the
    /// first time). The take-home model — your run persists (Satori Rush); pairs
    /// with a resumable objective.
    Resume,
    /// A brand-NEW saved world every play (they accumulate in your list). For
    /// experiences where keeping many separate runs is the point.
    KeepNew,
}

/// The launch plan for an experience world, decided from its [`ArenaMode`] and
/// whether a world for it already exists. PURE → unit-testable (the platform
/// world-create/delete lifecycle wrapped around it is not).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaLaunch {
    /// Enter the existing world and let its persisted run resume; create nothing.
    Resume,
    /// Create + enter a fresh world. `wipe` ⇒ delete the existing save first so
    /// the terrain regenerates clean (Reuse over an existing world).
    Fresh { wipe: bool },
}

/// Decide how a "Play" launches, given the def's [`ArenaMode`] and whether a
/// world for this experience already exists.
pub fn plan_arena_launch(mode: ArenaMode, world_exists: bool) -> ArenaLaunch {
    match mode {
        ArenaMode::Resume if world_exists => ArenaLaunch::Resume,
        // Reuse always regenerates fresh — wiping first only if a stale world is
        // there. Resume's first play + KeepNew have nothing to wipe (KeepNew's
        // folder is salted-unique, so it never collides).
        ArenaMode::Reuse => ArenaLaunch::Fresh { wipe: world_exists },
        ArenaMode::Resume | ArenaMode::KeepNew => ArenaLaunch::Fresh { wipe: false },
    }
}

/// Deterministic world-folder slug for an experience — Reuse/Resume share ONE
/// world per experience, so the folder must be stable (NOT run through
/// `sanitize_folder_name`, whose native collision suffix would break stability).
/// Lowercased, non-alphanumerics collapsed to single dashes, `exp-` prefixed.
/// KeepNew appends a random salt to this for uniqueness.
pub fn experience_world_slug(display_name: &str) -> String {
    let mapped: String = display_name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let core = mapped.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    format!("exp-{}", if core.is_empty() { "x" } else { &core })
}

/// One item provisioned into the player's kit at scenario start. Resolved at
/// use-time via `commands::builtins::give::resolve_item` (NO precomputed
/// stacks). `name` is the `/give` item vocabulary, e.g. `"stone_pickaxe"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KitItem {
    pub name: String,
    pub count: u8,
}

/// How a scenario is scored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scoring {
    /// Running score = total work (sum of `block_work` over `can_harvest` breaks).
    Work,
    /// Running score = the CUMULATIVE number of DISTINCT item KINDS the player has
    /// held at any point — the Scavenger trial (3 minutes, survival, from nothing:
    /// grab or craft as many DIFFERENT things as you can). Accumulated from the
    /// player's inventory each tick via [`ScenarioState::accumulate_variety`], so
    /// the score only ever GOES UP — crafting something away or using it up never
    /// lowers it (everything you ever collected or made counts). Variety, not
    /// quantity: a hundred of one block scores 1; one each of a hundred kinds scores 100.
    InventoryVariety,
    /// No running score; the result is the objective outcome (e.g. a time).
    None,
}

/// A feature-tagged completion event a coverage challenge can observe. Each
/// variant maps to a real subsystem call site (wired in Phase 3). Incoming
/// events always carry a concrete value (e.g. `BreakBlock { block: Some(id) }`);
/// an objective's *pattern* may leave the filter `None` to match any. Kept
/// minimal for v1 (no general Item/Mob taxonomy yet — `TameMob`/`CraftItem`
/// stay unfiltered; a companion tame and a wolf tame both just fire
/// `TameMob`, so a bundled challenge can only tell them apart by which mobs
/// its own arena spawns). `BreedAnimals` DOES carry an optional offspring
/// `MobType` filter (Task 19) — added so a "breed a Mule" trial can't be
/// satisfied by breeding two cows. Cross-game-generic — no AxeNStax
/// coupling beyond the block/material/mob id types it references.
/// Foundation: `docs/foundations/2026-06-06-feature-coverage-challenges.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChallengeEvent {
    /// A block-break completed. `None` = any block; `Some(id)` = that block.
    BreakBlock { block: Option<crate::block::BlockId> },
    /// A block-place committed. `None` = any block; `Some(id)` = that block.
    PlaceBlock { block: Option<crate::block::BlockId> },
    /// A craft committed (any recipe — v1 has no per-item filter).
    CraftItem,
    /// A cooked output was taken from a campfire.
    CookAtCampfire,
    /// A mob was tamed (wolf for alpha).
    TameMob,
    /// A Vendor purchase settled.
    VendorSale,
    /// A Workshop reskin/reshape was published.
    WorkshopPublish,
    /// A Plot was claimed/anchored.
    ClaimPlot,
    /// A material entered the inventory — generalises `FirstSatori` to any id.
    GainMaterial { material: MaterialId },
    /// A mob was killed by the player (combat).
    KillMob,
    /// The player ate any food (hunger system).
    EatFood,
    /// A mature crop was harvested (farming).
    HarvestCrop,
    /// A smelted output was taken from a furnace.
    SmeltItem,
    /// A fish was caught (fishing).
    CatchFish,
    /// The player mounted a rideable entity (cart or animal).
    RideEntity,
    /// An electricity device became powered (lamp lit / sensor tripped).
    PowerDevice,
    /// A piston extended (redstone-style mechanism).
    UsePiston,
    /// An explosion detonated (TNT / explosives).
    Detonate,
    /// A bucket was filled or emptied (fluids).
    UseBucket,
    /// A sheep was sheared or a cow milked (animal products).
    ShearOrMilk,
    /// Two animals were bred (breeding). `None` = any offspring; `Some(kind)`
    /// = only when the baby produced is that species (e.g. `Mule`, the one
    /// hybrid the breeding system produces — see `breeding::offspring_kind`).
    BreedAnimals { offspring: Option<crate::mob::MobType> },
    /// A self-driving power source went from idle to TURNING (Wind, Copper &
    /// Electricity wave §4). Distinct from `PowerDevice`, which is the far end
    /// of the circuit (a lamp lighting): this is the source itself catching the
    /// stream or the wind, so a trial can say "get the wheel turning" and then
    /// "and now light something with it" as two separate steps. `kind` is a
    /// required filter — a water wheel and a windmill are different lessons.
    SourceTurned { kind: TurningSource },
    /// A build-along guide was finished — every cell of the laid plan verifies
    /// Correct (`build_guide::verify`).
    CompleteBuildGuide,
    /// A painted avatar skin was saved to the wardrobe and worn.
    SaveSkin,
    /// A rig authored in Rig Studio was spawned into the world.
    SpawnRig,
    /// A `/bug` or `/idea` report was queued for the makers. Local-only — the
    /// report itself is unchanged and nothing about it reaches the scenario.
    /// Native only in practice: the browser build has no `/bug` or `/idea`.
    SendFeedback,
}

/// Which self-driving power source turned, for [`ChallengeEvent::SourceTurned`].
/// Deliberately its OWN two-variant enum rather than `power::PowerDeviceKind`:
/// only these two devices ever turn, so a trial can't author a filter (a Lever,
/// a Battery) that gameplay could never fire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurningSource {
    /// A Water Wheel caught a current.
    WaterWheel,
    /// A Windmill caught the wind.
    Windmill,
}

impl ChallengeEvent {
    /// Does `self` (an objective's pattern) match `incoming` (a fired event)?
    /// Same variant required; a `None` block filter matches any block, a
    /// `Some` filter must equal; `GainMaterial` matches by material.
    pub fn matches(&self, incoming: &ChallengeEvent) -> bool {
        use ChallengeEvent::*;
        match (self, incoming) {
            (BreakBlock { block: pat }, BreakBlock { block: got }) => {
                pat.is_none() || pat == got
            }
            (PlaceBlock { block: pat }, PlaceBlock { block: got }) => {
                pat.is_none() || pat == got
            }
            (GainMaterial { material: a }, GainMaterial { material: b }) => a == b,
            (BreedAnimals { offspring: pat }, BreedAnimals { offspring: got }) => {
                pat.is_none() || pat == got
            }
            (SourceTurned { kind: a }, SourceTurned { kind: b }) => a == b,
            (CraftItem, CraftItem)
            | (CookAtCampfire, CookAtCampfire)
            | (TameMob, TameMob)
            | (VendorSale, VendorSale)
            | (WorkshopPublish, WorkshopPublish)
            | (ClaimPlot, ClaimPlot)
            | (KillMob, KillMob)
            | (EatFood, EatFood)
            | (HarvestCrop, HarvestCrop)
            | (SmeltItem, SmeltItem)
            | (CatchFish, CatchFish)
            | (RideEntity, RideEntity)
            | (PowerDevice, PowerDevice)
            | (UsePiston, UsePiston)
            | (Detonate, Detonate)
            | (UseBucket, UseBucket)
            | (ShearOrMilk, ShearOrMilk)
            | (CompleteBuildGuide, CompleteBuildGuide)
            | (SaveSkin, SaveSkin)
            | (SpawnRig, SpawnRig)
            | (SendFeedback, SendFeedback) => true,
            _ => false,
        }
    }
}

/// What ends a scenario and defines its result.
///
/// NOTE: this enum is intentionally NOT `Copy` — the `Sequence`/`Checklist`
/// variants own a `Vec<Objective>`. Match on `&self.def.objective` (a borrow),
/// never by value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Objective {
    /// Timed score-attack: ends after `ticks` active ticks (Hash Dash).
    Timed { ticks: u32 },
    /// Untimed speedrun: ends when the player mines their first Satori — the
    /// world's Genesis Block (Satori Rush). The clock counts up; lower = better.
    FirstSatori,
    /// No win/lose condition — the experience never ends and shows no objective
    /// HUD. For exploration experiences, where the world itself
    /// (a prebuilt Adventure-mode space) is the whole point.
    FreeRoam,

    /// Count a tagged action `count` times (feature-coverage challenge). Ends
    /// when the count is reached.
    Action { event: ChallengeEvent, count: u32 },
    /// An ORDERED arc (onboarding): each step completes before the next is
    /// counted; ends when the last step completes. v1: each step is an `Action`.
    Sequence { steps: Vec<Objective> },
    /// An UNORDERED checklist (explorer "do A and B"): all items complete, any
    /// order; ends when all are done. v1: each item is an `Action`.
    Checklist { items: Vec<Objective> },
}

impl Objective {
    /// If this is an `Action`, its `(pattern event, target count)`. Used by the
    /// Sequence/Checklist progress logic (v1 leaves are Actions).
    fn as_action(&self) -> Option<(&ChallengeEvent, u32)> {
        if let Objective::Action { event, count } = self {
            Some((event, *count))
        } else {
            None
        }
    }

    /// Number of event-progress slots this objective needs (1 per Action leaf;
    /// 0 for passive Timed/FirstSatori/FreeRoam).
    fn leaf_count(&self) -> usize {
        match self {
            Objective::Action { .. } => 1,
            Objective::Sequence { steps } => steps.len(),
            Objective::Checklist { items } => items.len(),
            _ => 0,
        }
    }
}

/// Index of the first incomplete step in an ordered `Sequence`, given the
/// per-step progress counts. A non-Action leaf can't complete via events, so it
/// blocks the sequence there (returns its index). `None` ⇒ all steps complete.
fn first_incomplete_step(steps: &[Objective], progress: &[u32]) -> Option<usize> {
    steps
        .iter()
        .enumerate()
        .find(|(i, o)| match o.as_action() {
            Some((_, count)) => progress.get(*i).copied().unwrap_or(0) < count,
            None => true,
        })
        .map(|(i, _)| i)
}

fn default_work_exponent() -> f32 {
    1.0
}

/// Optional balance knobs. Kept minimal — playtest owns balance.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScenarioTuning {
    /// Exponent applied to each block's work in *scenario* scoring (NOT the
    /// world's lifetime `total_work`, which stays a raw linear tally). `1.0`
    /// is linear (default); `> 1.0` makes harder blocks worth disproportionately
    /// more so "tooling up to hit harder blocks" out-rates farming easy blocks —
    /// the Hash-Dash leaves-vs-tools lever from the hashing foundations doc.
    #[serde(default = "default_work_exponent")]
    pub work_exponent: f32,
}

impl Default for ScenarioTuning {
    fn default() -> Self {
        Self { work_exponent: default_work_exponent() }
    }
}

fn default_scoring() -> Scoring {
    Scoring::None
}

/// A scenario definition — the declarative mod data the runner interprets.
/// Serde-serializable: this IS the official/community mod artifact format
/// (JSON), delivered via the open-stash (Goal 5). No game is hardcoded into
/// the engine; a def fully describes a challenge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScenarioDef {
    pub kind: ScenarioKind,
    pub display_name: String,
    #[serde(default)]
    pub kit: Vec<KitItem>,
    pub objective: Objective,
    #[serde(default = "default_scoring")]
    pub scoring: Scoring,
    /// Fixed arena seed for a fair, repeatable map. Applied when a FRESH
    /// scenario world is CREATED — the menu-launch flow sets `WorldMeta.seed =
    /// arena_seed` so the new world generates the fixed arena. It is NOT
    /// re-applied by `/scenario` in your *current* world: `chunk_stream::
    /// initial_load` restores an existing save rather than regenerating from a
    /// new seed, so a fixed arena requires a fresh world. `None` = the world's
    /// own terrain (fine for work-scored modes — "any terrain, score is work").
    #[serde(default)]
    pub arena_seed: Option<u32>,
    #[serde(default)]
    pub tuning: ScenarioTuning,
    /// When true, the in-run pause menu HIDES "Switch to Creative". A scenario is
    /// a competition (timed work score / speedrun), and creative mode — flight,
    /// instant-break, infinite blocks — would trivially defeat it. Authorable per
    /// def so a build-style scenario can leave it off. `#[serde(default)]` = off
    /// unless the def opts in; the two official games (Hash Dash, Satori Rush)
    /// set it true.
    #[serde(default)]
    pub lock_creative: bool,
    /// How this experience's world is managed across repeat plays (reuse one
    /// fresh world / resume the take-home run / keep many). `#[serde(default)]` =
    /// [`ArenaMode::Reuse`]. Consumed by the Stash Column launch, not `/scenario`.
    #[serde(default)]
    pub arena_mode: ArenaMode,
    /// Optional world-shape overrides applied to the fresh arena's `WorldMeta`.
    /// `None` keeps the launcher's defaults (a normal/flat survival world — Hash
    /// Dash, Satori Rush leave all of these unset). An exploration def can set them
    /// to make a day-locked, mob-free, Adventure-mode world. Data-driven so any future
    /// experience can describe its own world without engine changes.
    #[serde(default)]
    pub world_type: Option<String>,
    #[serde(default)]
    pub game_mode: Option<String>,
    #[serde(default)]
    pub time_lock: Option<String>,
    /// Wind, Copper & Electricity wave §4 — pin the LOCAL weather window while
    /// this scenario runs: `"clear"` | `"rain"` | `"storm"`. Unlike `time_lock`
    /// (a world-shape field baked into the fresh arena's `WorldMeta`), this is
    /// re-applied every tick from the running scenario, so `/scenario <token>`
    /// in your own world pins the weather too — and it lands BEFORE the tick
    /// samples the wind, which is what makes a storm-locked trial actually turn
    /// a windmill. `None` = the world rolls its own weather. Applied by
    /// `weather::locked_window`; an unrecognised value is ignored at runtime and
    /// fails `trials_lint` at test time.
    #[serde(default)]
    pub weather_lock: Option<String>,
    #[serde(default)]
    pub mobs_enabled: Option<bool>,
    /// Trials (⚡ Race): when set, this scenario is a Race-trial wrapper — the
    /// engine arms the named `trials::CATALOG` race on entry (teleport to start,
    /// plant the finish beacon, start the clock + ghost). The scenario itself is
    /// just the flat arena the race runs in. `None` for every normal challenge.
    #[serde(default)]
    pub trial_race: Option<String>,
    /// Tailored Challenge scaffolding — blocks + mobs planted next to the
    /// player's spawn so a coverage Challenge is completable SOLO in its fresh
    /// arena (a furnace to smelt at, a cow to milk, a water pool to fish). Built
    /// ONLY on the fresh-world lobby launch, never the in-world `/scenario`
    /// overlay (which leaves the player's own world untouched). `None` for
    /// Races / Experiences and any challenge that needs no scaffolding.
    #[serde(default)]
    pub arena: Option<ArenaSetup>,
}

/// Scaffolding for a Challenge's fresh arena — see [`ScenarioDef::arena`].
/// Offsets are relative to the player's spawn position. Data-driven so a
/// community challenge can describe its arena without engine changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ArenaSetup {
    /// Blocks to place: `at` is a `[dx, dy, dz]` offset from spawn, `block` a
    /// `/give`-style block name (resolved via the same registry as kits).
    #[serde(default)]
    pub blocks: Vec<ArenaBlock>,
    /// Mobs to spawn: `at` offset from spawn, `mob` a `/spawn`-style name.
    #[serde(default)]
    pub mobs: Vec<ArenaMob>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArenaBlock {
    pub at: [i32; 3],
    pub block: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArenaMob {
    pub at: [i32; 3],
    pub mob: String,
    #[serde(default = "arena_mob_count_default")]
    pub count: u8,
}

fn arena_mob_count_default() -> u8 {
    1
}

impl ScenarioDef {
    /// Serialise to pretty JSON — the on-disk / open-stash mod artifact.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| format!("serialise ScenarioDef: {e}"))
    }

    /// Whether a world running this scenario should PERSIST it so the run
    /// resumes on reload (the long, resumable runs — Satori Rush). Transient
    /// score-attacks (Hash Dash) are not persisted to the world.
    pub fn is_resumable(&self) -> bool {
        matches!(self.objective, Objective::FirstSatori)
    }
}

/// The live state of a running scenario. Held as `Option<ScenarioState>` on
/// `GameState`; `Some` ⇒ a scenario is active. Ephemeral (not persisted) —
/// the per-world stats it reads/feeds (`total_work`, `genesis_found_at_tick`)
/// live on `WorldMeta` and survive save/load independently.
#[derive(Clone, Debug, PartialEq)]
pub struct ScenarioState {
    def: ScenarioDef,
    /// Active ticks since the scenario started (counts up while playing).
    elapsed_ticks: u32,
    /// Running work score (meaningful when `scoring == Work`).
    score: u64,
    /// Set once the objective completes; the run is over.
    ended: bool,
    /// `elapsed_ticks` at which the run ended (the Satori-Rush result tick, or
    /// the timer length for a timed run). `None` until ended.
    result_tick: Option<u32>,
    /// Per-Action-leaf event counts for the new coverage-challenge objectives
    /// (`Action`/`Sequence`/`Checklist`). One slot per leaf; empty for passive
    /// objectives. Driven by [`ScenarioState::on_event`]. NOT yet persisted —
    /// `resume` rebuilds it at zero (progress-on-reload is a later phase).
    progress: Vec<u32>,
    /// Scavenger only — the set of every item-kind KEY the player has held at any
    /// point this run (see [`crate::inventory::Inventory::item_kind_keys`]). The
    /// `InventoryVariety` score is this set's size, so it only ever grows. Empty
    /// for every other scenario.
    variety_seen: std::collections::HashSet<u64>,
}

impl ScenarioState {
    /// Start a fresh run of `def`.
    pub fn new(def: ScenarioDef) -> Self {
        let progress = vec![0; def.objective.leaf_count()];
        Self {
            def,
            elapsed_ticks: 0,
            score: 0,
            ended: false,
            result_tick: None,
            progress,
            variety_seen: std::collections::HashSet::new(),
        }
    }

    /// Reconstruct a scenario's live state on world reload (Satori-Rush resume).
    /// `elapsed_ticks` is the persisted world-clock (`total_ticks`); `ended` +
    /// `result_tick` come from the world's genesis stats. Score isn't persisted
    /// (the resumable runs are time-measured, not work-scored).
    pub fn resume(
        def: ScenarioDef,
        elapsed_ticks: u32,
        ended: bool,
        result_tick: Option<u32>,
    ) -> Self {
        let progress = vec![0; def.objective.leaf_count()];
        Self {
            def,
            elapsed_ticks,
            score: 0,
            ended,
            result_tick,
            progress,
            variety_seen: std::collections::HashSet::new(),
        }
    }

    /// Per-tick hook (call once per sim tick, after `GameState::tick`). Advances
    /// the active clock and ends the run if a timed objective expires. Returns
    /// `true` on the single tick it transitions to ended, so the caller can fire
    /// the end-screen / unlock-the-result side effects exactly once.
    pub fn tick(&mut self) -> bool {
        if self.ended {
            return false;
        }
        self.elapsed_ticks = self.elapsed_ticks.saturating_add(1);
        if let Objective::Timed { ticks } = &self.def.objective
            && self.elapsed_ticks >= *ticks {
                self.ended = true;
                self.result_tick = Some(self.elapsed_ticks);
                return true;
            }
        false
    }

    /// Break-completion hook. `work` is `crafting::block_work(block)` for a
    /// successful `can_harvest` break, or `0` for non-harvestable / creative /
    /// instant breaks (which do no work). Adds to the score when scoring is `Work`.
    pub fn on_block_broken(&mut self, work: u64) {
        if self.ended || self.def.scoring != Scoring::Work {
            return;
        }
        self.score = self.score.saturating_add(self.weighted_work(work));
    }

    /// Scavenger hook — fold the player's CURRENT item-kind keys (see
    /// [`crate::inventory::Inventory::item_kind_keys`]) into the cumulative
    /// "ever held" set, then set the score to that set's size. Call once per tick
    /// BEFORE [`ScenarioState::tick`]. The score only ever GROWS — using an item
    /// up or crafting it away never lowers it. No-op unless scoring is
    /// `InventoryVariety`; frozen once the run has ended.
    pub fn accumulate_variety(&mut self, current_keys: impl IntoIterator<Item = u64>) {
        if self.ended || self.def.scoring != Scoring::InventoryVariety {
            return;
        }
        self.variety_seen.extend(current_keys);
        self.score = self.variety_seen.len() as u64;
    }

    /// Material-gained hook (call after `inventory.add_item`). For a `FirstSatori`
    /// objective, the first Satori ends the run and records the result tick.
    pub fn on_material_gained(&mut self, material: MaterialId) {
        if self.ended {
            return;
        }
        if matches!(self.def.objective, Objective::FirstSatori) && material == MaterialId::Satori {
            self.ended = true;
            self.result_tick = Some(self.elapsed_ticks);
        }
    }

    /// Coverage-challenge event hook (Phase 3 wires the call sites). Advances
    /// the progress of an `Action`/`Sequence`/`Checklist` objective and ends the
    /// run when it completes. No-op for passive objectives or once ended, and
    /// guarded so it's free when no challenge cares about `ev`.
    pub fn on_event(&mut self, ev: ChallengeEvent) {
        if self.ended {
            return;
        }
        // Mutate progress in a scope that borrows `def` (shared) + `progress`
        // (mutable) as DISJOINT fields, so the borrow ends before the
        // completion check re-borrows `self`.
        {
            let obj = &self.def.objective;
            let progress = &mut self.progress;
            match obj {
                Objective::Action { event, count } => {
                    if event.matches(&ev) && progress[0] < *count {
                        progress[0] += 1;
                    }
                }
                Objective::Sequence { steps } => {
                    // Ordered: only the current (first incomplete) step counts.
                    if let Some(i) = first_incomplete_step(steps, progress)
                        && let Some((event, count)) = steps[i].as_action()
                            && event.matches(&ev) && progress[i] < count {
                                progress[i] += 1;
                            }
                }
                Objective::Checklist { items } => {
                    // Unordered: fill the first incomplete item that matches.
                    for (i, item) in items.iter().enumerate() {
                        if let Some((event, count)) = item.as_action()
                            && progress[i] < count && event.matches(&ev) {
                                progress[i] += 1;
                                break;
                            }
                    }
                }
                _ => return,
            }
        }
        if self.objective_is_complete() {
            self.ended = true;
            self.result_tick = Some(self.elapsed_ticks);
        }
    }

    /// Whether the event-driven objective has reached completion. `false` for
    /// passive objectives (they end via `tick`/`on_material_gained`, not here).
    fn objective_is_complete(&self) -> bool {
        let all_leaves_done = |subs: &[Objective]| -> bool {
            subs.iter().enumerate().all(|(i, o)| match o.as_action() {
                Some((_, count)) => self.progress.get(i).copied().unwrap_or(0) >= count,
                None => false, // a non-Action leaf can't complete via events (v1)
            })
        };
        match &self.def.objective {
            Objective::Action { count, .. } => {
                self.progress.first().copied().unwrap_or(0) >= *count
            }
            Objective::Sequence { steps } => all_leaves_done(steps),
            Objective::Checklist { items } => all_leaves_done(items),
            _ => false,
        }
    }

    /// Progress of an event-driven objective as `(done, total)` for the HUD:
    /// `Action` → `(events so far, count)`; `Sequence`/`Checklist` → `(completed
    /// leaves, total leaves)`. `None` for passive objectives (no progress HUD).
    pub fn objective_progress(&self) -> Option<(u32, u32)> {
        let leaves_done = |subs: &[Objective]| -> u32 {
            subs.iter()
                .enumerate()
                .filter(|(i, o)| match o.as_action() {
                    Some((_, count)) => self.progress.get(*i).copied().unwrap_or(0) >= count,
                    None => false,
                })
                .count() as u32
        };
        match &self.def.objective {
            Objective::Action { count, .. } => {
                Some((self.progress.first().copied().unwrap_or(0).min(*count), *count))
            }
            Objective::Sequence { steps } => Some((leaves_done(steps), steps.len() as u32)),
            Objective::Checklist { items } => Some((leaves_done(items), items.len() as u32)),
            _ => None,
        }
    }

    /// Weighted per-block work under the def's tuning. Linear (identity) by
    /// default; `work_exponent > 1.0` makes hard blocks worth steeper.
    fn weighted_work(&self, work: u64) -> u64 {
        // Clamp the exponent to a sane range so an adversarial / malformed mod
        // def can't award u64::MAX per block (a huge exponent → f64 INFINITY →
        // saturating `as u64` cast). 8.0 keeps even the hardest block finite.
        let e = self.def.tuning.work_exponent.clamp(0.0, 8.0);
        if work == 0 || (e - 1.0).abs() < f32::EPSILON {
            return work;
        }
        (work as f64).powf(e as f64).round() as u64
    }

    pub fn is_ended(&self) -> bool {
        self.ended
    }

    /// Whether ending this scenario blocks gameplay with a modal end-card
    /// (timed score-attacks: "time's up, you're done"). Untimed runs (Satori
    /// Rush) end with a NON-blocking celebration — the player may keep playing.
    pub fn blocks_on_end(&self) -> bool {
        matches!(self.def.objective, Objective::Timed { .. })
    }

    /// True while the modal end-card is on screen (ended AND shows a modal).
    /// Timed score-attacks ("time's up, here's your result") and coverage
    /// **Challenges** — which are launched as discrete Trials and finish with a
    /// "Well done! Back to Trials / Try again" card — both show it. Untimed
    /// free-roam runs (Satori Rush) celebrate NON-blocking and let
    /// the player keep going. The cursor must be RELEASED in this state so the
    /// card's buttons (and any leaderboard opt-in) are clickable, and the click
    /// handler must NOT re-capture the cursor on stray clicks while it holds.
    /// Single source of truth for both the cursor-release and the re-capture
    /// gate, the input gate, and the completion-badge mark.
    pub fn shows_blocking_end_card(&self) -> bool {
        self.is_ended() && (self.blocks_on_end() || self.def.kind == ScenarioKind::Challenge)
    }

    pub fn score(&self) -> u64 {
        self.score
    }

    pub fn elapsed_ticks(&self) -> u32 {
        self.elapsed_ticks
    }

    /// The result tick (Satori-Rush time-to-genesis, or timer length). `None`
    /// until the run ends.
    pub fn result_tick(&self) -> Option<u32> {
        self.result_tick
    }

    pub fn display_name(&self) -> &str {
        &self.def.display_name
    }

    /// Only the test suite reads this now (its former live caller was the web
    /// feedback snapshot, removed 2026-10-01).
    #[cfg(test)]
    pub fn kind(&self) -> ScenarioKind {
        self.def.kind
    }


    pub fn def(&self) -> &ScenarioDef {
        &self.def
    }

    /// `(completed leaves, total leaves)` for the objective — drives the
    /// challenge board's "2 / 3" readout. A leaf is complete when its event
    /// progress reaches the leaf's target count. Passive objectives
    /// (Timed / FirstSatori / FreeRoam) have no leaves and return `(0, 0)`.
    pub fn progress_summary(&self) -> (u32, u32) {
        let required: Vec<u32> = match &self.def.objective {
            Objective::Action { count, .. } => vec![*count],
            Objective::Sequence { steps } => steps
                .iter()
                .map(|s| s.as_action().map(|(_, c)| c).unwrap_or(1))
                .collect(),
            Objective::Checklist { items } => items
                .iter()
                .map(|s| s.as_action().map(|(_, c)| c).unwrap_or(1))
                .collect(),
            _ => Vec::new(),
        };
        let total = required.len() as u32;
        let done = required
            .iter()
            .enumerate()
            .filter(|(i, req)| self.progress.get(*i).copied().unwrap_or(0) >= **req)
            .count() as u32;
        (done, total)
    }

    /// Per-step `(done, required)` for the objective's leaves — one entry per
    /// task line (Action → one). Drives the in-game checklist + the badge's
    /// current-step counter. Empty for timed / free-roam / Satori objectives.
    pub fn step_status(&self) -> Vec<(u32, u32)> {
        let required: Vec<u32> = match &self.def.objective {
            Objective::Action { count, .. } => vec![*count],
            Objective::Sequence { steps } => steps
                .iter()
                .map(|s| s.as_action().map(|(_, c)| c).unwrap_or(1))
                .collect(),
            Objective::Checklist { items } => items
                .iter()
                .map(|s| s.as_action().map(|(_, c)| c).unwrap_or(1))
                .collect(),
            _ => Vec::new(),
        };
        required
            .iter()
            .enumerate()
            .map(|(i, req)| (self.progress.get(i).copied().unwrap_or(0).min(*req), *req))
            .collect()
    }

    /// Index of the current (first incomplete) step, or `None` when every step
    /// is done / the objective has no steps. For an ordered Sequence this is the
    /// step you're working on; for a Checklist it's just the first unfinished one.
    pub fn current_step(&self) -> Option<usize> {
        self.step_status().iter().position(|(done, req)| done < req)
    }

    /// Remaining ticks for a timed objective (`0` for untimed or ended).
    pub fn remaining_ticks(&self) -> u32 {
        match &self.def.objective {
            Objective::Timed { ticks } => ticks.saturating_sub(self.elapsed_ticks),
            // Untimed / event-driven objectives have no countdown.
            Objective::FirstSatori
            | Objective::FreeRoam
            | Objective::Action { .. }
            | Objective::Sequence { .. }
            | Objective::Checklist { .. } => 0,
        }
    }
}

/// Parse a [`ScenarioDef`] from JSON bytes — a local file or open-stash blob.
/// Keeps the SOURCE abstract: Goal 5 feeds bytes downloaded from the AxeNStax
/// open-stash; tests feed a literal. No game is hardcoded here.
pub fn load_scenario_def(bytes: &[u8]) -> Result<ScenarioDef, String> {
    serde_json::from_slice(bytes).map_err(|e| format!("invalid ScenarioDef JSON: {e}"))
}

/// Current scenario Beacon blob format version. Mirrors `OVERRIDE_SET_VERSION`:
/// a 1-byte tag prefixes the payload so an older client can DETECT + REJECT a
/// newer format rather than silently misparsing it. The scenario payload is
/// **JSON** (a `ScenarioDef`), not bincode — defs are small, human-auditable,
/// and JSON is already the canonical mod-artifact format (see `to_json`). This
/// is the second Beacon content type (the Stash Column, Prague delivery); the
/// first is the Workshop `override-set` (see `override_registry::to_blob_bytes`).
// `SCENARIO_BLOB_VERSION` and `scenario_to_blob_bytes` (the publish side) are
// exercised by round-trip tests below; only `scenario_from_blob_bytes` (the
// consume side) has a live caller so far (menu.rs's wasm32-only Stash-download
// drain). Neither condition alone is visible to a plain native clippy run.
#[cfg_attr(not(test), allow(dead_code))]
pub const SCENARIO_BLOB_VERSION: u8 = 1;

/// Serialise a [`ScenarioDef`] to the bytes published as a Beacon `scenario`
/// blob: a 1-byte version tag followed by the def's JSON. The version byte is the
/// permanent forward-compat guard. Counterpart to `OverrideSet::to_blob_bytes`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn scenario_to_blob_bytes(def: &ScenarioDef) -> Result<Vec<u8>, String> {
    let mut blob = vec![SCENARIO_BLOB_VERSION];
    blob.extend_from_slice(def.to_json()?.as_bytes());
    Ok(blob)
}

/// Parse a downloaded Beacon `scenario` blob. Rejects an empty blob, a
/// newer-than-supported version, and malformed JSON — never panics (mirrors
/// `OverrideSet::from_blob_bytes`). Live caller: menu.rs's wasm32-only Stash
/// download drain (`draw_stash_column`); also round-trip tested below.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub fn scenario_from_blob_bytes(bytes: &[u8]) -> Result<ScenarioDef, String> {
    let (&ver, rest) = bytes
        .split_first()
        .ok_or_else(|| "empty scenario blob".to_string())?;
    if ver > SCENARIO_BLOB_VERSION {
        return Err(format!(
            "scenario v{ver} is newer than supported v{SCENARIO_BLOB_VERSION} — update the game"
        ));
    }
    load_scenario_def(rest)
}

/// A tiny built-in scenario for the runner's own coverage + `/scenario test`:
/// a 5-second (100-tick) timed work score-attack with a basic kit. The real
/// games arrive as data (Goals 3/4), not as more arms here.
pub fn builtin_test_def() -> ScenarioDef {
    ScenarioDef {
        kind: ScenarioKind::Test,
        display_name: "Test Scenario".to_string(),
        lock_creative: false,
        arena_mode: ArenaMode::Reuse,
        kit: vec![
            KitItem { name: "wooden_pickaxe".to_string(), count: 1 },
            KitItem { name: "stone".to_string(), count: 4 },
        ],
        objective: Objective::Timed { ticks: 100 },
        scoring: Scoring::Work,
        arena_seed: None,
        tuning: ScenarioTuning::default(),
        world_type: None,
        game_mode: None,
        time_lock: None,
        weather_lock: None,
        mobs_enabled: None,
        trial_race: None,
        arena: None,
    }
}

/// Hash Dash — the first official challenge mod (Goal 3): a timed WORK
/// score-attack. DATA, not code: the JSON artifact is the source of truth (the
/// same file Goal 5 publishes to the AxeNStax open-stash); this embeds + parses
/// it so `/scenario hash-dash` works offline / before any live fetch.
pub const HASH_DASH_JSON: &[u8] = include_bytes!("../assets/scenarios/hash-dash.json");

/// Parse the embedded Hash Dash def. Parsing the bundled artifact is a
/// build-time invariant (covered by a test), so the `expect` is sound.
pub fn hash_dash_def() -> ScenarioDef {
    load_scenario_def(HASH_DASH_JSON).expect("bundled hash-dash.json must parse")
}

/// Satori Rush — the second official mod (Goal 4): an untimed Genesis-Block
/// speedrun (mine your FIRST Satori; lower world-clock time = better). DATA
/// artifact (open-stash-published by Goal 5); embedded here so `/scenario
/// satori-rush` works offline. RESUMABLE — it's the player's own world (see
/// WorldMeta.scenario_def + ScenarioState::resume).
pub const SATORI_RUSH_JSON: &[u8] = include_bytes!("../assets/scenarios/satori-rush.json");

/// Parse the embedded Satori Rush def (build-time invariant; covered by a test).
pub fn satori_rush_def() -> ScenarioDef {
    load_scenario_def(SATORI_RUSH_JSON).expect("bundled satori-rush.json must parse")
}

// ─── Feature-coverage challenge pack (Phase 4) ───────────────────────────────
// Authored as DATA (JSON ScenarioDefs), embedded like the games above so they
// work offline and can also ride the open-stash as community packs later. Each
// (name, bytes) pair is resolvable via `named_builtin_def` + listed by
// `challenge_pack` for the (Phase 6) challenge board.

/// The onboarding arc — an ORDERED first-session sequence (break → craft → place
/// → cook → mine coal). Fun-first; coverage is a side effect.
pub const ONBOARDING_JSON: &[u8] = include_bytes!("../assets/scenarios/onboarding.json");

/// Explorer set — unordered single-feature cards, each mapped to a shipped
/// feature so a tester can be steered at exactly the surface we want data on.
// Fun-first redesign (2026-06-24): every card is a mini-adventure with the right
// kit + a small pre-built arena, ordered/checklist objectives that tell a little
// story, Satoshi voice (see `trial_satoshi`), and coverage of every game system —
// movement (the ⚡ races), mining, crafting, smelting, cooking, electricity,
// logic/sensors, pistons, rails+carts, booby traps, combat, armour, farming,
// animals (breed/shear/milk/tame), fishing, riding, buckets, hunger, explosives,
// plots, vendor trade, and the Workshop. Design + coverage matrix:
// `docs/foundations/2026-06-24-trials-fun-redesign.md`.
pub const EXPLORER_CHALLENGES: &[(&str, &[u8])] = &[
    ("mine", include_bytes!("../assets/scenarios/explorer-mine.json")),
    ("craft", include_bytes!("../assets/scenarios/explorer-craft.json")),
    ("smelt", include_bytes!("../assets/scenarios/explorer-smelt.json")),
    ("cook-three", include_bytes!("../assets/scenarios/explorer-cook-three.json")),
    ("build", include_bytes!("../assets/scenarios/explorer-build.json")),
    ("power", include_bytes!("../assets/scenarios/explorer-power.json")),
    ("logic-gate", include_bytes!("../assets/scenarios/explorer-logic-gate.json")),
    ("piston", include_bytes!("../assets/scenarios/explorer-piston.json")),
    ("rail-rider", include_bytes!("../assets/scenarios/explorer-rail-rider.json")),
    ("booby-trap", include_bytes!("../assets/scenarios/explorer-booby-trap.json")),
    ("bucket", include_bytes!("../assets/scenarios/explorer-bucket.json")),
    ("workshop-publish", include_bytes!("../assets/scenarios/explorer-workshop-publish.json")),
    ("kill", include_bytes!("../assets/scenarios/explorer-kill.json")),
    ("armour-up", include_bytes!("../assets/scenarios/explorer-armour-up.json")),
    ("breach-and-clear", include_bytes!("../assets/scenarios/explorer-breach-and-clear.json")),
    ("eat", include_bytes!("../assets/scenarios/explorer-eat.json")),
    ("harvest", include_bytes!("../assets/scenarios/explorer-harvest.json")),
    ("farmhand", include_bytes!("../assets/scenarios/explorer-farmhand.json")),
    ("rancher", include_bytes!("../assets/scenarios/explorer-rancher.json")),
    ("mule-maker", include_bytes!("../assets/scenarios/explorer-mule-maker.json")),
    ("tame-wolf", include_bytes!("../assets/scenarios/explorer-tame-wolf.json")),
    ("friend-in-need", include_bytes!("../assets/scenarios/explorer-friend-in-need.json")),
    ("fish", include_bytes!("../assets/scenarios/explorer-fish.json")),
    ("ride", include_bytes!("../assets/scenarios/explorer-ride.json")),
    ("fish-feast", include_bytes!("../assets/scenarios/explorer-fish-feast.json")),
    ("vendor-sale", include_bytes!("../assets/scenarios/explorer-vendor-sale.json")),
    ("claim-plot", include_bytes!("../assets/scenarios/explorer-claim-plot.json")),
    ("scavenger", include_bytes!("../assets/scenarios/explorer-scavenger.json")),
    ("roadrunner", include_bytes!("../assets/scenarios/explorer-roadrunner.json")),
    ("lava-floor", include_bytes!("../assets/scenarios/explorer-lava-floor.json")),
    ("kaboomtown", include_bytes!("../assets/scenarios/explorer-kaboomtown.json")),
    ("monster-mash", include_bytes!("../assets/scenarios/explorer-monster-mash.json")),
    ("funny-farm", include_bytes!("../assets/scenarios/explorer-funny-farm.json")),
    ("splash-zone", include_bytes!("../assets/scenarios/explorer-splash-zone.json")),
    ("champion", include_bytes!("../assets/scenarios/explorer-champion.json")),
    ("generator", include_bytes!("../assets/scenarios/explorer-generator.json")),
    ("obsidian", include_bytes!("../assets/scenarios/explorer-obsidian.json")),
    ("dye", include_bytes!("../assets/scenarios/explorer-dye.json")),
    // Wind, Copper & Electricity wave (2026-09-07) §4 — coverage for the wave's
    // new ground (copper from the rock, the two self-driving power sources) plus
    // the solo-queue features that had no trial at all (build-along, the skin
    // painter, Rig Studio, the lobby mailbox).
    ("copper-rush", include_bytes!("../assets/scenarios/explorer-copper-rush.json")),
    ("mill-race", include_bytes!("../assets/scenarios/explorer-mill-race.json")),
    ("catch-the-wind", include_bytes!("../assets/scenarios/explorer-catch-the-wind.json")),
    ("follow-the-plan", include_bytes!("../assets/scenarios/explorer-follow-the-plan.json")),
    ("fresh-coat", include_bytes!("../assets/scenarios/explorer-fresh-coat.json")),
    ("bouncer", include_bytes!("../assets/scenarios/explorer-bouncer.json")),
    // Native only: its objective (`SendFeedback`) fires from `/bug`·`/idea`, which
    // the browser build does not have (no feedback channel on web, 2026-10-01).
    #[cfg(not(target_arch = "wasm32"))]
    ("suggestion-box", include_bytes!("../assets/scenarios/explorer-suggestion-box.json")),
];

/// `(start-name, display-name)` pairs for the feature-coverage challenges — the
/// discoverable list behind `/scenario list` (and what the Phase-6 board will
/// render). The start-name is the exact token `named_builtin_def` accepts, so
/// `/scenario <start-name>` launches it.
pub fn challenge_listing() -> Vec<(&'static str, String)> {
    let mut out = vec![(
        "onboarding",
        load_scenario_def(ONBOARDING_JSON)
            .map(|d| d.display_name)
            .unwrap_or_else(|_| "Onboarding".to_string()),
    )];
    for (name, bytes) in EXPLORER_CHALLENGES {
        if let Ok(def) = load_scenario_def(bytes) {
            out.push((name, def.display_name));
        }
    }
    out
}

/// The one challenge whose objective needs `/idea` (`SendFeedback`). It is part
/// of the alpha-tester feedback feature, so it is hidden — from the lobby, the
/// J board, `/trial list` and `/scenario list`, and not startable by name —
/// whenever `native_mailbox::feedback_enabled` is false. Native only (the web
/// build does not even bundle it).
pub const FEEDBACK_TRIAL: &str = "suggestion-box";

/// [`challenge_listing`] minus the feedback trial unless `feedback_on`. Every
/// player-facing listing goes through this; the unfiltered
/// [`challenge_listing`] stays for tests that must cover every bundled def.
pub fn challenge_listing_visible(feedback_on: bool) -> Vec<(&'static str, String)> {
    let mut out = challenge_listing();
    if !feedback_on {
        out.retain(|(name, _)| *name != FEEDBACK_TRIAL);
    }
    out
}

fn plural_s(n: u32) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// A friendly, control-FREE task line for one objective event — the building
/// block of the "What to do" task list. Says WHAT to do, never HOW (the specific
/// controls live in the in-game objective panel).
pub fn event_task_label(event: &ChallengeEvent, count: u32) -> String {
    let n = count.max(1);
    match event {
        ChallengeEvent::BreakBlock { .. } => format!("Break {n} block{}", plural_s(n)),
        ChallengeEvent::PlaceBlock { .. } => format!("Place {n} block{}", plural_s(n)),
        ChallengeEvent::CraftItem => {
            if n == 1 { "Craft an item".to_string() } else { format!("Craft {n} items") }
        }
        ChallengeEvent::CookAtCampfire => {
            if n == 1 { "Cook food at a campfire".to_string() } else { format!("Cook {n} foods at a campfire") }
        }
        ChallengeEvent::TameMob => "Tame an animal".to_string(),
        ChallengeEvent::VendorSale => "Trade something with the vendor".to_string(),
        ChallengeEvent::WorkshopPublish => "Publish your Workshop design".to_string(),
        ChallengeEvent::ClaimPlot => "Claim a plot of land".to_string(),
        ChallengeEvent::GainMaterial { .. } => "Collect the special item".to_string(),
        ChallengeEvent::KillMob => {
            if n == 1 { "Defeat an enemy".to_string() } else { format!("Defeat {n} enemies") }
        }
        ChallengeEvent::EatFood => {
            if n == 1 { "Eat some food".to_string() } else { format!("Eat food {n} times") }
        }
        ChallengeEvent::HarvestCrop => format!("Harvest {n} crop{}", plural_s(n)),
        ChallengeEvent::SmeltItem => {
            if n == 1 { "Smelt an item in a furnace".to_string() } else { format!("Smelt {n} items in a furnace") }
        }
        ChallengeEvent::CatchFish => format!("Catch {n} fish"),
        ChallengeEvent::RideEntity => "Ride an animal or cart".to_string(),
        ChallengeEvent::PowerDevice => "Power up the device".to_string(),
        ChallengeEvent::UsePiston => "Trigger the piston".to_string(),
        ChallengeEvent::Detonate => "Set off the explosion".to_string(),
        ChallengeEvent::UseBucket => {
            if n == 1 { "Use a bucket".to_string() } else { format!("Use a bucket {n} times") }
        }
        ChallengeEvent::ShearOrMilk => {
            if n == 1 { "Shear a sheep or milk a cow".to_string() } else { format!("Shear or milk {n} times") }
        }
        ChallengeEvent::BreedAnimals { .. } => "Breed a pair of animals".to_string(),
        ChallengeEvent::SourceTurned { kind } => match kind {
            TurningSource::WaterWheel => "Get the water wheel turning".to_string(),
            TurningSource::Windmill => "Get the windmill turning".to_string(),
        },
        ChallengeEvent::CompleteBuildGuide => "Finish the build-along guide".to_string(),
        ChallengeEvent::SaveSkin => "Save your painted skin and wear it".to_string(),
        ChallengeEvent::SpawnRig => "Spawn your rig".to_string(),
        ChallengeEvent::SendFeedback => "Send the makers a note".to_string(),
    }
}

/// The "What to do" task list for a trial — derived straight from its objective
/// so it always matches what actually completes the trial, and is control-free.
/// `ordered` → render as a numbered list (the steps must be done in order);
/// otherwise bullets (any order).
pub struct TaskList {
    pub ordered: bool,
    pub lines: Vec<String>,
}

/// Auto-derived [`TaskList`] from a scenario's objective events (the generic
/// fallback: "Break 3 blocks", "Craft an item").
fn objective_tasks_auto(def: &ScenarioDef) -> TaskList {
    let leaves = |subs: &[Objective]| -> Vec<String> {
        subs.iter()
            .filter_map(|o| o.as_action().map(|(e, c)| event_task_label(e, c)))
            .collect()
    };
    match &def.objective {
        Objective::Action { event, count } => TaskList {
            ordered: false,
            lines: vec![event_task_label(event, *count)],
        },
        Objective::Sequence { steps } => TaskList { ordered: true, lines: leaves(steps) },
        Objective::Checklist { items } => TaskList { ordered: false, lines: leaves(items) },
        Objective::Timed { .. } => TaskList {
            ordered: false,
            lines: vec!["Score as many points as you can before the timer runs out.".to_string()],
        },
        Objective::FirstSatori => TaskList {
            ordered: false,
            lines: vec!["Mine the Genesis Block as fast as you can.".to_string()],
        },
        Objective::FreeRoam => TaskList {
            ordered: false,
            lines: vec!["Explore freely — there's no set goal.".to_string()],
        },
    }
}

/// The [`TaskList`] for a scenario, preferring authored per-step labels (clear,
/// specific, fun) over the generic auto-derived ones. `authored` must line up
/// 1:1 with the auto-derived lines (one label per objective step) — guarded by
/// `authored_task_labels_align`. Pass `&[]` for pure auto-derive.
pub fn objective_tasks(def: &ScenarioDef, authored: &[&str]) -> TaskList {
    let mut tl = objective_tasks_auto(def);
    if !authored.is_empty() && authored.len() == tl.lines.len() {
        tl.lines = authored.iter().map(|s| s.to_string()).collect();
    }
    tl
}

/// Authored, player-facing task labels — one clear label per objective step, in
/// order. Says exactly what to DO (chop the tree, build a workbench) where the
/// generic events ("Break 3 blocks") would be opaque. `&[]` = auto-derive.
/// `authored_task_labels_align` (test) guards that the count matches the
/// objective's step count for every trial.
pub fn trial_task_labels(token: &str) -> &'static [&'static str] {
    match token {
        "onboarding" => &["Dig up 4 blocks", "Craft your first item", "Place 6 blocks to build a shelter", "Cook a meal on the campfire"],
        "mine" => &["Mine 20 blocks to open up your quarry"],
        "craft" => &["Chop down the oak tree (3 logs)", "Craft wooden planks", "Build a workbench and place it down", "Make a wooden pickaxe and a sword"],
        "smelt" => &["Place the furnace", "Smelt raw iron into a shining ingot", "Forge an iron pickaxe"],
        "cook-three" => &["Cook 3 meals over the campfire"],
        "build" => &["Place 30 blocks — raise a house with walls, a door and a window"],
        "power" => &["Lay 4 cables from the lever toward the lamp", "Place the lever", "Flip the lever to light the lamp"],
        "logic-gate" => &["Place the motion sensor", "Wire 3 cables through the logic gate", "Make it trip the lamp automatically"],
        "piston" => &["Place the sticky piston against the wall", "Run 2 cables from the lever", "Flip the lever to slide the door open"],
        "rail-rider" => &["Lay 3 rail tracks in a line", "Drop a cart on and ride it"],
        "booby-trap" => &["Place the blasting keg behind the wall", "Wire the pressure plate to it (2 cables)", "Step on the plate — boom!"],
        "bucket" => &["Scoop water from the pond with your bucket", "Pour it into the dry trough"],
        "workshop-publish" => &["Design a reskin in the Workshop", "Publish it for the whole world"],
        "kill" => &["Defeat 3 raiders at the gate"],
        "armour-up" => &["Smelt iron in the furnace", "Forge 4 pieces of iron armour", "Defeat 2 raiders"],
        "breach-and-clear" => &["Blow open the stone wall", "Defeat the raider hiding behind it"],
        "eat" => &["Cook a meal on the campfire", "Eat your fill"],
        "harvest" => &["Harvest 4 grown wheat"],
        "farmhand" => &["Shear a sheep or milk a cow — 2 chores in all"],
        "rancher" => &["Feed two cows wheat until a calf is born"],
        "mule-maker" => &["Feed a horse and a donkey wheat until a mule foal is born"],
        "tame-wolf" => &["Tame a wild wolf with bones"],
        "friend-in-need" => &["Tame a cat, parrot or fox — your pick"],
        "fish" => &["Catch 2 fish from the pond"],
        "ride" => &["Climb aboard a horse and ride"],
        "fish-feast" => &["Catch a fish from the pond", "Cook it on the campfire", "Eat your fresh meal"],
        "vendor-sale" => &["Trade your spare stone with the vendor"],
        "claim-plot" => &["Plant a claim marker", "Place a block on your new land"],
        "scavenger" => &["Grab as many DIFFERENT things as you can before time runs out"],
        "roadrunner" => &["Saddle up a Nostrich and take her for a ride"],
        "lava-floor" => &["Place 20 blocks to bridge across the lava sea"],
        "kaboomtown" => &["Blow the whole tower sky-high"],
        "monster-mash" => &["Defeat all 6 monsters in the pit"],
        "funny-farm" => &["Tame an animal", "Breed a pair of animals", "Ride an animal", "Shear a sheep or milk a cow"],
        "splash-zone" => &["Use your bucket 5 times to build a water park"],
        "champion" => &["Defeat the valley's fiercest raider"],
        "generator" => &["Place 3 generator blocks", "Switch on your home-made power"],
        "obsidian" => &["Pour lava beside water, then mine the obsidian you made"],
        "dye" => &["Mix and craft 3 dyes"],
        "copper-rush" => &["Mine 3 copper ore out of the rock face", "Smelt copper ore into a copper ingot", "Craft your first length of cable"],
        "mill-race" => &["Place the water wheel in the falling water", "Get the wheel turning in the current", "Wire it up and light the lamp"],
        "catch-the-wind" => &["Place the windmill on the hilltop platform", "Let the gale get the sails turning", "Wire it up and light the lamp"],
        "follow-the-plan" => &["Follow the ghost guide until the hut is finished"],
        "fresh-coat" => &["Paint the mannequin, then pin it on to wear it"],
        "bouncer" => &["Build a rig, give it the Bounce clip, and spawn it"],
        "suggestion-box" => &["Send the makers one idea"],
        _ => &[],
    }
}

/// The authored [`TaskList`] for a challenge token (auto-derive fallback).
pub fn trial_tasks_for_token(token: &str) -> Option<TaskList> {
    let def = named_builtin_def(token)?;
    Some(objective_tasks(&def, trial_task_labels(token)))
}

/// The authored [`TaskList`] for the active scenario, looked up by display name
/// (the runtime holds a [`ScenarioDef`], not its token).
pub fn trial_tasks_for_display(display: &str) -> Option<TaskList> {
    for (token, d) in challenge_listing() {
        if d == display {
            return trial_tasks_for_token(token);
        }
    }
    None
}

/// Items a crafting trial wants you to make — their recipes auto-appear in the
/// inventory's right-side card during the trial (no recipe book needed). Names
/// are `/give`-style tokens resolved via `give::resolve_item`; empty = no hint.
/// `trial_recipe_hints_resolve` (test) guards that every name maps to a real
/// catalogue recipe.
pub fn trial_recipe_hints(token: &str) -> &'static [&'static str] {
    match token {
        "onboarding" => &["crafting_table"],
        "craft" => &["crafting_table", "wooden_pickaxe", "wooden_sword"],
        "smelt" => &["iron_pickaxe"],
        "copper-rush" => &["cable"],
        _ => &[],
    }
}

/// Recipe hints for the active scenario, found by its display name (the runtime
/// holds a [`ScenarioDef`], not its challenge token). Reverse-maps the display
/// name → token via [`challenge_listing`], then defers to [`trial_recipe_hints`].
pub fn trial_recipe_hints_for_display(display_name: &str) -> &'static [&'static str] {
    for (token, display) in challenge_listing() {
        if display == display_name {
            return trial_recipe_hints(token);
        }
    }
    &[]
}

/// Player-facing help for a bundled Challenge, keyed by its `named_builtin_def`
/// token: `(tagline, how_to)`. The Trials dropdown shows the one-line `tagline`;
/// the info (ⓘ) pop-up shows the detailed `how_to`. Authored prose (kept here
/// rather than in the JSON so the def format stays minimal). A token with no
/// entry returns `("", "")`, and the UI falls back to the display-name premise.
pub fn challenge_help(name: &str) -> (&'static str, &'static str) {
    match name {
        "mine" => (
            "Swing a pickaxe and break 20 blocks — carve out your first little mine.",
            "Hold your pickaxe (press 1) and LEFT-CLICK-and-hold on the stone in front of you to break it. Each block that pops is one closer — break 20 in all. Dig straight into the hillside and watch a tunnel open up behind you.",
        ),
        "craft" => (
            "Chop a log, build a crafting table, and forge a pickaxe + sword from scratch.",
            "Walk to the oak tree and LEFT-CLICK the logs to chop them. Open your inventory (E) and turn logs into planks, then planks into sticks. Place a crafting table on the ground with RIGHT-CLICK, then RIGHT-CLICK it to open the big grid. Make a wooden pickaxe and a wooden sword. Every time something new pops into your inventory, that's one craft done!",
        ),
        "smelt" => (
            "Feed a furnace coal + raw iron, smelt an ingot, then hammer it into your first iron tool.",
            "Place the furnace with RIGHT-CLICK, then RIGHT-CLICK it to open it. Drop coal in the bottom fuel slot and raw iron in the top. Watch the flames — when a shiny iron ingot pops out, pull it into your inventory. Then take that ingot to the crafting table and forge an iron pickaxe. Real metalwork!",
        ),
        "cook-three" => (
            "Light a campfire and cook three pieces of food till they're golden.",
            "Place your campfire (right-click), drop in the sticks as fuel and light it with the flint & steel (right-click the fire). Now put raw meat on it and wait for it to brown — take 3 cooked pieces off to finish. Smells good already!",
        ),
        "build" => (
            "Free-build a tiny house: four walls, a glass window, a door, and a sign out front.",
            "You've got a stack of blocks, glass, a door, and a sign. Fly around with creative mode and RIGHT-CLICK to place blocks. Build four walls about three high on the marked-out floor, leave a gap for the door and pop it in, frame a glass window, and hang a sign by the entrance. Make it yours — there's no single right shape!",
        ),
        "power" => (
            "Run cable from a lever to an electric lamp and flip the switch — let there be light!",
            "There's a lamp on the far wall and a lever near you. Lay copper cable in a line RIGHT-CLICK by RIGHT-CLICK to connect the lever to the lamp. When the wire path is complete, RIGHT-CLICK the lever to flip it — the lamp blinks ON. That's real circuit-building: a switch, a wire, and a light. (Out in a normal world the copper for that cable comes straight out of the ground — copper ore sits in the mid-depth stone, and the Copper Rush trial shows you how to dig it.)",
        ),
        "logic-gate" => (
            "Build an automatic alarm: motion sensor, logic gate, cable, lamp. Walk past and it trips!",
            "This is a circuit with a brain. Place the motion sensor facing the doorway, wire it through the logic gate with cable, then run cable on to the electric lamp. Step into the sensor's view and — no lever needed — the gate passes the signal and the lamp flashes ON by itself. That's an automatic machine you designed. (Every gate and every cable here is copper, and copper now comes straight out of the ground — dig it in Copper Rush.)",
        ),
        "piston" => (
            "Wire a lever to a piston so a block in the wall slides aside — your own secret doorway.",
            "There's a stone wall blocking the passage with one gap for a piston. Place the sticky piston so it pushes a wall block, then run cable from a lever to the piston. RIGHT-CLICK the lever — the piston shoves the block aside and the wall opens like a hidden door. Flip it back and it closes. Secret-base technology!",
        ),
        "rail-rider" => (
            "Lay down rail, set a minecart on the line, hop in, and ride the railway you just built.",
            "Build a railway! RIGHT-CLICK to lay rail blocks in a line along the ground — make it as long as you like. Then place a cart on the track and RIGHT-CLICK it to climb aboard. Give yourself a push and roll down the line you laid. You're the engineer AND the driver.",
        ),
        "booby-trap" => (
            "Wire a pressure plate to a blasting keg, then step on the plate and trigger your own KABOOM.",
            "Time to build a trap. Place the blasting keg behind the wall, set a pressure plate on the floor in front, and run cable between them so the plate is wired to the keg. Then walk onto the plate yourself — CLICK — the keg goes off in a satisfying explosion. You've built a real booby trap. Stand clear!",
        ),
        "bucket" => (
            "Scoop water, walk it over, and fill the dry trough.",
            "Hold the empty bucket and right-click on the blue water source to scoop it up — your bucket fills. Walk to the empty stone trough on the right and right-click into it to pour the water back out. Refill once more and pour it into the second slot. Watch the trough fill up where it was dry before!",
        ),
        "workshop-publish" => (
            "Make a drafting stamp, then publish your reskin to share.",
            "This is a maker's trial! At the crafting table, craft a drafting stamp from your blueprint paper and plan — that's your design tool. Then open chat with T and use the workshop publish command (type /help if you forget it) to publish your reskin so other players can use your creation. You're a published creator now!",
        ),
        "kill" => (
            "Grab the sword, hold the gate, and drive off three raiders.",
            "You spawn behind a little stone wall with a gate. A stone_sword is in your hotbar — press 1 to hold it. Three brigands rush in from the front. Face them and LEFT-CLICK to swing. Keep hitting each one until it falls, then turn to the next. Back up against the wall if they crowd you.",
        ),
        "armour-up" => (
            "Smelt iron, forge four pieces of gear at the bench, then fight the raiders in style.",
            "First make metal: put raw iron and coal in the furnace and take the iron INGOTS out (that's a smelt). At the crafting table, forge FOUR pieces of iron gear — armour if you like, or iron tools — one craft at a time. Then hold your iron sword (press 1) and LEFT-CLICK the two raiders until they fall. Do it in any order you like.",
        ),
        "breach-and-clear" => (
            "Plant a blasting keg by the wall, spark it, then storm through the rubble.",
            "A thick stone wall blocks your path with a raider waiting behind it. Right-click to PLACE the blasting_keg right against the wall, then hold your flint_and_steel (press 2) and right-click the keg to light it — stand back, BOOM! Walk through the hole and LEFT-CLICK the raider with your sword.",
        ),
        "eat" => (
            "Cook raw meat on the campfire, then eat it to fill your hunger back up.",
            "Light the campfire with the flint & steel (it burns the sticks for fuel), lay raw beef on it and wait, then take the cooked food OFF. Now hold the cooked meat (press 1) and HOLD right-click to eat it. If nothing happens you're already full — sprint and jump around for a moment to work up an appetite, then eat. A good meal, made by you.",
        ),
        "harvest" => (
            "Till the soil, plant seeds, sprinkle bone meal, reap the gold.",
            "Hold the hoe and right-click the brown dirt in front of you to till it into soil. Then hold wheat seeds and right-click the tilled soil to plant them. Sprinkle bone meal (right-click the little sprouts) to make them shoot up fast, and when the wheat turns tall and golden, left-click to harvest it. Bring in 4 mature crops to finish.",
        ),
        "farmhand" => (
            "Shear a sheep AND milk a cow — two chores, any order.",
            "Walk up to the fuzzy sheep, hold the shears and right-click it — its wool pops right off. Then walk to the cow, hold the empty bucket and right-click it to fill the bucket with milk. Do both, in any order you like, and the chores are done.",
        ),
        "rancher" => (
            "Feed two cows wheat so they breed a wobbly little calf.",
            "Hold the wheat and right-click ONE cow to feed it — little hearts will float above its head. Then right-click the OTHER cow to feed it too. When both are in 'love mode' they'll pair up and a tiny calf will pop out between them. Feed both cows to make it happen!",
        ),
        "mule-maker" => (
            "Feed a horse and a donkey wheat until they breed a sturdy mule foal.",
            "A mule only comes from ONE pairing in the whole animal kingdom here: a horse AND a donkey. Hold the wheat, CROUCH, and right-click the horse to feed it — hearts will float above its head. CROUCH and right-click the donkey with wheat too. When both are in 'love mode' they'll pair up and a mule foal appears between them. Any other pairing just makes more of the same animal — it takes the cross to get a mule.",
        ),
        "tame-wolf" => (
            "Feed a wild wolf bones until it becomes your loyal pet.",
            "Walk up to the wolf, hold a bone, and right-click it to give the bone. Keep right-clicking with bones — it may take a few — and watch for the moment hearts burst out and a collar appears. That's your new best friend, tamed and yours.",
        ),
        "friend-in-need" => (
            "Win over a cat, parrot or fox with its favourite food — any one of them counts.",
            "Three shy companions are waiting nearby: a cat, a parrot and a fox. Cats love raw fish, parrots love wheat seeds, and foxes love berries — hold the right food and right-click the animal you fancy. It may take a couple of tries, so keep offering. The moment hearts burst out, that animal is yours for good — you only need ONE new friend to finish.",
        ),
        "fish" => (
            "Cast your rod into the pond and reel in a fish.",
            "Stand on the stone dock facing the water, hold the fishing rod and right-click to cast your line out onto the pond. Now wait — when the bobber dips and you see splashes, right-click again FAST to yank the fish in. Land 2 fish to finish.",
        ),
        "ride" => (
            "Hop on the horse and take your first ride.",
            "Walk right up to the horse and right-click it to climb on. Once you're in the saddle, use your normal walk keys to trot it forward — give it a little ride around the paddock. Mounting up is all it takes to finish.",
        ),
        "fish-feast" => (
            "Catch a fish, cook it on the fire, then eat your own catch.",
            "First, hold the rod and cast onto the pond with right-click, then snap it back when the bobber dips to land a fish. Carry your catch to the campfire and right-click the fire to set it cooking — wait for it to brown, then right-click again to take the cooked fish off. Finally, hold the cooked fish and hold right-click to eat it. Catch, cook, eat — a full meal, start to finish! If nothing happens when you eat, you're already full — sprint and jump around for a moment to work up an appetite, then tuck in.",
        ),
        "vendor-sale" => (
            "Trade your stone for warm bread at the village stall — a fair barter.",
            "You've got a stack of stone to spare. Walk up to the wooden vendor stall and RIGHT-CLICK it to open the swap window, then trade your stone across for a fresh loaf of bread. A fair barter — your stone for their baking!",
        ),
        "claim-plot" => (
            "Plant a corner post, then put a sign on your new plot.",
            "Walk to the flat patch fenced off in front of you. Hold the plot marker and right-click the ground to plant it — that whole patch is now YOUR plot. To make it truly yours, right-click to place the sign post by the corner so everyone knows whose land it is. A home base, claimed!",
        ),
        "onboarding" => (
            "Your first day: dig, craft, build a wall, cook supper.",
            "Left-click and hold to MINE 4 blocks of the little hill in front of you. Open your inventory (E) and at the crafting table make a wooden pickaxe. Then right-click to PLACE 6 blocks into a low wall. Finally, drop a raw porkchop on the campfire and right-click to take the cooked food off it. Press H any time to see what's left to do.",
        ),
        "champion" => (
            "One-on-one with a berserker boss — gear up, heal smart, and bring the giant down.",
            "This is a real boss fight. You've got an iron sword, a bow with arrows, and bread to heal. Soften the berserker from range first — hold the bow (press 2) and right-click to fire arrows. When it closes in, switch to your sword (press 1) and LEFT-CLICK, backing away between swings. Eat bread (hold right-click) whenever your health drops. Stand your ground until the giant falls!",
        ),
        "generator" => (
            "Light a lamp from a generator you build — a hand crank or steam engine, no lever needed.",
            "No lever this time — you're making the power yourself. Place a hand crank (or the steam generator) and run copper cable from it to the electric lamp on the far wall. Set the generator running and the lamp glows — electricity you made from nothing but elbow grease. Real power engineering! (In a normal world the copper for all of this comes out of the ground — Copper Rush is the trial that teaches the dig.)",
        ),
        "obsidian" => (
            "Handle a bucket of lava, meet it with water, and forge the toughest stone there is.",
            "Lava plus water makes obsidian — the hardest stone in the world. Hold the lava bucket and right-click to pour it onto the stone pad. Now hold the water bucket and pour it right next to the lava. Where they touch, the lava hardens into shiny black obsidian — give it a moment to freeze solid, then LEFT-CLICK to mine the block out and finish the trial. Careful — lava burns, so pour from the edge!",
        ),
        "dye" => (
            "Mix dyes into new shades and paint papyrus into bright wallpaper — your own colour lab.",
            "Time to play with colour! At the crafting table, combine two dyes to make a new shade — red and yellow make orange, blue and white make light blue. Then dye a sheet of papyrus with any colour to make bright wallpaper. Make three colourful things and your colour lab is open for business!",
        ),
        "scavenger" => (
            "Three minutes, survival, empty pockets — grab and craft as many DIFFERENT things as you can.",
            "The clock starts at 3:00 and your pockets are empty. Punch trees, dig dirt, sand and stone, pick flowers, scoop up water — every NEW thing counts once. Even better, CRAFT: turn logs into planks, planks into sticks, build a crafting table and tools — each different item bumps your score. It's variety that counts, not how many: one of everything beats a hundred of one. Fill your bag with as many different things as you can before time runs out!",
        ),
        "lava-floor" => (
            "The playground classic, for REAL — the whole floor is lava! Build blocks to make your own way across.",
            "Look down — that whole glowing orange floor is LAVA, and you do NOT want to touch it! Lucky you've got a giant stack of blocks. Right-click to place a block, hop onto it, and place the next — build your own islands and bridges all the way across the lava sea. Lay down 20 blocks to claim your path. You're flying in creative so it's totally safe... but pretend it isn't!",
        ),
        "kaboomtown" => (
            "Demolition time! Stack blasting kegs and set off the biggest, silliest chain of BOOMs you can.",
            "See that tower? It's BEGGING to be demolished. You've got a pile of blasting kegs and a flint & steel. Right-click to place kegs against the tower (the more the merrier!), then light one with the flint & steel and RUN. Set off at least three big booms to bring it down. Stand WELL back — this is going to be loud!",
        ),
        "monster-mash" => (
            "It's a monster mosh pit and you're the bouncer — gear up and bonk a whole rowdy crowd of raiders!",
            "A crowd of brigands has crashed the party and you're on bouncer duty! You've got an iron sword, a bow with arrows, and a pile of bread to keep your health up. Keep your back to the wall, LEFT-CLICK to bonk them one at a time, and fire arrows (hold the bow, key 2) at the ones still coming. Bonk SIX of them to clear the pit. Munch bread whenever you're hurt. Mosh on!",
        ),
        "funny-farm" => (
            "Assemble a wonderfully wacky menagerie: tame a wolf, breed the cows, ride a horse, and shear a sheep.",
            "Welcome to your very own funny farm! There's a whole crew of critters here, and four jobs to do — in ANY order: tame the wolf with a bone, feed BOTH cows wheat so they make a calf, hop on the horse for a ride, and give the sheep a haircut with the shears. Pull out the right item for each and right-click the animal. Round 'em all up!",
        ),
        "splash-zone" => (
            "Grab your buckets and flood the place — build a wild and wet water park, one splash at a time!",
            "Time to make a splash! You've got a stack of buckets full of water and a big walled pool. Right-click to pour water out wherever you like — fill the pool, send waterfalls over the edge, splash it everywhere! Pour water out FIVE times to get your park flowing. Run dry? Right-click the water you just poured to refill an empty bucket. The wetter and wackier, the better!",
        ),
        "roadrunner" => (
            "Saddle up a Nostrich and take her for a ride — climbing aboard is all it takes to finish.",
            "Right-click the Nostrich to hop on — that's the trial done! Stick around for the fun bit, though: HOLD FORWARD and she builds from a trot into a blazing sprint (watch the speed bar fill at the top). The faster you go, the more you DRIFT on turns — and you can HOLD CROUCH to drift even at lower speeds, Mario-Kart style! Wind all the way up and aim at the lake: get fast enough (watch for the '💧 SKIM!' flash) and you'll run right across the top of the water! Slow down over the lake and... SPLASH — you sink and fall off. Sneak to hop off.",
        ),
        "copper-rush" => (
            "Dig copper straight out of the rock, smelt it to an ingot, and draw your first cable.",
            "Copper starts life as ORE in the stone. Hold the stone pickaxe (press 1) and LEFT-CLICK-and-hold the orange-flecked blocks in the rock face — three of them. (A wooden pickaxe is too soft: copper needs stone or better, or the ore just crumbles to nothing.) Now place the furnace with RIGHT-CLICK and RIGHT-CLICK it to open it: coal in the bottom slot, copper ore in the top, and pull the shiny COPPER INGOT out when it pops. Last step: put the crafting table down, RIGHT-CLICK it, and lay Rubber / Copper Ingot / Rubber in a row across the middle — that's a length of cable, the wire every machine in the game runs on.",
        ),
        "mill-race" => (
            "Drop a water wheel into the falls, let the current spin it, and let it light your lamp.",
            "See the spring on top of the stone pillar? Water spills off it and falls all the way down — that falling water is a CURRENT, and a current is what turns a wheel. RIGHT-CLICK to place the water wheel right in the falling water (or in the pool at the bottom, touching the flow). Watch it: the moment it catches the current the sails start to spin. Now run cable from the wheel — RIGHT-CLICK, block by block — over to the electric lamp, place the lamp at the end, and it lights itself. No lever, no switch: the stream is doing the work. A still pond won't do it — it has to be moving water.",
        ),
        "catch-the-wind" => (
            "Put a windmill up on the hilltop platform in a thunderstorm and let the gale drive your lamp.",
            "It's blowing a gale up here — perfect mill weather. Fly up (double-tap SPACE, then SPACE to rise) to the stone platform and RIGHT-CLICK to place the windmill on it. A mill needs OPEN SKY above it, so don't build a roof over it. Give it a moment and the sails start to turn on their own. Then run cable down from the mill — RIGHT-CLICK, block by block — and put the electric lamp at the far end. The higher a mill stands the more wind it catches, which is why it's worth the climb.",
        ),
        "follow-the-plan" => (
            "Lay a plan down as a ghost guide and build the little hut it shows you, block by block.",
            "You've got a PLAN in your bag — a blueprint of a small hut. Hold it and RIGHT-CLICK the ground to set it down; a see-through outline appears where the hut will stand. When it asks how you'd like to build, choose BUILD ALONG (block by block or layer by layer) — that's the whole point of this one, so don't pick 'build it for me'. Now place your planks and glass into the glowing ghost blocks one at a time; each one you get right goes solid, and the panel tells you what's next. Fill every last ghost and the hut is yours.",
        ),
        "fresh-coat" => (
            "Blow your own body up big in the Workshop, paint it however you like, and wear it out.",
            "This is the Workshop — your painting room, and that's YOU standing on the floor. Hold the bellows (press 1) and RIGHT-CLICK the mannequin to blow yourself up to four times the size. Press TAB to open the paint panel: pick a colour off the wheel (or type a hex code), pick a tool, and LEFT-CLICK the giant body to paint it, pixel by pixel. Press R to swing the arms and legs apart so you can reach the hidden sides, and M mirrors your strokes left-to-right. When it looks right, hit PIN — your painted skin is saved and you're wearing it.",
        ),
        "bouncer" => (
            "Invent a little creature in Rig Studio out of blocks, give it the Bounce clip, and spawn it.",
            "Press Y to open RIG STUDIO. Pick a skeleton shape at the top, then hold a block in your hand (press 1–4 for the different kinds) and click a body part in the panel to build that part out of that block — head, body, arms, legs, whatever you fancy. Set the animation to BOUNCE. Then hit SPAWN and your creature appears in front of you, bouncing away. Make it silly. Make it enormous. It's your creature.",
        ),
        "suggestion-box" => (
            "Tell the makers one thing you'd love to see in the game — they really do read them.",
            "There's a straight line from you to the people building this game, and this is it. Press T to open chat, type /idea followed by your idea — for example: /idea please add hot air balloons — and press ENTER. That's it, it's on its way. If you'd rather report something broken, /bug works exactly the same way. Say what you actually think; that's the useful bit.",
        ),
        _ => ("", ""),
    }
}

/// Reverse of [`challenge_help`] for the in-game objective pop-up: the active
/// scenario carries only its `display_name`, so match that back to a bundled
/// challenge token and return its `(tagline, how_to)`. `("", "")` for a
/// scenario that isn't a bundled challenge (e.g. a community def).
#[cfg_attr(not(test), allow(dead_code))]
pub fn challenge_help_for_display(display: &str) -> (&'static str, &'static str) {
    if load_scenario_def(ONBOARDING_JSON).map(|d| d.display_name == display).unwrap_or(false) {
        return challenge_help("onboarding");
    }
    for (name, bytes) in EXPLORER_CHALLENGES {
        if load_scenario_def(bytes).map(|d| d.display_name == display).unwrap_or(false) {
            return challenge_help(name);
        }
    }
    ("", "")
}

/// Satoshi's spoken voice for a Trial, keyed by the same token as
/// [`challenge_help`] (and by a `⚡ Race` id for the three ghost races):
/// `(intro, hint)`. The `intro` is shown on a Satoshi speech card when the
/// trial starts; the `hint` is appended to the H objective pop-up as
/// "Satoshi's tip". Authored prose — warm, plain, never mentions money/earning
/// (covered by a test). A token with no entry returns `("", "")`.
pub fn trial_satoshi(name: &str) -> (&'static str, &'static str) {
    match name {
        "mine" => (
            "Every fortune under the ground starts with one swing of a pickaxe. Here's yours — point it at that stone and dig in. Twenty blocks and you've got the start of a real mine.",
            "Hold left-click on a block until it breaks — a pickaxe is much faster on stone than your bare hands.",
        ),
        "craft" => (
            "Everything you'll ever build starts with one log and a good idea. Chop that oak, lay down a workbench, and let's turn raw wood into your very first tools.",
            "Stuck? Logs become planks, planks become sticks, and a stick plus planks makes a pickaxe on the crafting table grid.",
        ),
        "smelt" => (
            "Stone tools chip and break, but iron lasts. Light the furnace, let the heat do its work, and you'll pull a gleaming ingot from the ash — then make something tough with it.",
            "No flames? The furnace needs coal in the lower slot AND raw iron in the upper slot before it'll start smelting.",
        ),
        "cook-three" => (
            "A warm fire and a hot meal — that's the heart of any camp. Get yours crackling, lay on the meat, and cook three good pieces. Nothing tastes better than food you cooked yourself.",
            "Light the campfire with the flint & steel first (it needs the sticks as fuel), then add raw food and wait for it to cook.",
        ),
        "build" => (
            "A blank floor is the best kind of invitation. Stack some walls, cut a window so the light comes in, hang a door and a sign — this is YOUR place. Build it however you dream it.",
            "Keep placing blocks to raise the walls — a window is just a hole filled with glass, and the door snaps into a one-block gap.",
        ),
        "power" => (
            "Electricity is just a path you build for power to travel. Drop a line of cable from this lever all the way to that lamp, flip the switch, and watch the whole lab light up. You're the engineer now.",
            "The lamp only lights if the cable makes an unbroken line from the lever to the lamp — fill any gaps, then flip the lever.",
        ),
        "logic-gate" => (
            "Switches are fun, but a machine that notices YOU is something else. Set a motion sensor by the door, route it through a logic gate, and watch the lamp light the instant you step in. Build a circuit that thinks a little.",
            "Connect sensor to logic gate to lamp all with cable, then walk into the space in front of the motion sensor to trip it.",
        ),
        "piston" => (
            "Every great hideout needs a door nobody can spot. Set a piston in this wall, wire it to a lever, and flip it — the wall slides open just for you. Let's build something sneaky.",
            "Connect the lever to the piston with cable, make sure the piston faces a block to push, then flip the lever to fire it.",
        ),
        "rail-rider" => (
            "There's a special joy in laying your own track and then riding it. Run a line of rail across the ground, set a cart on top, and hop in — the railway is all yours. Let's get rolling.",
            "Lay several rail blocks in a connected line first, then place the cart ON the rail and right-click it to ride.",
        ),
        "booby-trap" => (
            "Every fortress needs a nasty surprise for uninvited guests. Hook a blasting keg up to a hidden pressure plate, then test it yourself — step on, and BOOM. Engineering can be loud and fun. Mind your eyebrows.",
            "Wire the pressure plate to the blasting keg with cable, back up to a safe distance, then step onto the plate to set it off.",
        ),
        "bucket" => (
            "A bucket is the simplest little wonder there is — it lets you pick up a whole pond and carry it somewhere new. Scoop from the spring, carry it across, and pour it into the dry trough. The room changes shape in your hands.",
            "Right-click the water with the bucket to fill it, then right-click the empty trough hole to pour it back out.",
        ),
        "workshop-publish" => (
            "The deepest joy here is making something other people get to use. Craft your drafting stamp, design your reskin, and publish it. Somewhere out there, a player you'll never meet will build with the thing you made today. That's the whole dream.",
            "Craft the drafting stamp first, then press T to open chat and run the workshop publish command — type /help if you need to see it.",
        ),
        "kill" => (
            "They've come for the gate, friend — three raiders, no manners. Pick up that sword and stand your ground. You're braver than you think.",
            "Hold your sword with key 1 and left-click again and again on ONE raider until it drops, then turn to the next.",
        ),
        "armour-up" => (
            "A real warrior makes their own gear before the fight. Smelt the iron, hammer out four good pieces at the bench, then show those two raiders what fresh iron can do.",
            "Take the iron ingots OUT of the furnace, craft four iron pieces at the table, and swing your sword at the raiders — the order's up to you.",
        ),
        "breach-and-clear" => (
            "Some walls won't open with a key, friend — they open with a bang. Set that keg against the stone, spark it, and clear the way. Mind your eyebrows!",
            "Place the blasting_keg touching the wall, then light it with flint_and_steel (key 2) and step back before it blows.",
        ),
        "eat" => (
            "You can't adventure on an empty stomach, friend. Get that campfire going, roast some beef, and eat well. A fed explorer is a happy one.",
            "Light the fire with the flint & steel, take the cooked meat OFF when it's done, then hold right-click to eat — if it won't eat, you're full, so dash about a moment first.",
        ),
        "harvest" => (
            "Every great farm starts with one patch of dirt and a little patience. Here is your hoe and a pocket of seeds — let's coax some breakfast out of the ground together.",
            "Till the dirt with the hoe FIRST, then the seeds will take. A pinch of bone meal grows them in a blink.",
        ),
        "farmhand" => (
            "A homestead needs tending every morning. The sheep is woolly and the cow is full of milk — here are your shears and a bucket. Go give them their chores!",
            "Shears on the sheep, empty bucket on the cow — right-click each animal up close.",
        ),
        "rancher" => (
            "Animals grow a herd when they're well fed and happy. Hold this wheat and offer some to each cow — watch for the little hearts, and a brand-new calf will soon join the pair.",
            "Right-click EACH cow with wheat — both need hearts before a calf can appear.",
        ),
        "mule-maker" => (
            "A mule isn't born from just any two animals, friend — it takes a horse AND a donkey, the one cross going in this whole valley. Feed them both some wheat and see what turns up between them.",
            "Remember to CROUCH while you feed — a plain right-click just climbs into the saddle instead of offering the wheat.",
        ),
        "tame-wolf" => (
            "That wolf is wild and wary, but a kind hand and a few bones can change everything. Hold a bone, step close, and right-click — keep offering until it trusts you.",
            "Keep feeding bones — one isn't always enough. The burst of hearts means it's yours.",
        ),
        "friend-in-need" => (
            "Every good companion starts out as a stranger. There's a cat, a parrot and a fox nearby, each with a favourite treat — work out which likes what, offer it, and be patient. One loyal friend is all you need.",
            "Cat likes raw fish, parrot likes wheat seeds, fox likes berries. If it doesn't take the first time, just keep offering.",
        ),
        "fish" => (
            "There's a quiet patience to fishing that I've always loved. Cast your line onto the pond, watch the bobber, and the moment it dips — snap it back and reel in your catch.",
            "Cast with right-click, then right-click again the instant the bobber dips and splashes.",
        ),
        "ride" => (
            "Nothing covers ground quite like a good horse. Step up beside it and right-click to swing into the saddle — then the reins are yours. Off you go!",
            "Get right next to the horse and right-click it to mount up.",
        ),
        "fish-feast" => (
            "Here's the whole circle of a simple meal: catch it, cook it, eat it. Cast your rod onto the pond first, then bring your fish to the fire. There's real pride in eating something you made from nothing.",
            "Order matters: catch the fish first, THEN cook it on the campfire, THEN eat it.",
        ),
        "vendor-sale" => (
            "Market day! A trade is just two people each handing over something the other wants more. You've got strong stone; the stall has warm bread. Swap it across — everyone walks away happier.",
            "Right-click the wooden vendor stall to open the swap window, then move your stone in to trade it for bread.",
        ),
        "claim-plot" => (
            "Every great build starts with a single post in the ground that says 'this spot is mine'. Plant your plot marker on the open ground ahead, then set your sign by the corner. From here on, this little patch of the world answers to you.",
            "Hold the plot marker and right-click the flat open ground to plant your claim, then place the sign next to it.",
        ),
        "onboarding" => (
            "Welcome, friend. Everything here starts with your two hands and a little curiosity. Break a few blocks, make a tool, raise a wall, and cook your first meal — that's a whole day's adventuring, right there.",
            "Stuck? Hold left-click on a block until it pops, then open your inventory with E to craft. Right-click puts blocks back down.",
        ),
        "sprint" => (
            "Welcome to the starting line, runner. Sixty blocks of open ground, one bright beacon, and a ghost made of your own best run. Don't think too hard — just go the instant you're ready, and don't slow until the beacon's behind you.",
            "Stop steering and start sprinting — pick the straightest line to the beacon and never ease off; the ghost won't.",
        ),
        "cross-country" => (
            "Now the ground fights back a little. About 225 blocks of hills and hollows stand between you and the beacon (more, the way the hills make you walk it), and brute speed alone won't win it — the runner who reads the land and flows around the hills beats the one who charges over them. Find your line.",
            "Don't fight the hills — go around the steep ones and save your straight-line sprints for the flat stretches.",
        ),
        "marathon" => (
            "This is the long road, runner — about a thousand blocks to the beacon. Forget the sprinter's burst; out here the win goes to whoever holds the steadiest line and never stops moving. Pace yourself, trust your route, and let the ghost show you where you lost time last run.",
            "Smooth beats fast over this distance — keep one steady line to the beacon instead of zig-zagging, and momentum does the work.",
        ),
        "champion" => (
            "They say no one's beaten the berserker one-on-one. You've got a blade, a bow, and bread for the wounds — everything you need but the nerve. Soften him from afar, strike when he's close, and heal often. Make them tell a new story.",
            "Fire arrows with the bow (key 2) while he's far off, swap to the sword up close, and eat bread the moment your health dips.",
        ),
        "generator" => (
            "A switch just borrows power that's already there. A generator MAKES it. Set down a hand crank or a steam engine, wire it to the lamp, and bring your own light into the world. You're the power station now.",
            "Run cable from your generator (hand crank or steam generator) all the way to the lamp, then set it running — the lamp lights when power reaches it.",
        ),
        "obsidian" => (
            "Here's a bit of earth-craft, friend: fire and water don't cancel out — together they make something new. Pour your lava onto the pad, splash water against it, and watch obsidian form, tough enough to outlast everything. Give it a moment to freeze solid, then dig it up to finish. Mind your toes around the lava.",
            "Pour the lava bucket first, then pour the water bucket right beside it — wait for the black obsidian to form, then mine it out.",
        ),
        "dye" => (
            "Every world needs colour, and colour is something you MIX. Put two dyes together for a brand-new shade, then brush it onto papyrus for bright wallpaper. There's no wrong combination — make something that makes you smile.",
            "On the crafting table: two dyes together make a new colour, and a dye plus a papyrus sheet makes coloured wallpaper. Craft three things to finish.",
        ),
        "scavenger" => (
            "Empty pockets, three minutes on the clock, and a whole world to rummage through! It's not how MUCH you grab — it's how many DIFFERENT things. Punch some wood, dig the ground, pick a flower, then craft something brand-new from what you found. Ready? Go, go, go!",
            "Variety beats quantity — one of each new thing scores, a stack of the same doesn't. Crafting planks, sticks and tools makes whole new items, so keep crafting as you go!",
        ),
        "lava-floor" => (
            "Everybody knows the rules: the floor is LAVA! I've turned this whole room into a bubbling orange sea, and the only way across is the path YOU build. Drop blocks, hop along them, make your islands. Don't touch the floor... or do, you're in creative — but where's the fun in that?",
            "Right-click to lay a block at your feet, step onto it, and keep going — a chain of islands all the way across the lava.",
        ),
        "kaboomtown" => (
            "Some days you build things up. Today? We knock 'em DOWN. Here's a heap of blasting kegs — stack 'em against that tower, light the fuse, and leg it. The bigger the boom, the better. Mind your eyebrows!",
            "Place kegs right next to each other so one sets off the next, then light just ONE with the flint & steel and back away — boom, boom, BOOM!",
        ),
        "monster-mash" => (
            "Hope you brought your dancing shoes, because this monster mosh pit is HOPPING — six rowdy raiders and you're the only bouncer. Grab that sword, keep the bread handy, and bonk 'em out one by one. It's loud, it's chaos, and it's a riot. GO!",
            "Back into the wall so they can't surround you, bonk ONE at a time, and eat bread the second your health dips.",
        ),
        "funny-farm" => (
            "Every great adventurer needs a sidekick or twelve! I've gathered a wonderfully silly bunch of critters for you — a wolf, some cows, a horse, a fluffy sheep. Tame, breed, ride and shear your way to the wackiest little zoo in the world. Off you go, farmer!",
            "One job per critter: a bone for the wolf, wheat for BOTH cows, an empty hand on the horse, shears on the sheep — any order you like.",
        ),
        "splash-zone" => (
            "Who says you can't bring the beach indoors? Here are your buckets — go wild! Pour water into the pool, send it cascading down the sides, splash it everywhere. Build the silliest, soggiest water park you can dream up. Make a splash!",
            "Hold a water bucket and right-click the ground or the pool to pour it out — five splashes fills your park. Refill from any water you've poured.",
        ),
        "roadrunner" => (
            "Meet your new favourite ride — the mighty Nostrich! Climb aboard and she's yours to gallop — that's the whole trial, done the moment you're in the saddle. Stick around, though: wind her up and at full tilt she runs CLEAN ACROSS WATER. Hold on tight, lean into the drifts, and whatever you do — don't slow down over the lake!",
            "Right-click the Nostrich to mount up — that finishes the trial. Fancy the extra challenge? Wind up to full speed on dry land FIRST (watch for '💧 SKIM!'), THEN hit the water — and keep it pinned, because slowing down over water means a splash.",
        ),
        "copper-rush" => (
            "Everything electric in this world starts as a dull orange streak in the rock. Chip three lumps of copper ore out of that face, cook them in the furnace until they run bright, and roll the metal out into cable. Do that once and you can wire up anything you can imagine.",
            "Stone pickaxe or better for copper — wood just crumbles it. Coal in the bottom furnace slot, ore in the top. Then Rubber / Copper Ingot / Rubber in a row on the crafting table.",
        ),
        "mill-race" => (
            "The oldest machine there is: put a wheel where the water falls and let the river do your work for you. No switch to flip, no fuel to feed — drop it in the current and it just goes. Get it turning, then run a cable across and let the stream light your lamp.",
            "It has to be MOVING water — falling or flowing, never a still pond. Place the wheel right in the falls, then cable from the wheel to the lamp.",
        ),
        "catch-the-wind" => (
            "Hear that? That's a thunderstorm rolling through, and a mill has never had a better day. Get up on that platform, plant your windmill where nothing shades it from the sky, and let the gale turn the sails. Then wire it up — free power, straight out of the weather.",
            "A mill needs open sky above it, so don't roof it in. Once the sails are spinning, run cable from the mill to the lamp.",
        ),
        "follow-the-plan" => (
            "A plan is a builder's memory: lay it down and it shows you, ghost block by ghost block, exactly what to put where. Set this one down, pick BUILD ALONG, and put every block in yourself. There's a real satisfaction in the last ghost going solid.",
            "When it asks how to build, choose build-along — not 'build it for me', or nothing gets counted. Then fill every glowing ghost block.",
        ),
        "fresh-coat" => (
            "That's you standing there. Blow yourself up big with the bellows and you can paint every last pixel — freckles, stripes, armour, whatever you fancy. Nobody else gets a say in what you look like. Paint it, pin it, wear it.",
            "Bellows on the mannequin to blow it up, TAB for the paint panel, R to swing the limbs apart, then PIN to save it and put it on.",
        ),
        "bouncer" => (
            "Time to make something alive. Rig Studio takes plain blocks and hangs them on a skeleton, and then it MOVES — so pick a shape, hand it some blocks, set it to Bounce, and let it loose. The daftest creature wins.",
            "Y opens Rig Studio. Hold a block, click a body part to build that part from it, set the clip to Bounce, then Spawn.",
        ),
        "suggestion-box" => (
            "This bit matters. The people building this world want to know what you'd add to it — and there's a direct line from your chat box to them. One idea, in your own words. Nothing too big, nothing too silly.",
            "Press T, type /idea followed by whatever you'd like to see, and press Enter. /bug does the same for something that looks broken.",
        ),
        _ => ("", ""),
    }
}

/// Reverse of [`trial_satoshi`] for a running scenario carrying only its
/// `display_name`: match it back to a bundled-challenge token and return
/// Satoshi's `(intro, hint)`. `("", "")` for a non-bundled scenario.
pub fn trial_satoshi_for_display(display: &str) -> (&'static str, &'static str) {
    if load_scenario_def(ONBOARDING_JSON).map(|d| d.display_name == display).unwrap_or(false) {
        return trial_satoshi("onboarding");
    }
    for (name, bytes) in EXPLORER_CHALLENGES {
        if load_scenario_def(bytes).map(|d| d.display_name == display).unwrap_or(false) {
            return trial_satoshi(name);
        }
    }
    ("", "")
}

/// Whether to OFFER the onboarding arc to a player arriving at the lobby.
///
/// Owner decision (2026-06-15): only a genuine first-timer should be prompted —
/// someone with **no local saved worlds AND no Stash worlds** (no evidence
/// they've played before). Returning players (any local world or any stashed
/// world) go straight to their normal lobby with no prompt. The arc is a
/// non-forcing popup ("New here? Want the guided start?") with a skip — NOT an
/// unconditional auto-start, and it never dives past the lobby into a world.
///
/// Pure so the trigger is unit-testable; the popup UI + its create-or-start
/// lifecycle is Phase 6 (needs the running app). The lobby passes its real
/// counts (`local_world_count`, `stash_world_count`).
#[cfg_attr(not(test), allow(dead_code))]
pub fn should_offer_onboarding(local_world_count: usize, stash_world_count: usize) -> bool {
    local_world_count == 0 && stash_world_count == 0
}

/// Every bundled challenge def (onboarding arc first, then the explorer cards).
/// Parsing the embedded artifacts is a build-time invariant (covered by a test),
/// so the `expect`s are sound. This is the list the challenge board renders.
#[cfg_attr(not(test), allow(dead_code))]
pub fn challenge_pack() -> Vec<ScenarioDef> {
    let mut out = vec![load_scenario_def(ONBOARDING_JSON).expect("bundled onboarding.json must parse")];
    for (name, bytes) in EXPLORER_CHALLENGES {
        out.push(
            load_scenario_def(bytes)
                .unwrap_or_else(|e| panic!("bundled explorer challenge {name} must parse: {e}")),
        );
    }
    out
}

/// Resolve a built-in scenario by name (the `/scenario <name>` surface). The
/// runner ships `test`; official mods (`hash-dash`, `satori-rush`) are embedded
/// DATA here for offline dev/test, and delivered live via the open-stash (Goal 5).
pub fn named_builtin_def(name: &str) -> Option<ScenarioDef> {
    match name {
        "test" => Some(builtin_test_def()),
        "hash-dash" | "hashdash" => load_scenario_def(HASH_DASH_JSON).ok(),
        "satori-rush" | "satorirush" => load_scenario_def(SATORI_RUSH_JSON).ok(),
        "onboarding" | "getting-started" => load_scenario_def(ONBOARDING_JSON).ok(),
        // Explorer challenges by their short name (e.g. `/scenario tame-wolf`).
        other => EXPLORER_CHALLENGES
            .iter()
            .find(|(n, _)| *n == other)
            .and_then(|(_, bytes)| load_scenario_def(bytes).ok()),
    }
}

/// One-shot launch request for a Stash Column "Play": parked on `GameState`
/// while a fresh arena world loads, then consumed on the FIRST play tick
/// (mirrors `pending_workshop_reset`). Factored out + pure so the cross-frame
/// lifecycle is unit-testable without the running app. The `def` is the
/// read-only official/community template; the arena run saves as the player's
/// OWN world (the template is never mutated).
#[derive(Clone, Debug, PartialEq)]
pub struct PendingScenarioLaunch {
    pub def: ScenarioDef,
}

impl PendingScenarioLaunch {
    pub fn new(def: ScenarioDef) -> Self {
        Self { def }
    }

    /// The world seed for this launch's fresh arena. A def with a fixed
    /// `arena_seed` always generates the same fair map (repeatable scoring);
    /// without one, the caller's random `fallback` seed is used ("any terrain,
    /// score is work"). Pure — the testable crux of the per-def arena decision.
    pub fn arena_seed(&self, fallback: u32) -> u32 {
        self.def.arena_seed.unwrap_or(fallback)
    }

    /// Whether this launch wants a FIXED arena (a published, repeatable map) vs
    /// a random one. Drives the launch log + the "fair scoring" guarantee.
    pub fn has_fixed_arena(&self) -> bool {
        self.def.arena_seed.is_some()
    }
}

/// Does provisioning `def` wipe each local player's bag first? Arena games
/// (Hash Dash / Satori Rush) want a fair-start clear; so does a timed
/// score-attack Challenge (the Scavenger, "start with nothing" on every run).
/// Event-driven coverage Challenges never clear.
pub fn provision_clears_inventory(def: &ScenarioDef) -> bool {
    def.kind != ScenarioKind::Challenge || matches!(def.objective, Objective::Timed { .. })
}

/// Does starting `def` change a player's inventory at all — a clear or a kit?
/// Such a scenario only ever runs in its own arena world, never overlaid on the
/// player's current world (`world_exit::challenge_launch_mode`): in-world it
/// either wiped real gear (Scavenger) or was a repeatable free-items tap (most
/// explorer Challenges carry kits — 43 of 45 at the 2026-09-27 audit).
pub fn provision_touches_inventory(def: &ScenarioDef) -> bool {
    provision_clears_inventory(def) || !def.kit.is_empty()
}

/// Clear + provision **every** local player with the scenario's kit. Scenarios
/// are fair-start minigames (e.g. split-screen Hash Dash), so all local players
/// get the same loadout. The old launch path provisioned only slot 0, leaving
/// split-screen P2-4 holding their survival gear — an uneven start and an
/// inventory-laundering exploit (engine audit 2026-06-04, F; owner decision
/// P0.4: fix the clear; the no-ledger-flag design is left as-is, see the
/// `is_cheat()` rationale on `ScenarioCommand`).
pub fn provision_all_players(players: &mut [crate::player_slot::PlayerSlot], def: &ScenarioDef) {
    let fair_start = provision_clears_inventory(def);
    for slot in players.iter_mut() {
        if fair_start {
            slot.inventory.clear();
        }
        for kit_item in &def.kit {
            if let Ok(stack) =
                crate::commands::builtins::give::resolve_item(&kit_item.name, kit_item.count)
            {
                let _ = slot.inventory.add_item(stack);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timed_work_def(ticks: u32) -> ScenarioDef {
        ScenarioDef {
            kind: ScenarioKind::Test,
            display_name: "T".to_string(),
            lock_creative: false,
            arena_mode: ArenaMode::Reuse,
            kit: vec![],
            objective: Objective::Timed { ticks },
            scoring: Scoring::Work,
            arena_seed: None,
            tuning: ScenarioTuning::default(),
            world_type: None,
            game_mode: None,
            time_lock: None,
            weather_lock: None,
            mobs_enabled: None,
            trial_race: None,
            arena: None,
        }
    }

    #[test]
    fn provision_all_players_clears_and_kits_every_local_player() {
        use crate::item::{Item, ItemStack};
        use crate::player_slot::PlayerSlot;
        let mut def = timed_work_def(100);
        def.kit = vec![KitItem { name: "stone".to_string(), count: 5 }];
        let mut players = vec![
            PlayerSlot::new(0, glam::Vec3::ZERO, 0.5),
            PlayerSlot::new(1, glam::Vec3::ZERO, 0.5),
        ];
        // Both players hold pre-scenario survival gear that must be cleared.
        for p in players.iter_mut() {
            p.inventory.set_slot(0, Some(ItemStack::new_block(crate::block::DIAMOND_BLOCK, 64)));
        }
        provision_all_players(&mut players, &def);
        for (i, p) in players.iter().enumerate() {
            let has_diamond = p.inventory.slots_iter().flatten()
                .any(|s| matches!(s.item, Item::Block(b) if b == crate::block::DIAMOND_BLOCK));
            assert!(!has_diamond, "player {i}'s pre-scenario gear must be cleared");
            let stone: u32 = p.inventory.slots_iter().flatten()
                .filter(|s| matches!(s.item, Item::Block(b) if b == crate::block::STONE))
                .map(|s| s.count as u32).sum();
            // The old slot-0-only path left split-screen P2 unprovisioned.
            assert_eq!(stone, 5, "player {i} must be provisioned with the kit");
        }
    }

    #[test]
    fn provision_does_not_wipe_inventory_for_feature_coverage_challenges() {
        // Wave 6 footgun fix — a `Challenge`-kind def (onboarding / explorer card)
        // OVERLAYS its objective on the current world: clicking it on the board
        // must NOT clear the player's survival gear. Only fair-start arena games
        // wipe + kit.
        use crate::item::{Item, ItemStack};
        use crate::player_slot::PlayerSlot;
        let def = challenge_def(Objective::Action {
            event: ChallengeEvent::CookAtCampfire,
            count: 3,
        }); // challenge_def is ScenarioKind::Test — make it a real Challenge:
        let mut challenge = def;
        challenge.kind = ScenarioKind::Challenge;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        players[0]
            .inventory
            .set_slot(0, Some(ItemStack::new_block(crate::block::DIAMOND_BLOCK, 64)));
        provision_all_players(&mut players, &challenge);
        let diamonds: u32 = players[0]
            .inventory
            .slots_iter()
            .flatten()
            .filter(|s| matches!(s.item, Item::Block(b) if b == crate::block::DIAMOND_BLOCK))
            .map(|s| s.count as u32)
            .sum();
        assert_eq!(diamonds, 64, "a feature-coverage challenge must not wipe survival gear");
    }

    fn satori_def() -> ScenarioDef {
        ScenarioDef {
            kind: ScenarioKind::SatoriRush,
            display_name: "S".to_string(),
            lock_creative: false,
            arena_mode: ArenaMode::Reuse,
            kit: vec![],
            objective: Objective::FirstSatori,
            scoring: Scoring::None,
            arena_seed: None,
            tuning: ScenarioTuning::default(),
            world_type: None,
            game_mode: None,
            time_lock: None,
            weather_lock: None,
            mobs_enabled: None,
            trial_race: None,
            arena: None,
        }
    }

    #[test]
    fn timer_ends_exactly_after_duration() {
        let mut s = ScenarioState::new(timed_work_def(3));
        assert!(!s.tick()); // 1
        assert!(!s.tick()); // 2
        assert!(s.tick()); // 3 — transitions to ended this tick
        assert!(s.is_ended());
        assert_eq!(s.result_tick(), Some(3));
    }

    #[test]
    fn shows_blocking_end_card_only_for_ended_timed_scenarios() {
        // Fresh timed scenario: not ended → no card (cursor stays captured).
        let mut timed = ScenarioState::new(timed_work_def(2));
        assert!(!timed.shows_blocking_end_card(), "not ended yet");
        timed.tick();
        timed.tick(); // expires → ended this tick
        assert!(timed.is_ended());
        assert!(timed.shows_blocking_end_card(), "ended timed → blocking end-card");

        // Satori Rush ends NON-blocking (player keeps playing) → never the
        // cursor-releasing blocking card, even once ended.
        let mut satori = ScenarioState::new(satori_def());
        satori.on_material_gained(MaterialId::Satori);
        assert!(satori.is_ended());
        assert!(!satori.shows_blocking_end_card(), "satori ends non-blocking");
    }

    #[test]
    fn timer_does_not_end_early() {
        let mut s = ScenarioState::new(timed_work_def(10));
        for _ in 0..9 {
            assert!(!s.tick());
        }
        assert!(!s.is_ended());
    }

    #[test]
    fn remaining_ticks_counts_down() {
        let mut s = ScenarioState::new(timed_work_def(10));
        s.tick();
        s.tick();
        s.tick();
        assert_eq!(s.remaining_ticks(), 7);
    }

    #[test]
    fn work_accrues_on_break_when_scoring_work() {
        let mut s = ScenarioState::new(timed_work_def(100));
        s.on_block_broken(50);
        s.on_block_broken(1);
        assert_eq!(s.score(), 51);
    }

    #[test]
    fn no_score_when_scoring_none() {
        let mut def = timed_work_def(100);
        def.scoring = Scoring::None;
        let mut s = ScenarioState::new(def);
        s.on_block_broken(50);
        assert_eq!(s.score(), 0);
    }

    #[test]
    fn ended_scenario_ignores_further_events() {
        let mut s = ScenarioState::new(timed_work_def(1));
        assert!(s.tick()); // ends
        s.on_block_broken(50);
        assert_eq!(s.score(), 0);
        assert!(!s.tick()); // no further ticks accrue
        assert_eq!(s.elapsed_ticks(), 1);
    }

    #[test]
    fn work_exponent_weights_hard_blocks_steeper() {
        let mut def = timed_work_def(100);
        def.tuning.work_exponent = 2.0;
        let mut s = ScenarioState::new(def);
        s.on_block_broken(10); // 10^2 = 100
        assert_eq!(s.score(), 100);
    }

    #[test]
    fn work_exponent_is_clamped_against_absurd_values() {
        let mut def = timed_work_def(100);
        def.tuning.work_exponent = 1000.0; // adversarial / malformed
        let mut s = ScenarioState::new(def);
        s.on_block_broken(75); // the hardest block's work
        // Clamped to exponent 8 → a large but FINITE value, not a saturated u64.
        assert_eq!(s.score(), 75f64.powf(8.0).round() as u64);
        assert!(s.score() < u64::MAX);
    }

    #[test]
    fn tuning_default_is_linear() {
        assert_eq!(ScenarioTuning::default().work_exponent, 1.0);
    }

    #[test]
    fn first_satori_ends_run_and_records_genesis_tick() {
        let mut s = ScenarioState::new(satori_def());
        for _ in 0..42 {
            s.tick();
        }
        s.on_material_gained(MaterialId::Coal); // not the objective material
        assert!(!s.is_ended());
        s.on_material_gained(MaterialId::Satori); // Genesis Block!
        assert!(s.is_ended());
        assert_eq!(s.result_tick(), Some(42));
    }

    #[test]
    fn first_satori_ignores_non_satori_materials() {
        let mut s = ScenarioState::new(satori_def());
        s.on_material_gained(MaterialId::Diamond);
        s.on_material_gained(MaterialId::IronIngot);
        assert!(!s.is_ended());
    }

    #[test]
    fn timed_objective_unaffected_by_satori() {
        // A timed run does not end just because a Satori drops.
        let mut s = ScenarioState::new(timed_work_def(100));
        s.on_material_gained(MaterialId::Satori);
        assert!(!s.is_ended());
    }

    #[test]
    fn def_json_round_trips() {
        let def = builtin_test_def();
        let json = def.to_json().unwrap();
        let back = load_scenario_def(json.as_bytes()).unwrap();
        assert_eq!(def, back);
    }

    #[test]
    fn minimal_def_json_loads_with_serde_defaults() {
        // No kit / scoring / arena_seed / tuning → all default (forward-compat).
        let json = r#"{"kind":"Test","display_name":"M","objective":"FirstSatori"}"#;
        let def = load_scenario_def(json.as_bytes()).unwrap();
        assert_eq!(def.kit.len(), 0);
        assert_eq!(def.scoring, Scoring::None);
        assert_eq!(def.arena_seed, None);
        assert_eq!(def.tuning.work_exponent, 1.0);
        assert!(!def.lock_creative, "creative lock is off by default (opt-in)");
        assert_eq!(def.arena_mode, ArenaMode::Reuse, "arena mode defaults to Reuse");
    }

    #[test]
    fn load_is_tolerant_of_unknown_fields_for_save_and_mod_forward_compat() {
        // RUNTIME `load_scenario_def` must stay tolerant of unknown fields: a
        // ScenarioDef is persisted into a world save (`WorldMeta.scenario_def`)
        // and deserialised from community mods authored against other client
        // versions, so a `deny_unknown_fields` here would break a stale save /
        // forward-published mod. An unknown key is IGNORED, not an error — the
        // known fields still load. (The strict "no stray keys" guarantee is
        // enforced only over BUNDLED trials, at test time, by trials_lint's
        // `every_bundled_field_is_recognised`.)
        let json = r#"{"kind":"Test","display_name":"M","objective":"FirstSatori","future_field":42}"#;
        let def = load_scenario_def(json.as_bytes())
            .expect("an unknown field must be tolerated at runtime, not rejected");
        assert_eq!(def.display_name, "M");
        assert_eq!(def.objective, Objective::FirstSatori);
    }

    #[test]
    fn plan_arena_launch_covers_all_modes() {
        use ArenaMode::*;
        // Reuse: always fresh; wipe a stale world, else nothing to wipe.
        assert_eq!(plan_arena_launch(Reuse, true), ArenaLaunch::Fresh { wipe: true });
        assert_eq!(plan_arena_launch(Reuse, false), ArenaLaunch::Fresh { wipe: false });
        // Resume: resume an existing run; fresh (no wipe) the first time.
        assert_eq!(plan_arena_launch(Resume, true), ArenaLaunch::Resume);
        assert_eq!(plan_arena_launch(Resume, false), ArenaLaunch::Fresh { wipe: false });
        // KeepNew: always a fresh (salted-unique) world; never wipes.
        assert_eq!(plan_arena_launch(KeepNew, true), ArenaLaunch::Fresh { wipe: false });
        assert_eq!(plan_arena_launch(KeepNew, false), ArenaLaunch::Fresh { wipe: false });
    }

    #[test]
    fn experience_world_slug_is_stable_and_safe() {
        assert_eq!(experience_world_slug("Hash Dash"), "exp-hash-dash");
        assert_eq!(experience_world_slug("Satori Rush"), "exp-satori-rush");
        // Punctuation / extra spacing collapse to single dashes; no random suffix.
        assert_eq!(experience_world_slug("My  Cool!! Map"), "exp-my-cool-map");
        assert_eq!(experience_world_slug(""), "exp-x");
        // Stable across calls (Reuse/Resume rely on this).
        assert_eq!(experience_world_slug("Hash Dash"), experience_world_slug("Hash Dash"));
    }

    #[test]
    fn official_arena_modes() {
        // Hash Dash is a throwaway (reuse one fresh world); Satori Rush is the
        // take-home speedrun (resume your run).
        assert_eq!(hash_dash_def().arena_mode, ArenaMode::Reuse);
        assert_eq!(satori_rush_def().arena_mode, ArenaMode::Resume);
    }

    #[test]
    fn named_builtin_resolves_known_scenarios() {
        assert!(named_builtin_def("test").is_some());
        assert!(named_builtin_def("hash-dash").is_some());
        assert!(named_builtin_def("nope").is_none());
    }

    #[test]
    fn challenge_listing_covers_onboarding_plus_explorer_and_every_name_starts() {
        let listing = challenge_listing();
        // onboarding + the 5 explorer cards.
        assert_eq!(listing.len(), 1 + EXPLORER_CHALLENGES.len());
        assert!(listing.iter().any(|(name, _)| *name == "onboarding"));
        assert!(listing.iter().any(|(name, _)| *name == "tame-wolf"));
        // Every listed start-name must actually launch (the list can't lie).
        for (name, display) in &listing {
            assert!(named_builtin_def(name).is_some(), "{name} must start");
            assert!(!display.is_empty(), "{name} must have a display name");
        }
    }

    // ─── Beacon scenario blob (Stash Column, Prague delivery) ───

    #[test]
    fn scenario_blob_round_trips_for_every_builtin() {
        // Each embedded def serialises to a version-prefixed blob and decodes
        // back identically — the artifact that rides a `scenario` Beacon item.
        for def in [builtin_test_def(), hash_dash_def(), satori_rush_def()] {
            let blob = scenario_to_blob_bytes(&def).unwrap();
            assert_eq!(blob[0], SCENARIO_BLOB_VERSION, "blob is version-prefixed");
            assert_eq!(scenario_from_blob_bytes(&blob).unwrap(), def);
        }
    }

    #[test]
    fn scenario_from_blob_rejects_empty_newer_and_garbage() {
        // Empty → error (no version byte).
        assert!(scenario_from_blob_bytes(&[]).is_err());
        // Forge a version one past supported → must reject loudly.
        let mut blob = scenario_to_blob_bytes(&builtin_test_def()).unwrap();
        let good = blob.clone();
        blob[0] = SCENARIO_BLOB_VERSION + 1;
        let err = scenario_from_blob_bytes(&blob).unwrap_err();
        assert!(err.to_lowercase().contains("newer"), "version error: {err}");
        // Valid version byte but non-JSON payload → parse error, never a panic.
        assert!(scenario_from_blob_bytes(&[SCENARIO_BLOB_VERSION, 0xff, 0x00, 0x13]).is_err());
        // Sanity: the untouched blob still parses.
        assert!(scenario_from_blob_bytes(&good).is_ok());
    }

    // ─── Stash Column arena launch (Phase 3 — pure transitions) ───

    #[test]
    fn pending_launch_uses_fixed_arena_seed_when_present() {
        let mut def = timed_work_def(100);
        def.arena_seed = Some(777);
        let p = PendingScenarioLaunch::new(def);
        assert!(p.has_fixed_arena());
        // The fixed seed is used regardless of the random fallback.
        assert_eq!(p.arena_seed(0xDEAD_BEEF), 777);
        assert_eq!(p.arena_seed(123), 777);
    }

    #[test]
    fn pending_launch_falls_back_to_random_seed_without_arena() {
        let mut def = timed_work_def(100);
        def.arena_seed = None;
        let p = PendingScenarioLaunch::new(def);
        assert!(!p.has_fixed_arena());
        // No fixed arena → the caller's random fallback drives the world.
        assert_eq!(p.arena_seed(0xDEAD_BEEF), 0xDEAD_BEEF);
        assert_eq!(p.arena_seed(42), 42);
    }

    #[test]
    fn pending_launch_carries_the_def_unmutated() {
        // The launch must preserve the exact def (the read-only template) so the
        // arena run starts the intended scenario, kit + objective intact.
        let def = hash_dash_def();
        let p = PendingScenarioLaunch::new(def.clone());
        assert_eq!(p.def, def);
    }

    #[test]
    fn pending_launch_option_is_a_one_shot() {
        // Models the GameState field: arm with Some, consume once with take().
        let mut slot: Option<PendingScenarioLaunch> = None;
        slot = Some(PendingScenarioLaunch::new(satori_rush_def()));
        let taken = slot.take();
        assert!(taken.is_some(), "first take yields the launch");
        assert!(slot.is_none(), "armed launch is consumed exactly once");
        assert!(slot.take().is_none(), "second take is empty — no double-launch");
    }

    #[test]
    fn scenario_and_skin_blobs_do_not_cross_parse() {
        // The two Beacon content types are distinct payloads under distinct
        // tags; neither blob may decode as the other (a JSON def is not a
        // bincode override-set and vice versa). The tag is the primary
        // discriminator (see open_stash::classify_content_type); this is the
        // belt-and-braces byte-level guard.
        let scenario_blob = scenario_to_blob_bytes(&hash_dash_def()).unwrap();
        assert!(
            crate::override_registry::OverrideSet::from_blob_bytes(&scenario_blob).is_err(),
            "a scenario blob must not parse as a skin override-set"
        );
        let skin_blob = crate::override_registry::OverrideSet {
            version: crate::override_registry::OVERRIDE_SET_VERSION,
            ..Default::default()
        }
        .to_blob_bytes()
        .unwrap();
        assert!(
            scenario_from_blob_bytes(&skin_blob).is_err(),
            "a skin override-set blob must not parse as a scenario"
        );
    }

    #[test]
    fn kit_items_resolve_to_real_items() {
        // Every KitItem name in the built-in def must resolve via the give
        // vocabulary, so the runner can actually provision the kit.
        for item in builtin_test_def().kit {
            assert!(
                crate::commands::builtins::give::resolve_item(&item.name, item.count).is_ok(),
                "kit item {:?} should resolve via give::resolve_item",
                item.name
            );
        }
    }

    // ─── Hash Dash (Goal 3) ───

    #[test]
    fn hash_dash_def_shape() {
        let d = hash_dash_def();
        assert_eq!(d.kind, ScenarioKind::HashDash);
        assert_eq!(d.scoring, Scoring::Work);
        // Hard ~3-minute timer (3600 ticks @ 20 TPS = 180s).
        assert_eq!(d.objective, Objective::Timed { ticks: 3600 });
        // Hand-only start (owner call 2026-06-06): no kit — you begin bare-handed
        // and the winning strategy is to SPEED-CRAFT up the tool tree (punch wood
        // → wood tools → stone → harder blocks) inside the timer. Tooling up fast
        // IS the run, not a positioning decision off a free stone kit.
        assert!(d.kit.is_empty(), "Hash Dash starts hand-only — no kit");
        // Slightly steeper-than-linear so harder blocks out-rate easy ones — the
        // lever that makes racing UP the tool tree pay off.
        assert!(d.tuning.work_exponent > 1.0);
    }

    #[test]
    fn hash_dash_json_artifact_round_trips() {
        // The bundled JSON (Goal 5's mod artifact) parses to the canonical def,
        // and re-serialising round-trips.
        let from_json = load_scenario_def(HASH_DASH_JSON).unwrap();
        assert_eq!(from_json, hash_dash_def());
        let json = from_json.to_json().unwrap();
        assert_eq!(load_scenario_def(json.as_bytes()).unwrap(), from_json);
    }

    #[test]
    fn hash_dash_runs_to_timeout_accruing_work() {
        // Drive the actual Hash Dash def: mining accrues a WORK score and the
        // run ends exactly when the 3-minute timer expires.
        let mut s = ScenarioState::new(hash_dash_def());
        s.on_block_broken(crate::crafting::block_work(crate::block::STONE));
        s.on_block_broken(crate::crafting::block_work(crate::block::OAK_LOG));
        assert!(s.score() > 0, "work must accrue into the Hash Dash score");
        for _ in 0..3599 {
            assert!(!s.tick(), "must not end before the 3600th tick");
        }
        assert!(s.tick(), "the 3600th tick ends Hash Dash");
        assert!(s.is_ended());
    }

    // ─── Satori Rush (Goal 4) ───

    #[test]
    fn satori_rush_def_shape() {
        let d = satori_rush_def();
        assert_eq!(d.kind, ScenarioKind::SatoriRush);
        assert_eq!(d.objective, Objective::FirstSatori);
        // From scratch — no kit; tooling up IS the run.
        assert!(d.kit.is_empty());
        assert!(d.is_resumable());
    }

    #[test]
    fn satori_rush_json_artifact_round_trips() {
        let from_json = load_scenario_def(SATORI_RUSH_JSON).unwrap();
        assert_eq!(from_json, satori_rush_def());
        let json = from_json.to_json().unwrap();
        assert_eq!(load_scenario_def(json.as_bytes()).unwrap(), from_json);
    }

    #[test]
    fn satori_rush_ends_on_first_satori_recording_world_clock() {
        let mut s = ScenarioState::new(satori_rush_def());
        for _ in 0..200 {
            s.tick();
        }
        assert!(!s.is_ended(), "untimed run does not end on its own");
        s.on_material_gained(MaterialId::Satori);
        assert!(s.is_ended());
        assert_eq!(s.result_tick(), Some(200));
    }

    #[test]
    fn hash_dash_blocks_on_end_satori_rush_does_not() {
        // Timed score-attacks block with a modal end-card; the untimed Satori
        // speedrun ends non-blocking so the player may keep playing.
        assert!(ScenarioState::new(hash_dash_def()).blocks_on_end());
        assert!(!ScenarioState::new(satori_rush_def()).blocks_on_end());
    }

    #[test]
    fn hash_dash_not_resumable_satori_rush_is() {
        assert!(!hash_dash_def().is_resumable());
        assert!(satori_rush_def().is_resumable());
    }

    #[test]
    fn official_games_have_fixed_arenas() {
        // Prague: BOTH official games launch into a fixed, fair, repeatable arena
        // (per-def `arena_seed`) so Hash Dash scores + Satori Rush times compare
        // across players — the "portal into the mod" model. Terrain *quality* of
        // these seeds is a playtest call; that they're PINNED is the invariant.
        assert!(hash_dash_def().arena_seed.is_some(), "Hash Dash needs a fixed arena");
        assert!(satori_rush_def().arena_seed.is_some(), "Satori Rush needs a fixed arena");
    }

    #[test]
    fn scenario_kind_tags_are_player_facing() {
        // The Stash Column card tag for each official Experience.
        assert_eq!(ScenarioKind::HashDash.label(), "Race");
        assert_eq!(ScenarioKind::SatoriRush.label(), "Speedrun");
    }

    #[test]
    fn official_games_lock_creative() {
        // Both official games are competitions — the pause menu must not offer a
        // mid-run switch to creative (flight / instant-break / infinite blocks).
        assert!(hash_dash_def().lock_creative, "Hash Dash must lock creative");
        assert!(satori_rush_def().lock_creative, "Satori Rush must lock creative");
    }

    #[test]
    fn resume_reconstructs_in_progress_run() {
        // On reload, the clock continues from the persisted world-clock.
        let s = ScenarioState::resume(satori_rush_def(), 1234, false, None);
        assert!(!s.is_ended());
        assert_eq!(s.elapsed_ticks(), 1234);
        assert_eq!(s.remaining_ticks(), 0); // untimed
    }

    #[test]
    fn resume_reconstructs_completed_run() {
        // A finished Satori Rush resumes showing its genesis result.
        let s = ScenarioState::resume(satori_rush_def(), 5000, true, Some(4242));
        assert!(s.is_ended());
        assert_eq!(s.result_tick(), Some(4242));
    }

    // ─── Feature-coverage challenges (2026-06-14) — Phase 2 objective model ───

    /// Build a challenge def carrying `objective` (no scoring, non-locking).
    fn challenge_def(objective: Objective) -> ScenarioDef {
        ScenarioDef {
            kind: ScenarioKind::Test,
            display_name: "C".to_string(),
            lock_creative: false,
            arena_mode: ArenaMode::Reuse,
            kit: vec![],
            objective,
            scoring: Scoring::None,
            arena_seed: None,
            tuning: ScenarioTuning::default(),
            world_type: None,
            game_mode: None,
            time_lock: None,
            weather_lock: None,
            mobs_enabled: None,
            trial_race: None,
            arena: None,
        }
    }

    #[test]
    fn action_objective_completes_after_count_matching_events() {
        let mut s = ScenarioState::new(challenge_def(Objective::Action {
            event: ChallengeEvent::CookAtCampfire,
            count: 3,
        }));
        assert_eq!(s.objective_progress(), Some((0, 3)));
        s.on_event(ChallengeEvent::CookAtCampfire);
        s.on_event(ChallengeEvent::CookAtCampfire);
        assert!(!s.is_ended(), "2 of 3 — not done yet");
        assert_eq!(s.objective_progress(), Some((2, 3)));
        s.on_event(ChallengeEvent::CookAtCampfire);
        assert!(s.is_ended(), "3rd cook completes the action");
        assert_eq!(s.objective_progress(), Some((3, 3)));
    }

    #[test]
    fn progress_summary_counts_completed_checklist_leaves() {
        // Wave 6 (challenge board) — a 2-item checklist reports (done, total)
        // leaves as the board's "X / Y" readout.
        let mut s = ScenarioState::new(challenge_def(Objective::Checklist {
            items: vec![
                Objective::Action { event: ChallengeEvent::CookAtCampfire, count: 1 },
                Objective::Action { event: ChallengeEvent::TameMob, count: 1 },
            ],
        }));
        assert_eq!(s.progress_summary(), (0, 2));
        s.on_event(ChallengeEvent::CookAtCampfire);
        assert_eq!(s.progress_summary(), (1, 2), "one of two checklist items done");
        s.on_event(ChallengeEvent::TameMob);
        assert_eq!(s.progress_summary(), (2, 2));
        // A passive objective has no leaves.
        let passive = ScenarioState::new(challenge_def(Objective::FreeRoam));
        assert_eq!(passive.progress_summary(), (0, 0));
    }

    #[test]
    fn action_objective_ignores_nonmatching_events() {
        let mut s = ScenarioState::new(challenge_def(Objective::Action {
            event: ChallengeEvent::TameMob,
            count: 1,
        }));
        s.on_event(ChallengeEvent::VendorSale);
        s.on_event(ChallengeEvent::CookAtCampfire);
        assert!(!s.is_ended(), "unrelated events must not advance a TameMob action");
        s.on_event(ChallengeEvent::TameMob);
        assert!(s.is_ended());
    }

    #[test]
    fn break_block_filter_matches_specific_block_only() {
        // A filtered BreakBlock counts only that block id; None would match any.
        let mut s = ScenarioState::new(challenge_def(Objective::Action {
            event: ChallengeEvent::BreakBlock { block: Some(crate::block::OAK_LOG) },
            count: 1,
        }));
        s.on_event(ChallengeEvent::BreakBlock { block: Some(crate::block::STONE) });
        assert!(!s.is_ended(), "breaking a different block must not count");
        s.on_event(ChallengeEvent::BreakBlock { block: Some(crate::block::OAK_LOG) });
        assert!(s.is_ended(), "breaking the target block counts");
    }

    #[test]
    fn break_block_none_filter_matches_any_block() {
        let mut s = ScenarioState::new(challenge_def(Objective::Action {
            event: ChallengeEvent::BreakBlock { block: None },
            count: 2,
        }));
        s.on_event(ChallengeEvent::BreakBlock { block: Some(crate::block::STONE) });
        s.on_event(ChallengeEvent::BreakBlock { block: Some(crate::block::DIRT) });
        assert!(s.is_ended(), "an unfiltered BreakBlock counts any block");
    }

    #[test]
    fn breed_animals_filter_matches_offspring_species_only() {
        // Task 19 — "Mule Maker" must NOT complete on an everyday cow calf; a
        // filtered BreedAnimals counts only babies of the target species.
        let mut s = ScenarioState::new(challenge_def(Objective::Action {
            event: ChallengeEvent::BreedAnimals { offspring: Some(crate::mob::MobType::Mule) },
            count: 1,
        }));
        s.on_event(ChallengeEvent::BreedAnimals { offspring: Some(crate::mob::MobType::Cow) });
        assert!(!s.is_ended(), "breeding a cow calf must not count toward a Mule trial");
        s.on_event(ChallengeEvent::BreedAnimals { offspring: Some(crate::mob::MobType::Mule) });
        assert!(s.is_ended(), "breeding the target species (Mule) counts");
    }

    #[test]
    fn breed_animals_none_filter_matches_any_offspring() {
        let mut s = ScenarioState::new(challenge_def(Objective::Action {
            event: ChallengeEvent::BreedAnimals { offspring: None },
            count: 1,
        }));
        s.on_event(ChallengeEvent::BreedAnimals { offspring: Some(crate::mob::MobType::Cow) });
        assert!(s.is_ended(), "an unfiltered BreedAnimals counts any offspring species");
    }

    #[test]
    fn source_turned_counts_only_the_source_the_objective_names() {
        // Wind, Copper & Electricity wave §4 — a wheel trial must not be finished
        // by a windmill that happens to be spinning somewhere else in the world
        // (and vice versa). `SourceTurned` has no "any" option for exactly this.
        let mut wheel = ScenarioState::new(challenge_def(Objective::Action {
            event: ChallengeEvent::SourceTurned { kind: TurningSource::WaterWheel },
            count: 1,
        }));
        wheel.on_event(ChallengeEvent::SourceTurned { kind: TurningSource::Windmill });
        assert!(!wheel.is_ended(), "a windmill must not finish a water-wheel objective");
        wheel.on_event(ChallengeEvent::SourceTurned { kind: TurningSource::WaterWheel });
        assert!(wheel.is_ended(), "the named source finishes it");

        let mut mill = ScenarioState::new(challenge_def(Objective::Action {
            event: ChallengeEvent::SourceTurned { kind: TurningSource::Windmill },
            count: 1,
        }));
        mill.on_event(ChallengeEvent::SourceTurned { kind: TurningSource::WaterWheel });
        assert!(!mill.is_ended(), "a water wheel must not finish a windmill objective");
        mill.on_event(ChallengeEvent::SourceTurned { kind: TurningSource::Windmill });
        assert!(mill.is_ended());
    }

    #[test]
    fn the_new_unit_events_each_count_only_themselves() {
        // The four unit-variant events added with the wave. Cheap, but it pins
        // that `matches` didn't collapse any two of them into one arm.
        use ChallengeEvent::*;
        let unit = [CompleteBuildGuide, SaveSkin, SpawnRig, SendFeedback];
        for want in unit {
            for got in unit {
                assert_eq!(
                    want.matches(&got),
                    want == got,
                    "{want:?} should match {got:?} only when they are the same event"
                );
            }
        }
    }

    #[test]
    fn sequence_advances_in_order_and_completes_on_last_step() {
        // Onboarding arc shape: break → craft → cook, strictly in order.
        let mut s = ScenarioState::new(challenge_def(Objective::Sequence {
            steps: vec![
                Objective::Action { event: ChallengeEvent::BreakBlock { block: None }, count: 1 },
                Objective::Action { event: ChallengeEvent::CraftItem, count: 1 },
                Objective::Action { event: ChallengeEvent::CookAtCampfire, count: 1 },
            ],
        }));
        // Step counter starts at step 1 of 3.
        assert_eq!(s.objective_progress(), Some((0, 3)));
        s.on_event(ChallengeEvent::BreakBlock { block: Some(crate::block::STONE) });
        assert_eq!(s.objective_progress(), Some((1, 3)), "step 1 done");
        s.on_event(ChallengeEvent::CraftItem);
        assert_eq!(s.objective_progress(), Some((2, 3)), "step 2 done");
        assert!(!s.is_ended());
        s.on_event(ChallengeEvent::CookAtCampfire);
        assert!(s.is_ended(), "last step completes the sequence");
        assert_eq!(s.objective_progress(), Some((3, 3)));
    }

    #[test]
    fn sequence_ignores_out_of_order_events() {
        // A later step's event must NOT count while an earlier step is pending.
        let mut s = ScenarioState::new(challenge_def(Objective::Sequence {
            steps: vec![
                Objective::Action { event: ChallengeEvent::BreakBlock { block: None }, count: 1 },
                Objective::Action { event: ChallengeEvent::CookAtCampfire, count: 1 },
            ],
        }));
        // Fire the SECOND step's event first — must be ignored (ordered arc).
        s.on_event(ChallengeEvent::CookAtCampfire);
        assert_eq!(s.objective_progress(), Some((0, 2)), "out-of-order event ignored");
        assert!(!s.is_ended());
        s.on_event(ChallengeEvent::BreakBlock { block: None });
        s.on_event(ChallengeEvent::CookAtCampfire);
        assert!(s.is_ended());
    }

    #[test]
    fn checklist_completes_in_any_order() {
        // Explorer "do A and B" — unordered.
        let mut s = ScenarioState::new(challenge_def(Objective::Checklist {
            items: vec![
                Objective::Action { event: ChallengeEvent::VendorSale, count: 1 },
                Objective::Action { event: ChallengeEvent::ClaimPlot, count: 1 },
            ],
        }));
        assert_eq!(s.objective_progress(), Some((0, 2)));
        // Complete the SECOND item first — order doesn't matter.
        s.on_event(ChallengeEvent::ClaimPlot);
        assert_eq!(s.objective_progress(), Some((1, 2)));
        assert!(!s.is_ended());
        s.on_event(ChallengeEvent::VendorSale);
        assert!(s.is_ended(), "all checklist items done, any order");
    }

    #[test]
    fn gain_material_event_matches_by_material() {
        let mut s = ScenarioState::new(challenge_def(Objective::Action {
            event: ChallengeEvent::GainMaterial { material: MaterialId::Satori },
            count: 1,
        }));
        s.on_event(ChallengeEvent::GainMaterial { material: MaterialId::Coal });
        assert!(!s.is_ended(), "wrong material must not count");
        s.on_event(ChallengeEvent::GainMaterial { material: MaterialId::Satori });
        assert!(s.is_ended());
    }

    #[test]
    fn challenge_objectives_show_a_completion_card() {
        // A coverage Challenge isn't a Timed score-attack (`blocks_on_end` is
        // false), but it IS launched as a discrete Trial, so it must finish with
        // the modal completion card ("Well done! Back to Trials / Try again") —
        // that card is what marks the lobby badge done. (Regression: this used
        // to assert non-blocking, which left the badge + completion flow dead.)
        // `challenge_def` is ScenarioKind::Test — make it a real Challenge (as
        // the explorer JSONs ship), which is what flips the completion card on.
        let mut def = challenge_def(Objective::Action {
            event: ChallengeEvent::VendorSale,
            count: 1,
        });
        def.kind = ScenarioKind::Challenge;
        let mut s = ScenarioState::new(def);
        s.on_event(ChallengeEvent::VendorSale);
        assert!(s.is_ended());
        assert!(!s.blocks_on_end(), "a Challenge isn't a timed score-attack");
        assert!(
            s.shows_blocking_end_card(),
            "but an ended Challenge shows the completion card"
        );
    }

    #[test]
    fn completed_challenge_records_result_tick() {
        let mut s = ScenarioState::new(challenge_def(Objective::Action {
            event: ChallengeEvent::VendorSale,
            count: 1,
        }));
        for _ in 0..17 {
            s.tick();
        }
        s.on_event(ChallengeEvent::VendorSale);
        assert!(s.is_ended());
        assert_eq!(s.result_tick(), Some(17), "completion tick is recorded");
    }

    #[test]
    fn ended_challenge_ignores_further_events() {
        let mut s = ScenarioState::new(challenge_def(Objective::Action {
            event: ChallengeEvent::VendorSale,
            count: 1,
        }));
        s.on_event(ChallengeEvent::VendorSale);
        assert!(s.is_ended());
        assert_eq!(s.objective_progress(), Some((1, 1)));
        // Further events must not over-count past completion.
        s.on_event(ChallengeEvent::VendorSale);
        assert_eq!(s.objective_progress(), Some((1, 1)));
    }

    #[test]
    fn passive_objectives_have_no_event_progress() {
        // Timed / FirstSatori / FreeRoam are not event-driven.
        for obj in [Objective::Timed { ticks: 10 }, Objective::FirstSatori, Objective::FreeRoam] {
            let mut s = ScenarioState::new(challenge_def(obj));
            assert_eq!(s.objective_progress(), None);
            s.on_event(ChallengeEvent::VendorSale); // no-op
            assert!(!s.is_ended(), "an unrelated event must not end a passive objective");
        }
    }

    #[test]
    fn action_sequence_checklist_defs_round_trip_json() {
        for objective in [
            Objective::Action { event: ChallengeEvent::TameMob, count: 1 },
            Objective::Action { event: ChallengeEvent::BreakBlock { block: Some(crate::block::STONE) }, count: 5 },
            Objective::Action {
                event: ChallengeEvent::BreedAnimals { offspring: Some(crate::mob::MobType::Mule) },
                count: 1,
            },
            Objective::Sequence {
                steps: vec![
                    Objective::Action { event: ChallengeEvent::CraftItem, count: 1 },
                    Objective::Action { event: ChallengeEvent::CookAtCampfire, count: 1 },
                ],
            },
            Objective::Checklist {
                items: vec![
                    Objective::Action { event: ChallengeEvent::WorkshopPublish, count: 1 },
                    Objective::Action { event: ChallengeEvent::VendorSale, count: 1 },
                ],
            },
        ] {
            let def = challenge_def(objective);
            let json = def.to_json().unwrap();
            assert_eq!(load_scenario_def(json.as_bytes()).unwrap(), def);
        }
    }

    #[test]
    fn offers_onboarding_only_to_first_time_players() {
        // Owner decision (2026-06-15): the onboarding arc is offered ONLY when
        // there's no sign the player has played before — no local saved worlds
        // AND nothing in their Stash. Returning players (any local world OR any
        // stash world) just get the normal lobby; the prompt never shows.
        assert!(should_offer_onboarding(0, 0), "true first-timer → offer the arc");
        assert!(!should_offer_onboarding(1, 0), "has a local world → returning");
        assert!(!should_offer_onboarding(0, 1), "has a stash world → returning");
        assert!(!should_offer_onboarding(3, 2), "clearly a returning player");
    }

    #[test]
    fn existing_scenarios_unaffected_by_new_variants() {
        // The new objective variants must not change how the two shipped games
        // deserialise — back-compat invariant.
        assert_eq!(hash_dash_def().objective, Objective::Timed { ticks: 3600 });
        assert_eq!(satori_rush_def().objective, Objective::FirstSatori);
    }

    // ─── Phase 4 — bundled challenge content ───

    #[test]
    fn bundled_challenge_pack_parses_and_round_trips() {
        // Every authored challenge JSON must parse, be tagged Challenge, and
        // survive a re-serialise (guards a hand-authoring typo at build time).
        let pack = challenge_pack();
        assert_eq!(pack.len(), 1 + EXPLORER_CHALLENGES.len(), "onboarding + explorer cards");
        for def in &pack {
            assert_eq!(def.kind, ScenarioKind::Challenge, "{} must be a Challenge", def.display_name);
            assert_eq!(load_scenario_def(def.to_json().unwrap().as_bytes()).unwrap(), *def);
        }
    }

    #[test]
    fn onboarding_is_an_ordered_arc_of_action_leaves() {
        let def = load_scenario_def(ONBOARDING_JSON).unwrap();
        match def.objective {
            Objective::Sequence { steps } => {
                assert!(steps.len() >= 4, "onboarding arc is a multi-step warm-up");
                // Every step is an Action leaf (v1 invariant the runner relies on).
                assert!(steps.iter().all(|s| matches!(s, Objective::Action { .. })));
            }
            other => panic!("onboarding must be a Sequence, got {other:?}"),
        }
    }

    #[test]
    fn each_explorer_challenge_is_event_driven_and_resolvable_by_name() {
        // The fun-first redesign (2026-06-24) uses ordered Sequences + unordered
        // Checklists (gather → craft → use mini-arcs), not just single Actions.
        // Every explorer must still be an EVENT-DRIVEN objective (so it can
        // actually complete + show progress) and resolve via `/scenario <name>`.
        for (name, bytes) in EXPLORER_CHALLENGES {
            let def = load_scenario_def(bytes)
                .unwrap_or_else(|e| panic!("explorer {name} parse: {e}"));
            assert!(
                matches!(
                    def.objective,
                    Objective::Action { .. }
                        | Objective::Sequence { .. }
                        | Objective::Checklist { .. }
                        | Objective::Timed { .. } // the Scavenger — a timed variety score-attack
                ),
                "explorer {name} must be an Action/Sequence/Checklist or a Timed score-attack"
            );
            // A Timed explorer is the Scavenger: it must score by inventory variety
            // (the timer alone has no result without it).
            if matches!(def.objective, Objective::Timed { .. }) {
                assert_eq!(
                    def.scoring,
                    Scoring::InventoryVariety,
                    "{name}: a timed explorer must score by InventoryVariety"
                );
            }
            // Resolvable via the `/scenario <name>` surface.
            assert!(named_builtin_def(name).is_some(), "{name} must resolve");
        }
    }

    #[test]
    fn every_bundled_challenge_has_authored_help() {
        // The complaint that started this: a trial with no explanation. Every
        // bundled challenge must have a one-line tagline AND a detailed how-to,
        // so the dropdown line + the "What to do?" pop-up are never blank.
        for (name, _display) in challenge_listing() {
            let (tagline, how_to) = challenge_help(name);
            assert!(!tagline.trim().is_empty(), "{name}: needs a one-line tagline");
            assert!(!how_to.trim().is_empty(), "{name}: needs a detailed how-to");
        }
    }

    #[test]
    fn challenge_help_resolves_by_display_name() {
        // The in-game objective pop-up (H) has only the active scenario's
        // display_name to go on; the reverse-lookup must find every bundled
        // challenge's how-to so the panel is never blank in-world.
        for def in challenge_pack() {
            let (tagline, how_to) = challenge_help_for_display(&def.display_name);
            assert!(!tagline.is_empty(), "{}: reverse-lookup tagline", def.display_name);
            assert!(!how_to.is_empty(), "{}: reverse-lookup how_to", def.display_name);
        }
    }

    #[test]
    fn trial_recipe_hints_resolve_to_catalogue_cards() {
        // A hint that doesn't resolve (or has no recipe card) would make the
        // inventory auto-pin silently show nothing.
        for (token, _display) in challenge_listing() {
            for name in trial_recipe_hints(token) {
                let stack = crate::commands::builtins::give::resolve_item(name, 1)
                    .unwrap_or_else(|e| panic!("hint '{name}' for '{token}' won't resolve: {e}"));
                assert!(
                    crate::crafting_catalogue::recipe_index_for_output(&stack.item).is_some(),
                    "hint '{name}' for '{token}' has no recipe card",
                );
            }
        }
    }

    #[test]
    fn objective_tasks_nonempty_for_every_challenge() {
        // Every trial's "What to do" popup must produce at least one task line.
        for (token, _display) in challenge_listing() {
            let tasks = trial_tasks_for_token(token).expect("challenge tasks resolve");
            assert!(!tasks.lines.is_empty(), "{token} produced an empty task list");
        }
        // Onboarding is an ordered Sequence → numbered.
        assert!(
            trial_tasks_for_token("onboarding").unwrap().ordered,
            "onboarding tasks are ordered",
        );
    }

    #[test]
    fn authored_task_labels_align_with_objective_steps() {
        // An authored label list must line up 1:1 with the auto-derived steps,
        // else objective_tasks silently ignores it (and the player gets the
        // generic wording). Every authored trial is checked.
        for (token, _display) in challenge_listing() {
            let labels = trial_task_labels(token);
            if labels.is_empty() {
                continue; // intentional auto-derive
            }
            let def = named_builtin_def(token).unwrap();
            let auto = objective_tasks(&def, &[]);
            assert_eq!(
                labels.len(),
                auto.lines.len(),
                "{token}: {} authored labels vs {} objective steps",
                labels.len(),
                auto.lines.len(),
            );
        }
    }

    #[test]
    fn every_challenge_kit_and_arena_name_resolves() {
        // A typo'd item/block/mob name would ship a BROKEN kit — silently
        // dropped by provision_all_players / apply_arena_setup, leaving the
        // challenge uncompletable. Assert every authored name in every bundled
        // challenge resolves to a real item / placeable block / spawnable mob.
        for def in challenge_pack() {
            for kit in &def.kit {
                assert!(
                    crate::commands::builtins::give::resolve_item(&kit.name, kit.count).is_ok(),
                    "{}: kit item '{}' must resolve via /give",
                    def.display_name,
                    kit.name,
                );
            }
            if let Some(arena) = &def.arena {
                for b in &arena.blocks {
                    let stack = crate::commands::builtins::give::resolve_item(&b.block, 1)
                        .unwrap_or_else(|e| {
                            panic!("{}: arena block '{}': {e}", def.display_name, b.block)
                        });
                    assert!(
                        stack.item.as_block().is_some(),
                        "{}: arena '{}' must be a placeable block",
                        def.display_name,
                        b.block,
                    );
                }
                for m in &arena.mobs {
                    assert!(
                        crate::mob::MobType::from_name(&m.mob).is_some(),
                        "{}: arena mob '{}' must resolve via /spawn",
                        def.display_name,
                        m.mob,
                    );
                }
            }
        }
    }

    #[test]
    fn onboarding_arc_completes_when_driven_in_order() {
        // End-to-end through the real bundled def: drive each step's event in
        // order and confirm the arc completes exactly on the last step (cook).
        // Mirrors the authored sequence break×4 → craft → place×6 → cook.
        let mut s = ScenarioState::new(load_scenario_def(ONBOARDING_JSON).unwrap());
        for _ in 0..4 { s.on_event(ChallengeEvent::BreakBlock { block: Some(crate::block::STONE) }); }
        s.on_event(ChallengeEvent::CraftItem);
        for _ in 0..6 { s.on_event(ChallengeEvent::PlaceBlock { block: Some(crate::block::DIRT) }); }
        assert!(!s.is_ended(), "not done until the campfire-cook capstone");
        s.on_event(ChallengeEvent::CookAtCampfire);
        assert!(s.is_ended(), "cooking the first meal completes the onboarding arc");
    }

    #[test]
    fn plumber_needs_both_fill_and_empty() {
        // Regression: the bucket trial is fill THEN empty (2 UseBucket events).
        // The engine must fire UseBucket on BOTH ends (the empty path used to be
        // silent, leaving this stuck at 1/2). One event must NOT complete it.
        let mut s = ScenarioState::new(named_builtin_def("bucket").unwrap());
        s.on_event(ChallengeEvent::UseBucket);
        assert!(!s.is_ended(), "one bucket use (fill) must not finish the Plumber");
        s.on_event(ChallengeEvent::UseBucket);
        assert!(s.is_ended(), "fill + empty (2 UseBucket) completes the Plumber");
    }

    #[test]
    fn breach_and_clear_completes_in_either_order() {
        // Regression: breach-and-clear is a Checklist, NOT an ordered Sequence —
        // a lone brigand isn't reliably penned, so killing it before the blast
        // must still count. Both orders must complete.
        for events in [
            [ChallengeEvent::Detonate, ChallengeEvent::KillMob],
            [ChallengeEvent::KillMob, ChallengeEvent::Detonate],
        ] {
            let mut s = ScenarioState::new(named_builtin_def("breach-and-clear").unwrap());
            s.on_event(events[0]);
            assert!(!s.is_ended(), "one of the two breach steps must not finish it");
            s.on_event(events[1]);
            assert!(s.is_ended(), "blast + kill (any order) completes breach-and-clear");
        }
    }

    #[test]
    fn scavenger_is_a_three_minute_variety_score_attack() {
        let def = named_builtin_def("scavenger").expect("scavenger must resolve");
        assert_eq!(def.kind, ScenarioKind::Challenge);
        assert_eq!(def.objective, Objective::Timed { ticks: 3600 }, "3 min @ 20 TPS");
        assert_eq!(def.scoring, Scoring::InventoryVariety);
        assert_eq!(def.game_mode.as_deref(), Some("survival"));
        assert!(def.kit.is_empty(), "Scavenger starts with nothing");
    }

    #[test]
    fn variety_score_is_cumulative_and_only_grows() {
        let mut s = ScenarioState::new(named_builtin_def("scavenger").unwrap());
        s.accumulate_variety([1, 2, 3]);
        assert_eq!(s.score(), 3);
        // Holding FEWER kinds now (used some up) must NOT lower the score —
        // everything ever held still counts.
        s.accumulate_variety([2]);
        assert_eq!(s.score(), 3, "using items up never lowers the cumulative score");
        // New kinds add; a repeat of an already-seen kind doesn't double-count.
        s.accumulate_variety([3, 4, 5]);
        assert_eq!(s.score(), 5, "new kinds add, repeats don't");
        for _ in 0..3599 {
            assert!(!s.tick(), "the run is not over before 3 minutes");
        }
        assert!(s.tick(), "ends on the 3600th tick (3:00)");
        assert!(s.is_ended());
        s.accumulate_variety([6, 7]);
        assert_eq!(s.score(), 5, "nothing accumulates after the buzzer");
    }

    #[test]
    fn scavenger_clears_inventory_to_start_with_nothing() {
        use crate::item::ItemStack;
        use crate::player_slot::PlayerSlot;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        players[0]
            .inventory
            .set_slot(0, Some(ItemStack::new_block(crate::block::DIAMOND_BLOCK, 64)));
        provision_all_players(&mut players, &named_builtin_def("scavenger").unwrap());
        assert_eq!(
            players[0].inventory.distinct_item_kinds(),
            0,
            "a timed Challenge (Scavenger) wipes the bag so you start with nothing"
        );
    }

    #[test]
    fn every_trial_has_satoshi_voice() {
        // The fun-first redesign promises Satoshi speaks on EVERY trial. Each
        // bundled challenge token AND each ⚡ race id must have a non-empty
        // intro + hint, and the in-game reverse-lookup must find the challenge's.
        for (name, _display) in challenge_listing() {
            let (intro, hint) = trial_satoshi(name);
            assert!(!intro.trim().is_empty(), "{name}: Satoshi needs an intro");
            assert!(!hint.trim().is_empty(), "{name}: Satoshi needs a hint");
        }
        for def in challenge_pack() {
            let (intro, hint) = trial_satoshi_for_display(&def.display_name);
            assert!(!intro.is_empty(), "{}: reverse-lookup intro", def.display_name);
            assert!(!hint.is_empty(), "{}: reverse-lookup hint", def.display_name);
        }
        for race in crate::trials::CATALOG {
            let (intro, hint) = trial_satoshi(race.id);
            assert!(!intro.trim().is_empty(), "race {}: Satoshi intro", race.id);
            assert!(!hint.trim().is_empty(), "race {}: Satoshi hint", race.id);
        }
    }

    #[test]
    fn trial_text_has_no_money_or_earning_words() {
        // Compliance: no trial text (tagline / how-to / Satoshi intro+hint) may
        // frame play as money/earning. Mirrors satoshi.rs's corpus guard, over
        // the Trials corpus. "Trade/barter/swap" is fine; "buy/sell/earn/sats"
        // is not.
        // Whole-WORD banned terms (so "learn" doesn't trip "earn", etc.).
        const BANNED: &[&str] = &[
            "sats", "bitcoin", "btc", "earn", "earning", "earned", "payout", "payouts",
            "wallet", "money", "cash", "cashback", "prize", "prizes", "sell", "buy",
            "lightning", "wages", "salary",
        ];
        let mut corpus = String::new();
        for (name, _d) in challenge_listing() {
            let (t, h) = challenge_help(name);
            let (i, hi) = trial_satoshi(name);
            corpus.push_str(&format!(" {t} {h} {i} {hi} "));
        }
        for race in crate::trials::CATALOG {
            corpus.push_str(&format!(" {} {} ", race.blurb, race.how_to));
            let (i, hi) = trial_satoshi(race.id);
            corpus.push_str(&format!(" {i} {hi} "));
        }
        // Tokenise into lowercase words (letters + apostrophe) and check membership,
        // so banned terms only match as whole words.
        let words: std::collections::HashSet<String> = corpus
            .to_lowercase()
            .split(|c: char| !(c.is_ascii_alphabetic() || c == '\''))
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        for w in BANNED {
            assert!(!words.contains(*w), "trial text contains banned money/earning word: {w:?}");
        }
    }
}
