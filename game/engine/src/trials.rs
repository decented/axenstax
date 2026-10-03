//! Trials — the Challenge Engine's first family (⚡ Race). Phase 1 PURE CORE:
//! ghost recording/replay, personal-best tracking, and race timing. The
//! playable loop (a race objective on `scenario`, the ghost-avatar render, the
//! Trials browser, bundled trial defs) builds on these primitives.
//!
//! Design + roadmap: `docs/superpowers/specs/2026-06-23-challenge-engine-vision.md`
//! (Phase 1 = prove the loop: a Race trial with personal best + your-own-ghost).
//!
//! A ghost is just a recording of the runner's transform per tick, replayed
//! visually — NOT a re-simulation. So it needs no determinism, works in the
//! fixed read-only Adventure arenas Race trials use, and dodges the world-
//! mutability problem (you can only place blocks in editable worlds; races
//! aren't). Self-custodied + portable (lives in the player's trials store).

use serde::{Deserialize, Serialize};

/// One recorded instant of the runner: position + look direction. ~20 bytes; at
/// 20 TPS a 60-second run is ~1200 frames ≈ 24 KB — cheap to store and share.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GhostFrame {
    pub pos: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
}

/// A full run recording: one frame per simulation tick from the start line.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GhostRecording {
    pub frames: Vec<GhostFrame>,
}

/// Cap on recorded ghost frames (~10 min at 20 TPS). A run longer than this
/// stops appending — the stored ghost stays bounded in memory AND on disk /
/// localStorage (a new PB serialises the whole recording), and replay simply
/// holds at the cap. Real races finish far under this.
pub const MAX_GHOST_FRAMES: usize = 12_000;

impl GhostRecording {
    /// No production caller constructs via `new()` (the `TrialRun.recording`
    /// field starts via `Default`) — tested directly.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new() -> Self {
        Self { frames: Vec::new() }
    }

    pub fn record(&mut self, f: GhostFrame) {
        if self.frames.len() >= MAX_GHOST_FRAMES {
            return; // bounded — an over-long / idle run can't grow without limit
        }
        self.frames.push(f);
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// The ghost's transform `tick` ticks after the start. Once the recording is
    /// exhausted it HOLDS on the final frame (the ghost crosses the line and
    /// waits there) rather than vanishing. `None` only for an empty recording.
    pub fn sample(&self, tick: usize) -> Option<GhostFrame> {
        if self.frames.is_empty() {
            return None;
        }
        Some(self.frames[tick.min(self.frames.len() - 1)])
    }
}

/// The best result on one trial: the time (in ticks; lower = better) + the ghost
/// of that run to chase next time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrialBest {
    pub ticks: u32,
    #[serde(default)]
    pub ghost: GhostRecording,
}

/// Personal bests across all trials, keyed by stable trial id. Serialises to the
/// player's local trials store (native file / web persistence) — self-custodied
/// + portable, so a kid's records are theirs and travel with them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrialBests {
    /// Timed (Race) trials → best time + ghost, keyed by trial id.
    pub bests: std::collections::BTreeMap<String, TrialBest>,
    /// Binary (Challenge) trials you've completed, keyed by display name (what
    /// the lobby list shows). `#[serde(default)]` so older stores load clean.
    #[serde(default)]
    pub completed_challenges: std::collections::BTreeSet<String>,
    /// Campaign G — imported rival ghosts, keyed by race id (one per race,
    /// import replaces). `#[serde(default)]` so pre-rival stores load clean.
    #[serde(default)]
    pub rivals: std::collections::BTreeMap<String, RivalGhost>,
}

impl TrialBests {
    /// Record a finished run. Returns `true` if it's a NEW best (and stores it),
    /// `false` if the existing best was as good or better (left untouched).
    pub fn submit(&mut self, trial_id: &str, ticks: u32, ghost: GhostRecording) -> bool {
        let improved = match self.bests.get(trial_id) {
            Some(b) => ticks < b.ticks,
            None => true,
        };
        if improved {
            self.bests.insert(trial_id.to_string(), TrialBest { ticks, ghost });
        }
        improved
    }

    pub fn best(&self, trial_id: &str) -> Option<&TrialBest> {
        self.bests.get(trial_id)
    }

    /// Mark a binary (Challenge) trial complete by its display name. Returns
    /// `true` if it was newly marked (so the caller can save once).
    pub fn mark_challenge_done(&mut self, display_name: &str) -> bool {
        self.completed_challenges.insert(display_name.to_string())
    }

    /// Whether a binary trial with this display name has been completed.
    pub fn is_challenge_done(&self, display_name: &str) -> bool {
        self.completed_challenges.contains(display_name)
    }

    /// Serialise the store (pure; the platform load/save wraps this).
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Parse a store; tolerant — malformed/empty JSON → empty (never panics), so
    /// a corrupt file just means "no bests yet", not a crash.
    pub fn from_json(json: &str) -> Self {
        serde_json::from_str(json).unwrap_or_default()
    }

    /// Fold another store into this one, keeping the better of each record.
    /// Used when a `.axeprofile` bundle brings a player's browser Trials
    /// records across to the desktop app: nothing they've already done here is
    /// lost, and nothing they did in the browser is thrown away.
    ///
    /// "Better", per field:
    /// - **bests** — fewer ticks wins (a race time; lower is faster). Ties keep
    ///   what's already here, so a re-import is a no-op.
    /// - **completed_challenges** — a union: done anywhere is done.
    /// - **rivals** — the faster rival ghost wins (it's the one worth chasing);
    ///   a race with no local rival takes the incoming one.
    ///
    /// Pure: no I/O, so the caller decides when to `save()`.
    pub fn merge(&mut self, other: &TrialBests) {
        for (id, incoming) in &other.bests {
            let take = match self.bests.get(id) {
                Some(mine) => incoming.ticks < mine.ticks,
                None => true,
            };
            if take {
                self.bests.insert(id.clone(), incoming.clone());
            }
        }
        for name in &other.completed_challenges {
            self.completed_challenges.insert(name.clone());
        }
        for (id, incoming) in &other.rivals {
            let take = match self.rivals.get(id) {
                Some(mine) => incoming.ticks < mine.ticks,
                None => true,
            };
            if take {
                self.rivals.insert(id.clone(), incoming.clone());
            }
        }
    }
}

/// Version written into (and accepted from) `.axeghost` share files.
pub const GHOST_FILE_VERSION: u32 = 1;
/// File extension for shared ghosts (native save dialog filter + web filename).
pub const GHOST_FILE_EXT: &str = "axeghost";

/// Campaign G (2026-07-05) — a shareable ghost file (`.axeghost`, JSON). A kid
/// exports their best run and hands the file to a friend however they like;
/// no relay, no upload, nothing collected (Regulatory Red Lines). `author` is
/// a free-text label written by the exporter — a HUD caption, trusted by
/// nobody, never an identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GhostShareFile {
    pub version: u32,
    pub trial_id: String,
    pub trial_name: String,
    pub ticks: u32,
    #[serde(default)]
    pub author: Option<String>,
    pub ghost: GhostRecording,
}

impl GhostShareFile {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Parse + validate a share file. Errors are player-readable (they land in
    /// a toast).
    ///
    /// Bounds (review fix 2026-07-05): local recordings cap at
    /// [`MAX_GHOST_FRAMES`] precisely so the trials store stays small; an
    /// import must honour the same cap or one oversized/corrupt file bloats
    /// the single store (and on web, silently breaks ALL trials persistence
    /// once localStorage hits quota). The byte guard runs BEFORE parsing so a
    /// giant file can't even allocate.
    pub fn from_json(json: &str) -> Result<Self, String> {
        // ~100 bytes/frame in JSON; 12k frames ≈ 1.2 MB. 4 MiB = generous slack.
        const MAX_GHOST_FILE_BYTES: usize = 4 * 1024 * 1024;
        if json.len() > MAX_GHOST_FILE_BYTES {
            return Err("That ghost file is too big to be a real run.".to_string());
        }
        let f: Self =
            serde_json::from_str(json).map_err(|_| "That's not a ghost file.".to_string())?;
        if f.version > GHOST_FILE_VERSION {
            return Err(
                "This ghost file is from a newer version of the game — update to race it."
                    .to_string(),
            );
        }
        if f.ghost.frames.is_empty() {
            return Err("This ghost file has no run in it.".to_string());
        }
        if f.ghost.frames.len() > MAX_GHOST_FRAMES {
            return Err("That ghost file is too big to be a real run.".to_string());
        }
        Ok(f)
    }
}

/// An imported rival ghost for one race — one per race, importing replaces.
/// Persisted alongside the personal bests in the trials store.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RivalGhost {
    pub label: String,
    pub ticks: u32,
    pub ghost: GhostRecording,
}

/// The finish-panel line for a race run against a rival ghost: beat their
/// time → celebrate, else report what you're chasing.
pub fn rival_outcome_line(rival: &RivalGhost, my_ticks: u32) -> String {
    if my_ticks < rival.ticks {
        format!("You beat {}'s ghost!", rival.label)
    } else {
        format!("{}'s ghost: {}", rival.label, format_time(rival.ticks))
    }
}

/// Live state of a race attempt. Counts UP from the start line until the finish
/// is crossed, recording a ghost frame each tick. Distinct from
/// `scenario::Objective::Timed`, which counts DOWN to a fixed deadline.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RaceRun {
    pub elapsed_ticks: u32,
    pub started: bool,
    pub finished: bool,
    pub recording: GhostRecording,
}

impl RaceRun {
    /// Cross the start line: reset the clock + recording and begin.
    pub fn start(&mut self) {
        *self = RaceRun { started: true, ..Default::default() };
    }

    /// Advance one tick, recording the runner's frame. No-op until started or
    /// after finishing, so frames map 1:1 to elapsed ticks for ghost replay.
    pub fn tick(&mut self, frame: GhostFrame) {
        if !self.started || self.finished {
            return;
        }
        self.recording.record(frame);
        self.elapsed_ticks = self.elapsed_ticks.saturating_add(1);
    }

    /// Cross the finish line. Returns the finishing time in ticks, or `None` if
    /// the run wasn't actually started.
    pub fn finish(&mut self) -> Option<u32> {
        if !self.started || self.finished {
            return None;
        }
        self.finished = true;
        Some(self.elapsed_ticks)
    }
}

/// Whether `pos` is inside the axis-aligned box `[min, max]` (a start/finish
/// trigger volume, world coords). Inclusive — touching a face counts. The live
/// finish check (`reached`, below) uses a point+radius test instead, so this
/// box variant has no caller — tested directly.
#[cfg_attr(not(test), allow(dead_code))]
pub fn in_volume(pos: [f32; 3], min: [f32; 3], max: [f32; 3]) -> bool {
    (0..3).all(|i| pos[i] >= min[i] && pos[i] <= max[i])
}

/// Whether the runner at `pos` has reached `finish` — horizontal distance only
/// (Y is ignored, so a marker pillar's exact height doesn't matter). `radius` in
/// blocks.
pub fn reached(pos: [f32; 3], finish: [f32; 3], radius: f32) -> bool {
    let dx = pos[0] - finish[0];
    let dz = pos[2] - finish[2];
    dx * dx + dz * dz <= radius * radius
}

/// A bundled ⚡ Race trial. The course is `start_xz → finish_xz`; Y snaps to the
/// surface at launch, so the markers + teleport land on the ground in any world
/// (feels best in a flat / Blank-Canvas world). Phase 1 launch set; expand the
/// `CATALOG` freely — adding a row is a whole new trial.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrialDef {
    pub id: &'static str,
    pub name: &'static str,
    /// One-line "what is it" shown in the Trials dropdown.
    pub blurb: &'static str,
    /// Detailed "how to do it" shown in the info (ⓘ) pop-up.
    pub how_to: &'static str,
    pub start_xz: [i32; 2],
    pub finish_xz: [i32; 2],
    pub finish_radius: f32,
}

/// The bundled Race trials — an escalating roster, all surface-snapped so they
/// run in any world (cleanest in a flat / Blank-Canvas one). One active at a
/// time; they all start at the origin so markers never collide. The fun is your
/// own ghost + clock, so the course needn't be globally fair. Distances + axes
/// vary to keep them feeling distinct; richer families (Build / Survive / Solve)
/// arrive in later Challenge-Engine phases.
pub const CATALOG: &[TrialDef] = &[
    TrialDef {
        id: "sprint",
        name: "The Sprint",
        blurb: "Pure speed — 60 blocks flat out, race the ghost of your best run.",
        how_to: "You're on the start line. The moment you move, the clock starts and a \
                 faint ghost-runner of your best-ever run leaps off the line beside you. \
                 Hold the forward key and sprint in a dead-straight line to the bright \
                 yellow finish beacon 60 blocks ahead — beat that ghost across the line. \
                 Nothing to dodge: it's all about your reaction off the line and never \
                 letting up. Run it again and again to shave off tenths.",
        start_xz: [0, 0],
        finish_xz: [0, 60],
        finish_radius: 2.5,
    },
    TrialDef {
        id: "cross-country",
        name: "Cross Country",
        blurb: "About 225 blocks over rough country — beat the ghost with the smartest line, not just speed.",
        how_to: "The finish beacon is about 225 blocks away as the crow flies, across open, \
                 bumpy country — and the land is NOT flat, so the hills make you walk it \
                 further than that. Your ghost runs the line you took last time, so the way \
                 to beat it is a better line: skirt around the steep hills instead of \
                 grinding over them, hop the small dips, and keep your momentum. Hold \
                 forward, steer with the mouse, and chase the yellow beacon. Every run you \
                 read the land a little better and carve seconds off.",
        start_xz: [0, 0],
        finish_xz: [200, 100],
        finish_radius: 3.5,
    },
    TrialDef {
        id: "marathon",
        name: "Marathon",
        blurb: "~1000 blocks of endurance and route-finding — a whole different game from the sprint.",
        how_to: "The big one: the finish beacon sits roughly 1000 blocks out. This isn't a \
                 sprint — it's a long haul where holding a smooth, unbroken line beats any \
                 single burst of speed. Hold forward, steer wide around obstacles, and stay \
                 locked on the beacon. Your ghost ran the whole distance too, so the win is \
                 a cleaner, straighter route start to finish. Settle into a rhythm and chip \
                 away at your best time.",
        start_xz: [0, 0],
        finish_xz: [700, 700],
        finish_radius: 5.0,
    },
];

/// Look up a bundled trial by id.
pub fn find_def(id: &str) -> Option<&'static TrialDef> {
    CATALOG.iter().find(|d| d.id == id)
}

/// The five styles of Trial. Each has an icon shown immediately before the
/// trial's name in the lobby (and named in its description) so a player can tell
/// at a glance what KIND of thing a trial is — without the list being split into
/// separate sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrialType {
    /// 🏁 Movement & speed — run a course, beat your ghost.
    Race,
    /// 🔨 Gather, craft, smelt, build — make things.
    Make,
    /// ⚡ Electricity, logic, pistons, rails, machines — wire it up.
    Tech,
    /// ⚔ Combat, explosives, hazards — face the danger.
    Brave,
    /// 🌱 Farming, animals, fishing, riding, trade — tend the living world.
    Tend,
}

impl TrialType {
    /// Emoji shown immediately before the trial name (and in its description).
    pub fn icon(self) -> &'static str {
        match self {
            TrialType::Race => "\u{1F3C1}",  // 🏁 chequered flag
            TrialType::Make => "\u{1F528}",  // 🔨 hammer
            TrialType::Tech => "\u{26A1}",   // ⚡ high voltage
            TrialType::Brave => "\u{2694}",  // ⚔ crossed swords
            TrialType::Tend => "\u{1F331}",  // 🌱 seedling
        }
    }

    /// Short style name shown in each trial's description.
    pub fn label(self) -> &'static str {
        match self {
            TrialType::Race => "Race",
            TrialType::Make => "Make",
            TrialType::Tech => "Tech",
            TrialType::Brave => "Brave",
            TrialType::Tend => "Tend",
        }
    }

    /// Signature colour (RGB) for this style's icon — kept as a plain triple so
    /// `trials` stays free of any UI-toolkit dependency; the menu converts it to
    /// its own colour type. Five clearly-distinct hues so a glance tells them
    /// apart.
    pub fn color_rgb(self) -> [u8; 3] {
        match self {
            TrialType::Race => [232, 86, 76],   // red
            TrialType::Make => [232, 150, 58],  // orange
            TrialType::Tech => [240, 205, 70],  // gold
            TrialType::Brave => [186, 130, 224], // violet
            TrialType::Tend => [120, 200, 110], // green
        }
    }
}

/// Resolve a lobby row key ("race:&lt;id&gt;" / "ch:&lt;token&gt;") to its
/// [`TrialType`] by looking it up in [`TRIAL_ORDER`]. Used by the "What to do"
/// popup so it can show the same coloured icon.
pub fn type_for_key(key: &str) -> Option<TrialType> {
    TRIAL_ORDER
        .iter()
        .find(|(_, tref)| match tref {
            TrialRef::Race(id) => key.strip_prefix("race:") == Some(*id),
            TrialRef::Challenge(tok) => key.strip_prefix("ch:") == Some(*tok),
        })
        .map(|(t, _)| *t)
}

/// One entry in the unified Trials list: a Race (id into [`CATALOG`]) or a
/// Challenge (token accepted by `scenario::named_builtin_def`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrialRef {
    Race(&'static str),
    Challenge(&'static str),
}

/// The lobby Trials list as ONE progression — easiest / first-session trials at
/// the top, deepest systems at the bottom — with Races and Challenges
/// interleaved (no Race-vs-Challenge split). Each entry carries its [`TrialType`]
/// so the row shows the style icon before the name and names the style in the
/// description. Ordering is "reasonable, not perfect": where a trial leans on a
/// skill, an easier trial that teaches it comes first (mine → craft → smelt →
/// armour-up; power → logic-gate → generator). `trial_order_covers_every_trial`
/// guards that this stays in sync with `CATALOG` + the challenge set.
pub const TRIAL_ORDER: &[(TrialType, TrialRef)] = &[
    (TrialType::Make, TrialRef::Challenge("onboarding")),
    (TrialType::Make, TrialRef::Challenge("mine")),
    (TrialType::Make, TrialRef::Challenge("craft")),
    (TrialType::Race, TrialRef::Race("sprint")),
    (TrialType::Make, TrialRef::Challenge("build")),
    (TrialType::Make, TrialRef::Challenge("smelt")),
    (TrialType::Make, TrialRef::Challenge("cook-three")),
    (TrialType::Tend, TrialRef::Challenge("eat")),
    (TrialType::Tend, TrialRef::Challenge("harvest")),
    (TrialType::Make, TrialRef::Challenge("bucket")),
    (TrialType::Tend, TrialRef::Challenge("fish")),
    (TrialType::Tend, TrialRef::Challenge("tame-wolf")),
    (TrialType::Tend, TrialRef::Challenge("friend-in-need")),
    (TrialType::Tend, TrialRef::Challenge("ride")),
    (TrialType::Race, TrialRef::Challenge("roadrunner")),
    (TrialType::Tend, TrialRef::Challenge("farmhand")),
    (TrialType::Tend, TrialRef::Challenge("rancher")),
    (TrialType::Tend, TrialRef::Challenge("mule-maker")),
    (TrialType::Race, TrialRef::Race("cross-country")),
    (TrialType::Tend, TrialRef::Challenge("vendor-sale")),
    (TrialType::Tend, TrialRef::Challenge("claim-plot")),
    (TrialType::Make, TrialRef::Challenge("scavenger")),
    (TrialType::Make, TrialRef::Challenge("dye")),
    (TrialType::Tend, TrialRef::Challenge("fish-feast")),
    (TrialType::Tend, TrialRef::Challenge("funny-farm")),
    (TrialType::Brave, TrialRef::Challenge("kill")),
    (TrialType::Brave, TrialRef::Challenge("armour-up")),
    (TrialType::Tech, TrialRef::Challenge("rail-rider")),
    (TrialType::Tech, TrialRef::Challenge("power")),
    (TrialType::Tech, TrialRef::Challenge("piston")),
    (TrialType::Tech, TrialRef::Challenge("logic-gate")),
    (TrialType::Tech, TrialRef::Challenge("generator")),
    // Wind, Copper & Electricity wave §4 — the copper dig comes FIRST (it is
    // where every cable in the three trials above actually comes from), then
    // the two self-driving sources that need no switch at all.
    (TrialType::Tech, TrialRef::Challenge("copper-rush")),
    (TrialType::Tech, TrialRef::Challenge("mill-race")),
    (TrialType::Tech, TrialRef::Challenge("catch-the-wind")),
    (TrialType::Make, TrialRef::Challenge("splash-zone")),
    (TrialType::Brave, TrialRef::Challenge("lava-floor")),
    (TrialType::Make, TrialRef::Challenge("obsidian")),
    (TrialType::Brave, TrialRef::Challenge("booby-trap")),
    (TrialType::Brave, TrialRef::Challenge("breach-and-clear")),
    (TrialType::Brave, TrialRef::Challenge("kaboomtown")),
    (TrialType::Brave, TrialRef::Challenge("monster-mash")),
    (TrialType::Race, TrialRef::Race("marathon")),
    (TrialType::Make, TrialRef::Challenge("follow-the-plan")),
    (TrialType::Make, TrialRef::Challenge("fresh-coat")),
    (TrialType::Make, TrialRef::Challenge("bouncer")),
    // Native only: `SendFeedback` fires from `/bug`·`/idea`, which the browser
    // build does not have (no feedback channel on web, 2026-10-01).
    #[cfg(not(target_arch = "wasm32"))]
    (TrialType::Tech, TrialRef::Challenge("suggestion-box")),
    (TrialType::Make, TrialRef::Challenge("workshop-publish")),
    (TrialType::Brave, TrialRef::Challenge("champion")),
];

/// Wrap a Race trial as a launchable `ScenarioDef` for the lobby Trials column.
/// The scenario is just the arena (a fixed-seed, day-locked, mob-free normal
/// world); `trial_race` tells the engine to arm the race — teleport to the
/// start, plant the finish beacon, start the clock + ghost — on entry. A fixed
/// `arena_seed` makes the course identical every play, so personal bests + the
/// ghost stay comparable.
pub fn race_scenario_def(def: &TrialDef) -> crate::scenario::ScenarioDef {
    crate::scenario::ScenarioDef {
        kind: crate::scenario::ScenarioKind::Test,
        display_name: def.name.to_string(),
        kit: Vec::new(),
        objective: crate::scenario::Objective::FreeRoam,
        scoring: crate::scenario::Scoring::None,
        arena_seed: Some(0x5A_CE_5A_CE),
        tuning: crate::scenario::ScenarioTuning::default(),
        lock_creative: false,
        arena_mode: crate::scenario::ArenaMode::Reuse,
        world_type: None,
        game_mode: Some("survival".to_string()),
        time_lock: Some("day".to_string()),
        weather_lock: None,
        mobs_enabled: Some(false),
        trial_race: Some(def.id.to_string()),
        arena: None,
    }
}

/// The result of a finished Race trial — drives the "Well done!" completion
/// panel (celebrate, then usher the player back to Trials or let them retry).
#[derive(Clone, Debug)]
pub struct TrialOutcome {
    pub trial_id: String,
    pub name: String,
    /// The big result line (e.g. the finishing time + "new best!").
    pub headline: String,
    /// True if this run set a new personal best (for the ⭐ flourish).
    pub is_best: bool,
}

/// What the player picked on the completion panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrialDoneAction {
    None,
    /// Return to the lobby Trials column (the trial is marked done).
    BackToTrials,
    /// Re-run the same trial in place (beat your time).
    TryAgain,
}

/// A live trial attempt, held as `Option<ActiveTrial>` on GameState.
#[derive(Clone, Debug)]
pub struct ActiveTrial {
    pub id: String,
    pub name: String,
    /// World-space finish point (Y snapped at launch).
    pub finish: [f32; 3],
    pub finish_radius: f32,
    pub run: RaceRun,
    /// The personal-best ghost to chase — `None` on a first attempt.
    pub chase: Option<GhostRecording>,
    /// Campaign G — an imported friend's ghost racing alongside (orange to the
    /// PB's cyan). `None` when no rival is stored for this race.
    pub rival: Option<RivalGhost>,
}

/// Format a tick count (20 TPS) as a runner-friendly time string.
pub fn format_time(ticks: u32) -> String {
    let total_tenths = ticks * 10 / 20; // 20 ticks/sec → tenths of a second
    let tenths = total_tenths % 10;
    let secs = (total_tenths / 10) % 60;
    let mins = total_tenths / 600;
    if mins > 0 {
        format!("{mins}:{secs:02}.{tenths}")
    } else {
        format!("{secs}.{tenths}s")
    }
}

// ── Persistence — per-device. Native: profile/trials.json. WASM: localStorage.
// Mirrors graphics_settings: best-effort, failure never breaks play.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const TRIALS_STORAGE_KEY: &str = "axenstax_trials";

impl TrialBests {
    /// Load persisted bests (absent/corrupt → empty; never panics).
    pub fn load() -> Self {
        load_raw().map(|j| Self::from_json(&j)).unwrap_or_default()
    }

    /// Persist these bests (best-effort — a write failure just means they don't
    /// survive the session, never a crash).
    pub fn save(&self) {
        save_raw(&self.to_json());
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn trials_path() -> std::path::PathBuf {
    crate::data_dir::profile_dir().join("trials.json")
}
#[cfg(not(target_arch = "wasm32"))]
fn load_raw() -> Option<String> {
    std::fs::read_to_string(trials_path()).ok()
}
#[cfg(not(target_arch = "wasm32"))]
fn save_raw(json: &str) {
    let p = trials_path();
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(p, json);
}
#[cfg(target_arch = "wasm32")]
fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}
#[cfg(target_arch = "wasm32")]
fn load_raw() -> Option<String> {
    local_storage()?.get_item(TRIALS_STORAGE_KEY).ok()?
}
#[cfg(target_arch = "wasm32")]
fn save_raw(json: &str) {
    if let Some(store) = local_storage() {
        let _ = store.set_item(TRIALS_STORAGE_KEY, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(x: f32) -> GhostFrame {
        GhostFrame { pos: [x, 64.0, 0.0], yaw: 0.0, pitch: 0.0 }
    }

    #[test]
    fn ghost_share_file_round_trips() {
        // Campaign G (2026-07-05) — the `.axeghost` file a kid hands a friend.
        let mut ghost = GhostRecording::new();
        ghost.record(frame(0.0));
        ghost.record(frame(1.0));
        let f = GhostShareFile {
            version: GHOST_FILE_VERSION,
            trial_id: "sprint".to_string(),
            trial_name: "The Sprint".to_string(),
            ticks: 240,
            author: Some("Axo".to_string()),
            ghost,
        };
        let back = GhostShareFile::from_json(&f.to_json()).expect("round trip");
        assert_eq!(back, f);
    }

    #[test]
    fn ghost_share_file_rejects_bad_input_with_readable_errors() {
        assert!(GhostShareFile::from_json("not json").is_err());
        assert!(GhostShareFile::from_json("{}").is_err());
        // Future version → readable refusal, not a panic or silent garbage.
        let mut ghost = GhostRecording::new();
        ghost.record(frame(0.0));
        let future = GhostShareFile {
            version: GHOST_FILE_VERSION + 1,
            trial_id: "sprint".into(),
            trial_name: "The Sprint".into(),
            ticks: 1,
            author: None,
            ghost,
        };
        let err = GhostShareFile::from_json(&future.to_json()).unwrap_err();
        assert!(err.contains("newer"), "version error is readable: {err}");
        // Empty recording → refused (nothing to race).
        let empty = GhostShareFile {
            version: GHOST_FILE_VERSION,
            trial_id: "sprint".into(),
            trial_name: "The Sprint".into(),
            ticks: 1,
            author: None,
            ghost: GhostRecording::new(),
        };
        assert!(GhostShareFile::from_json(&empty.to_json()).is_err());
        // Oversized recording (review fix) — a ghost bigger than the local
        // recording cap is refused so one import can't bloat the trials store
        // (web: quota-broken persistence). Byte guard catches it pre-parse.
        let mut big = GhostRecording::new();
        big.frames = vec![frame(0.0); MAX_GHOST_FRAMES + 1];
        let over = GhostShareFile {
            version: GHOST_FILE_VERSION,
            trial_id: "sprint".into(),
            trial_name: "The Sprint".into(),
            ticks: 1,
            author: None,
            ghost: big,
        };
        let err = GhostShareFile::from_json(&over.to_json()).unwrap_err();
        assert!(err.contains("too big"), "oversized ghost refused: {err}");
    }

    #[test]
    fn rivals_persist_and_old_stores_load_clean() {
        let mut bests = TrialBests::default();
        let mut ghost = GhostRecording::new();
        ghost.record(frame(0.0));
        bests.rivals.insert(
            "sprint".to_string(),
            RivalGhost { label: "Axo".to_string(), ticks: 200, ghost },
        );
        let back = TrialBests::from_json(&bests.to_json());
        assert_eq!(back.rivals.len(), 1);
        assert_eq!(back.rivals["sprint"].label, "Axo");
        // A pre-rival store (no `rivals` key) still loads — tolerant default.
        let old = r#"{"bests":{},"completed_challenges":[]}"#;
        assert!(TrialBests::from_json(old).rivals.is_empty());
    }

    #[test]
    fn rival_outcome_line_celebrates_or_reports() {
        let mut ghost = GhostRecording::new();
        ghost.record(frame(0.0));
        let rival = RivalGhost { label: "Axo".to_string(), ticks: 200, ghost };
        assert_eq!(rival_outcome_line(&rival, 150), "You beat Axo's ghost!");
        let slower = rival_outcome_line(&rival, 300);
        assert!(slower.starts_with("Axo's ghost: "), "reports their time: {slower}");
    }

    #[test]
    fn trial_order_covers_every_trial_once() {
        use std::collections::HashSet;
        // Canonical sets the lobby renders from.
        let races: HashSet<&str> = CATALOG.iter().map(|d| d.id).collect();
        let challenges: HashSet<String> = crate::scenario::challenge_listing()
            .into_iter()
            .map(|(tok, _)| tok.to_string())
            .collect();

        let mut seen_races: HashSet<&str> = HashSet::new();
        let mut seen_ch: HashSet<String> = HashSet::new();
        for (_, tref) in TRIAL_ORDER {
            match tref {
                TrialRef::Race(id) => {
                    assert!(races.contains(id), "TRIAL_ORDER race '{id}' is not in CATALOG");
                    assert!(seen_races.insert(id), "TRIAL_ORDER lists race '{id}' twice");
                }
                TrialRef::Challenge(tok) => {
                    assert!(
                        challenges.contains(*tok),
                        "TRIAL_ORDER challenge '{tok}' is not a known challenge token"
                    );
                    assert!(
                        seen_ch.insert(tok.to_string()),
                        "TRIAL_ORDER lists challenge '{tok}' twice"
                    );
                }
            }
        }
        assert_eq!(seen_races.len(), races.len(), "every Race must appear in TRIAL_ORDER");
        assert_eq!(
            seen_ch.len(),
            challenges.len(),
            "every Challenge must appear in TRIAL_ORDER"
        );
    }

    #[test]
    fn trial_types_have_icon_and_label() {
        for t in [
            TrialType::Race,
            TrialType::Make,
            TrialType::Tech,
            TrialType::Brave,
            TrialType::Tend,
        ] {
            assert!(!t.icon().is_empty());
            assert!(!t.label().is_empty());
        }
    }

    #[test]
    fn ghost_sample_holds_on_last_frame_and_none_when_empty() {
        let mut g = GhostRecording::new();
        assert_eq!(g.sample(0), None, "empty recording samples to None");
        g.record(frame(0.0));
        g.record(frame(1.0));
        assert_eq!(g.sample(0).unwrap().pos[0], 0.0);
        assert_eq!(g.sample(1).unwrap().pos[0], 1.0);
        // Past the end → holds on the final frame (ghost waits at the line).
        assert_eq!(g.sample(99).unwrap().pos[0], 1.0);
    }

    #[test]
    fn every_race_has_a_blurb_and_how_to() {
        // The Trials dropdown shows `blurb`; the "What to do?" pop-up shows
        // `how_to`. Neither may be blank for any bundled race.
        for d in CATALOG {
            assert!(!d.blurb.trim().is_empty(), "{}: needs a blurb", d.id);
            assert!(!d.how_to.trim().is_empty(), "{}: needs a how_to", d.id);
        }
    }

    #[test]
    fn ghost_recording_is_bounded() {
        // An over-long / idle run can't grow the stored ghost without limit
        // (memory + the serialized size written on a new PB).
        let mut g = GhostRecording::new();
        for i in 0..(MAX_GHOST_FRAMES + 500) {
            g.record(frame(i as f32));
        }
        assert_eq!(g.frames.len(), MAX_GHOST_FRAMES, "recording caps at the limit");
        // Replay still works — it holds on the final recorded frame.
        assert!(g.sample(MAX_GHOST_FRAMES + 9_000).is_some());
    }

    #[test]
    fn personal_best_keeps_only_the_fastest() {
        let mut bests = TrialBests::default();
        assert!(bests.submit("race-a", 200, GhostRecording::new()), "first run is a best");
        assert!(!bests.submit("race-a", 250, GhostRecording::new()), "slower is not a best");
        assert_eq!(bests.best("race-a").unwrap().ticks, 200, "best stays the fastest");
        assert!(bests.submit("race-a", 150, GhostRecording::new()), "faster is a new best");
        assert_eq!(bests.best("race-a").unwrap().ticks, 150);
        assert!(bests.best("race-b").is_none(), "untried trial has no best");
    }

    #[test]
    fn bests_round_trip_json_and_tolerate_garbage() {
        let mut bests = TrialBests::default();
        let mut g = GhostRecording::new();
        g.record(frame(3.0));
        bests.submit("race-a", 120, g);
        let restored = TrialBests::from_json(&bests.to_json());
        assert_eq!(restored, bests);
        assert_eq!(TrialBests::from_json("not json"), TrialBests::default());
    }

    #[test]
    fn race_run_records_only_between_start_and_finish() {
        let mut run = RaceRun::default();
        run.tick(frame(0.0)); // ignored — not started
        assert_eq!(run.elapsed_ticks, 0);
        run.start();
        run.tick(frame(0.0));
        run.tick(frame(1.0));
        assert_eq!(run.elapsed_ticks, 2);
        assert_eq!(run.recording.len(), 2);
        assert_eq!(run.finish(), Some(2));
        assert_eq!(run.finish(), None, "can't finish twice");
        run.tick(frame(2.0)); // ignored — finished
        assert_eq!(run.elapsed_ticks, 2);
    }

    #[test]
    fn finish_before_start_is_none() {
        let mut run = RaceRun::default();
        assert_eq!(run.finish(), None);
    }

    #[test]
    fn volume_trigger_is_inclusive() {
        assert!(in_volume([1.0, 64.0, 1.0], [0.0, 63.0, 0.0], [2.0, 66.0, 2.0]));
        assert!(in_volume([0.0, 63.0, 0.0], [0.0, 63.0, 0.0], [2.0, 66.0, 2.0]), "faces count");
        assert!(!in_volume([3.0, 64.0, 1.0], [0.0, 63.0, 0.0], [2.0, 66.0, 2.0]));
    }

    #[test]
    fn reached_is_horizontal_only() {
        let finish = [0.0, 64.0, 60.0];
        assert!(reached([1.0, 99.0, 61.0], finish, 2.5), "within radius, any height");
        assert!(!reached([5.0, 64.0, 60.0], finish, 2.5), "too far horizontally");
    }

    #[test]
    fn catalog_ids_are_unique_and_findable() {
        let mut seen = std::collections::HashSet::new();
        for d in CATALOG {
            assert!(seen.insert(d.id), "duplicate trial id {}", d.id);
            assert!(find_def(d.id).is_some());
            assert!(d.finish_radius > 0.0);
        }
        assert!(find_def("nope").is_none());
    }

    #[test]
    fn time_format_reads_well() {
        assert_eq!(format_time(0), "0.0s");
        assert_eq!(format_time(20), "1.0s"); // 20 ticks = 1.0s
        assert_eq!(format_time(25), "1.2s"); // 1.25s → 1.2s
        assert_eq!(format_time(1200), "1:00.0"); // 60s
        assert_eq!(format_time(1230), "1:01.5"); // 61.5s
    }

    // ── Profile merge (`.axeprofile` import — "Take your worlds to native") ──

    fn best(ticks: u32, x: f32) -> TrialBest {
        let mut g = GhostRecording::new();
        g.record(frame(x));
        TrialBest { ticks, ghost: g }
    }

    #[test]
    fn merge_keeps_the_faster_time_and_its_ghost() {
        let mut mine = TrialBests::default();
        mine.bests.insert("sprint".into(), best(300, 1.0));
        mine.bests.insert("climb".into(), best(500, 2.0));

        let mut web = TrialBests::default();
        web.bests.insert("sprint".into(), best(250, 9.0)); // faster — wins
        web.bests.insert("climb".into(), best(900, 8.0)); // slower — ignored
        web.bests.insert("swim".into(), best(400, 7.0)); // new — taken

        mine.merge(&web);

        assert_eq!(mine.bests["sprint"].ticks, 250, "faster web time must win");
        assert_eq!(
            mine.bests["sprint"].ghost.frames[0].pos[0], 9.0,
            "the winning time must bring its own ghost, not keep the old one"
        );
        assert_eq!(mine.bests["climb"].ticks, 500, "a slower web time must not overwrite");
        assert_eq!(mine.bests["swim"].ticks, 400, "a trial only raced on the web must arrive");
    }

    #[test]
    fn merge_unions_completed_challenges_and_is_idempotent() {
        let mut mine = TrialBests::default();
        mine.mark_challenge_done("Chop a tree");
        let mut web = TrialBests::default();
        web.mark_challenge_done("Chop a tree");
        web.mark_challenge_done("Light a fire");

        mine.merge(&web);
        assert!(mine.is_challenge_done("Chop a tree"));
        assert!(mine.is_challenge_done("Light a fire"));
        assert_eq!(mine.completed_challenges.len(), 2);

        // Re-importing the same bundle must change nothing.
        let before = mine.clone();
        mine.merge(&web);
        assert_eq!(mine, before, "merge must be idempotent");
    }

    #[test]
    fn merge_takes_the_faster_rival_ghost() {
        let rival = |label: &str, ticks: u32| RivalGhost {
            label: label.to_string(),
            ticks,
            ghost: GhostRecording::new(),
        };
        let mut mine = TrialBests::default();
        mine.rivals.insert("sprint".into(), rival("Axo", 400));
        let mut web = TrialBests::default();
        web.rivals.insert("sprint".into(), rival("Stax", 350));
        web.rivals.insert("climb".into(), rival("Stax", 600));

        mine.merge(&web);
        assert_eq!(mine.rivals["sprint"].label, "Stax", "the faster rival is the one to chase");
        assert_eq!(mine.rivals["climb"].ticks, 600, "a race with no local rival takes the web one");
    }

    #[test]
    fn merging_an_empty_store_changes_nothing() {
        let mut mine = TrialBests::default();
        mine.bests.insert("sprint".into(), best(300, 1.0));
        mine.mark_challenge_done("Chop a tree");
        let before = mine.clone();
        mine.merge(&TrialBests::default());
        assert_eq!(mine, before);
    }
}
