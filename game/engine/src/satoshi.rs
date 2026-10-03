//! Satoshi — the optional, fully-scripted in-world onboarding guide.
//!
//! Design: `docs/superpowers/specs/2026-06-22-guided-onboarding-satoshi-design.md`
//! v2 design: `docs/superpowers/specs/2026-06-23-satoshi-onboarding-v2-design.md`
//!
//! Satoshi is a `MobType::Villager` + a [`SatoshiMarker`] tag (NOT a new
//! `MobType`), so he rides every existing villager code path for free. He lives
//! in a small **wooden** hut near the world spawn and gives the player a
//! **starter hut schematic** (a `Plan`) to build — Survival greets with food
//! first, Creative goes straight to the plan. He only appears in worlds created
//! to host him ([`SatoshiState::enabled`], set at create for new normal
//! Survival/Creative worlds), never in an existing world.
//!
//! COMPLIANCE (load-bearing, enforced by `corpus_has_no_money_or_earning_words`):
//! every authored line is kid-safe, UK English, and NEVER mentions sats,
//! Bitcoin, earning, payouts, wallets, money, or rewards-as-money. His currency
//! is *fun and care*. There is NO live AI, no network, no free-text input.

use crate::block;
use crate::chunk::CHUNK_SIZE;
use crate::plan::{CapturedCell, PlanData};
use crate::world::World;
use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Marks the single villager who is Satoshi in a given world. Satoshi is a
/// `MobType::Villager` + this marker — never a new `MobType` — so he stays on
/// every existing villager code path. `Profession::None` (no workstation claim).
pub struct SatoshiMarker;

/// Per-world Satoshi progress + placement. Persisted as the NEWEST appended
/// field on [`crate::save::WorldSave`] (after `saved_mobs`) and mirrored on
/// [`World`]. Brand-new struct → its internal field order is free *now*; any
/// FUTURE field must be appended (bincode is positional).
#[derive(Default, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SatoshiState {
    /// House origin `[x, y, z]` once placed. `None` until the spawn-area ground
    /// is loaded. House *blocks* persist in the chunks; the villager *entity* is
    /// re-derived on load (not saved) — re-spawned here if no marker exists.
    pub home: Option<[i32; 3]>,
    /// He has greeted the player at least once.
    pub has_greeted: bool,
    /// The welcome **food** gift was handed over (Survival care touch).
    pub has_gifted: bool,
    /// The starter **schematic** was handed over (the centrepiece gift).
    pub schematic_gifted: bool,
    /// The player waved him off — pull-only thereafter (never initiates again).
    pub waved_off: bool,
    /// Last tick he initiated contact (frequency-budget guard).
    pub last_initiated_tick: u64,
    /// Whether this world should host Satoshi at all. Mirrored from
    /// `WorldMeta.satoshi_enabled` on load — `true` only for new normal
    /// Survival/Creative worlds. Existing/flat/gallery/workshop worlds and
    /// Adventure/Spectator never set it, so he never appears there.
    pub enabled: bool,
}

/// Minimum ticks between any Satoshi-initiated cue (~1 in-game day) — anti-Navi.
/// MVP keeps initiation off in practice (see `should_initiate` below) —
/// tested directly.
#[cfg_attr(not(test), allow(dead_code))]
pub const SATOSHI_INITIATE_COOLDOWN_TICKS: u64 = 24_000;

// ── House placement (a short, deterministic walk from the world spawn) ──
const HOUSE_OX: i32 = 4; // house origin, a few blocks off the (0,0) spawn
const HOUSE_OZ: i32 = 3;
const HUT_W: u8 = 5; // square footprint (x and z)
const HUT_H: u8 = 4; // floor → roof: ry 0..=3 (floor, 2 walls, roof)

/// Centre column of the house footprint (for the surface probe + spawn point).
const fn centre() -> (i32, i32) {
    (HOUSE_OX + HUT_W as i32 / 2, HOUSE_OZ + HUT_W as i32 / 2)
}

/// Satoshi's stand position: centre of the house, one block above the floor.
fn satoshi_pos(floor_y: i32) -> Vec3 {
    let (cx, cz) = centre();
    Vec3::new(cx as f32 + 0.5, floor_y as f32 + 1.0, cz as f32 + 0.5)
}

/// The structural cells of the humble **wooden** hut, relative to the lowest-XZ
/// corner at `ry = 0` (floor). Oak-plank floor + roof, oak-log corner posts +
/// plank walls, a 2-high doorway on the front (`rz = 0`) centre column.
/// SHARED by [`build_house`] (Satoshi's home) and [`starter_hut_plan`] (the
/// gifted schematic), so "build one like mine" is literally true. "Start with
/// wood." Non-air cells only (air = omitted, e.g. the doorway).
pub fn hut_cells() -> Vec<CapturedCell> {
    let w = HUT_W;
    let h = HUT_H;
    let dc = w / 2; // door centre column
    let mut cells = Vec::new();
    for rz in 0..w {
        for rx in 0..w {
            cells.push(CapturedCell { rx, ry: 0, rz, block_id: block::OAK_PLANKS }); // floor
            cells.push(CapturedCell { rx, ry: h - 1, rz, block_id: block::OAK_PLANKS }); // roof
        }
    }
    // Walls (ry 1..=h-2), perimeter only; log corners + plank infill; door gap.
    for ry in 1..(h - 1) {
        for rz in 0..w {
            for rx in 0..w {
                let perim = rx == 0 || rx == w - 1 || rz == 0 || rz == w - 1;
                if !perim {
                    continue;
                }
                if rz == 0 && rx == dc {
                    continue; // doorway (front wall, centre, full height)
                }
                let corner = (rx == 0 || rx == w - 1) && (rz == 0 || rz == w - 1);
                let blk = if corner { block::OAK_LOG } else { block::OAK_PLANKS };
                cells.push(CapturedCell { rx, ry, rz, block_id: blk });
            }
        }
    }
    cells
}

/// The gifted starter schematic — the same wooden hut Satoshi lives in.
pub fn starter_hut_plan() -> PlanData {
    PlanData::from_imported(
        "Satoshi's Starter Hut".to_string(),
        HUT_W,
        HUT_W,
        HUT_H,
        hut_cells(),
    )
}

/// Build Satoshi's wooden hut at floor `oy`: clear the volume, place the shared
/// [`hut_cells`], add glass windows on the three non-door walls + a roof torch
/// beacon. Idempotent in effect (writes the same blocks for a given `oy`).
fn build_house(world: &mut World, oy: i32) {
    let (ox, oz) = (HOUSE_OX, HOUSE_OZ);
    let w = HUT_W as i32;
    let h = HUT_H as i32;
    // Clear the interior + walls volume so terrain doesn't intrude.
    for dy in 0..=h {
        for dz in 0..w {
            for dx in 0..w {
                world.set_block(ox + dx, oy + dy, oz + dz, block::AIR);
            }
        }
    }
    // Place the structural hut (identical to the gifted plan).
    for c in hut_cells() {
        world.set_block(ox + c.rx as i32, oy + c.ry as i32, oz + c.rz as i32, c.block_id);
    }
    // Glass windows on the three non-door walls (mid-height) + a roof torch.
    let dc = HUT_W as i32 / 2;
    world.set_block(ox + dc, oy + 2, oz + w - 1, block::GLASS); // back wall
    world.set_block(ox, oy + 2, oz + dc, block::GLASS); // left wall
    world.set_block(ox + w - 1, oy + 2, oz + dc, block::GLASS); // right wall
    world.set_block(ox + dc, oy + h, oz + dc, block::TORCH); // beacon on the roof
    // Interior hearth-glow — a warm corner light so the hut reads "someone wise
    // lives here" from within (atmosphere; entities don't sample block-light, so
    // this lights the room, not Satoshi himself — his glow is his amulet).
    world.set_block(ox + 1, oy + 1, oz + 1, block::TORCH);
}

/// Spawn the Satoshi villager entity + tag it. `VillagerComponent`
/// (`Profession::None`) is auto-attached by `entity::spawn_mob`.
pub fn spawn_satoshi(ecs: &mut hecs::World, pos: Vec3) -> hecs::Entity {
    let id = crate::entity::spawn_mob(ecs, crate::mob::MobType::Villager, pos);
    let _ = ecs.insert_one(id, SatoshiMarker);
    id
}

/// True if a Satoshi entity currently exists in the ECS.
fn satoshi_exists(ecs: &hecs::World) -> bool {
    ecs.query::<&SatoshiMarker>().iter().next().is_some()
}

/// Session-only tag (never persisted — the entity is re-spawned on load):
/// Satoshi has been summoned and is walking to the player. Without it he heads
/// back to his hut. Toggled by the summon key (H).
pub struct Summoned;

/// Walk speed toward the summon/dismiss target (blocks/tick; ≈2.4 blocks/sec).
const SUMMON_SPEED: f32 = 0.12;
/// How close he stops to the player when summoned (so he doesn't crowd them).
const SUMMON_STOP: f32 = 1.8;

/// Toggle Satoshi's summoned state. `Some(true)` = he's now coming to you,
/// `Some(false)` = he's heading home, `None` = there's no Satoshi in this world.
pub fn toggle_summon(ecs: &mut hecs::World) -> Option<bool> {
    let id = ecs.query::<&SatoshiMarker>().iter().next().map(|(id, _)| id)?;
    if ecs.get::<&Summoned>(id).is_ok() {
        let _ = ecs.remove_one::<Summoned>(id);
        Some(false)
    } else {
        let _ = ecs.insert_one(id, Summoned);
        Some(true)
    }
}

/// Move Satoshi a step toward the player (summoned) or back to his hut
/// (dismissed) this tick, snapping to the ground. Direct stepping rather than
/// mob physics, so he reads as walking without falling/sticking — call AFTER the
/// mob-AI + entity-physics passes so it's the last word on his position. He also
/// turns to face the target. No-op when Satoshi isn't enabled or spawned.
pub fn tick_summon(world: &World, ecs: &mut hecs::World, player_pos: Vec3) {
    if !world.satoshi.enabled {
        return;
    }
    let Some(id) = ecs.query::<&SatoshiMarker>().iter().next().map(|(id, _)| id) else {
        return;
    };
    let summoned = ecs.get::<&Summoned>(id).is_ok();
    let (tx, tz) = if summoned {
        (player_pos.x, player_pos.z)
    } else {
        // Dismissed → walk back to his STAND spot (the house centre), NOT the
        // origin corner stored in `home`. The corner is a roofed log post, so
        // targeting it (and snapping to its highest block) would put him on the
        // roof. The centre is where he spawns.
        match world.satoshi.home {
            Some(_) => {
                let (cx, cz) = centre();
                (cx as f32 + 0.5, cz as f32 + 0.5)
            }
            None => return,
        }
    };
    let cur = match ecs.get::<&crate::entity::Position>(id) {
        Ok(p) => p.0,
        Err(_) => return,
    };
    let (dx, dz) = (tx - cur.x, tz - cur.z);
    let dist = (dx * dx + dz * dz).sqrt();
    // Always face the target (so he looks at you even once he's arrived).
    if dist > 0.01
        && let Ok(mut ai) = ecs.get::<&mut crate::mob_ai::MobAi>(id) {
            ai.facing = dz.atan2(dx);
        }
    let stop = if summoned { SUMMON_STOP } else { 0.4 };
    if dist <= stop {
        return;
    }
    let step = SUMMON_SPEED.min(dist - stop);
    let nx = cur.x + dx / dist * step;
    let nz = cur.z + dz / dist * step;
    let (ncx, ncz) = (nx.floor() as i32, nz.floor() as i32);
    // Ground-snap. His hut roof spans the whole footprint, so a naive
    // highest-block probe inside the hut would stand him ON the roof — there,
    // use the known floor height; elsewhere follow the terrain surface.
    let ny = match world.satoshi.home {
        Some([hx, fy, hz])
            if (hx..hx + HUT_W as i32).contains(&ncx)
                && (hz..hz + HUT_W as i32).contains(&ncz) =>
        {
            fy as f32 + 1.0
        }
        _ => world
            .highest_block(ncx, ncz)
            .map(|(y, _)| y as f32 + 1.0)
            .unwrap_or(cur.y),
    };
    if let Ok(mut p) = ecs.get::<&mut crate::entity::Position>(id) {
        p.0 = Vec3::new(nx, ny, nz);
    }
}

/// Once-per-cadence spawn pass (call on the village tick). Places Satoshi in his
/// hut on first entry to an enabled world, then re-spawns the (non-persisted)
/// entity at his home on reload. Idempotent. Only acts in worlds explicitly
/// created to host him — never an existing world, never the Workshop.
pub fn tick_spawn(world: &mut World, ecs: &mut hecs::World) {
    if !world.satoshi.enabled || world.is_workshop {
        return;
    }
    match world.satoshi.home {
        None => {
            let (cx, cz) = centre();
            let Some((surf_y, _)) = world.highest_block(cx, cz) else {
                return; // ground not loaded yet — try again next cadence
            };
            let floor_y = surf_y + 1;
            build_house(world, floor_y);
            spawn_satoshi(ecs, satoshi_pos(floor_y));
            world.satoshi.home = Some([HOUSE_OX, floor_y, HOUSE_OZ]);
        }
        Some([_, floor_y, _]) => {
            if satoshi_exists(ecs) {
                return;
            }
            let (cx, cz) = centre();
            let cs = CHUNK_SIZE as i32;
            if !world.has_chunk(cx.div_euclid(cs), floor_y.div_euclid(cs), cz.div_euclid(cs)) {
                return;
            }
            spawn_satoshi(ecs, satoshi_pos(floor_y));
        }
    }
}

// ── Authored dialogue corpus (100% scripted; scanned by the compliance test) ──
//
// COMPLIANCE: UK English, kid-safe, care-first. NEVER mentions sats, Bitcoin,
// earning, payouts, wallets, money, or rewards-as-money.

/// His name. (Contains "sat", but the compliance test matches whole words.)
pub const SATOSHI_NAME: &str = "Satoshi";

/// Warm sage greeting — care first (leads to the food gift). Obi-Wan-calm:
/// wise and gentle, never stuffy. Short (the onboarding minimal-reading rule).
pub const SATOSHI_GREETING: &str =
    "Ah — there you are. I had a feeling the wind would bring someone today. \
     You look weary from the road; let me see to that.";

/// Creative greeting — a fresh canvas (leads straight to the schematic).
pub const SATOSHI_CREATIVE_GREETING: &str =
    "Ah — a fresh world, and a maker's hands. Shall we begin?";

/// Said as he hands over the welcome food (Survival) — framed as a kindness.
pub const SATOSHI_GIFT_LINE: &str =
    "Take this. A small kindness for the road ahead.";

/// Said as he hands over the starter schematic (both modes).
pub const SATOSHI_SCHEMATIC_LINE: &str =
    "And this — a plan, old as these parts. Lay it down and build along; \
     in Survival you'll gather what it asks for as you go.";

/// Warm line when the player returns after the gifts (pull-on-demand).
pub const SATOSHI_RETURN_LINE: &str =
    "Back again? Good. My door is always open to those who seek it.";

/// Label on the food-accept action.
pub const SATOSHI_GIFT_LABEL: &str = "Thank you!";
/// Label on the schematic-accept action.
pub const SATOSHI_SCHEMATIC_LABEL: &str = "Take the plan";
/// Label on the always-present dismissal action.
pub const SATOSHI_DISMISS_LABEL: &str = "Thanks — I'll explore";

/// Test Lab — shown when the player has been through every mission Satoshi has
/// for this build. Warm + open-ended (he restocks each build).
pub const SATOSHI_MISSIONS_DONE: &str =
    "That is all I have for you for now — and you've done well. \
     Come and find me again when you're ready; there is always more to learn.";

/// The welcome food gift (a kindness, never a payout): a little bread.
pub fn welcome_food() -> crate::item::ItemStack {
    crate::item::ItemStack::new_material(crate::item::MaterialId::Bread, 2)
}

/// Hand over the welcome food exactly once (Survival). Returns `true` if gifted
/// this call. Marks greeted + gifted.
pub fn give_welcome_gift(state: &mut SatoshiState, inv: &mut crate::inventory::Inventory) -> bool {
    state.has_greeted = true;
    if state.has_gifted {
        return false;
    }
    // Only burn the one-shot flag if the food actually landed. A full inventory
    // returns the leftover; setting `has_gifted` then would silently eat the
    // gift forever. Leave it unset so the player can take it after making room.
    if inv.add_item(welcome_food()).is_some() {
        return false;
    }
    state.has_gifted = true;
    true
}

/// Hand over the starter-hut schematic exactly once (both modes). Returns `true`
/// if gifted this call. Marks greeted + schematic_gifted.
pub fn give_schematic(state: &mut SatoshiState, inv: &mut crate::inventory::Inventory) -> bool {
    state.has_greeted = true;
    if state.schematic_gifted {
        return false;
    }
    // The plan is a UNIQUE item with no other source — never burn the one-shot
    // flag unless it actually landed. A full inventory hands the stack back; set
    // the flag only when nothing comes back, so the gift can be re-offered.
    let leftover = inv.add_item(crate::item::ItemStack {
        item: crate::item::Item::Plan(starter_hut_plan()),
        count: 1,
    });
    if leftover.is_some() {
        return false;
    }
    state.schematic_gifted = true;
    true
}

/// What the player picked in the Satoshi dialogue this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SatoshiAction {
    None,
    /// Accept the welcome food (Survival).
    TakeGift,
    /// Accept the starter schematic.
    TakeSchematic,
    /// "I'll explore" — dismiss (pull-only thereafter).
    Dismiss,
}

/// What the player picked in a Test Lab mission dialogue this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissionAction {
    None,
    /// "It worked" → submit a good verdict for the current mission + advance.
    Worked,
    /// "It's broken" → submit a broken verdict (carrying the note) + advance.
    Broken,
    /// "Skip this one" → advance to the next mission without a verdict.
    Skip,
    /// Close the dialogue (come back to Satoshi later).
    Close,
}

/// Which Satoshi panel to show, derived from state + mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SatoshiView {
    /// Survival first meeting: greeting + the food gift.
    Greeting,
    /// Offer the starter schematic (after food in Survival; straight away in Creative).
    SchematicOffer,
    /// Both gifts done: a warm return line, nothing pending.
    Idle,
}

/// Decide which panel to show. Survival greets with food first, then the
/// schematic; Creative goes straight to the schematic. Pure.
pub fn view_for(s: &SatoshiState, is_creative: bool) -> SatoshiView {
    if s.schematic_gifted {
        SatoshiView::Idle
    } else if !is_creative && !s.has_gifted {
        SatoshiView::Greeting
    } else {
        SatoshiView::SchematicOffer
    }
}

/// Whether Satoshi may *initiate* a gentle approach this tick — the anti-Navi
/// frequency budget. Pull-first: only while never-greeted and never-waved-off,
/// and never inside the cooldown. MVP keeps initiation off in practice; this is
/// the guard for any future cue.
#[cfg_attr(not(test), allow(dead_code))]
pub fn should_initiate(s: &SatoshiState, now_tick: u64) -> bool {
    if s.has_greeted || s.waved_off {
        return false;
    }
    s.last_initiated_tick == 0
        || now_tick.saturating_sub(s.last_initiated_tick) >= SATOSHI_INITIATE_COOLDOWN_TICKS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A world configured to host Satoshi (as a new normal world would be).
    fn flat_ground(world: &mut World, y: i32) {
        world.satoshi.enabled = true;
        for z in (HOUSE_OZ - 2)..(HOUSE_OZ + HUT_W as i32 + 2) {
            for x in (HOUSE_OX - 2)..(HOUSE_OX + HUT_W as i32 + 2) {
                world.set_block(x, y, z, block::GRASS);
            }
        }
    }

    /// Every authored, player-facing string — the compliance test scans these.
    fn authored_strings() -> Vec<&'static str> {
        vec![
            SATOSHI_NAME,
            SATOSHI_GREETING,
            SATOSHI_CREATIVE_GREETING,
            SATOSHI_GIFT_LINE,
            SATOSHI_SCHEMATIC_LINE,
            SATOSHI_RETURN_LINE,
            SATOSHI_GIFT_LABEL,
            SATOSHI_SCHEMATIC_LABEL,
            SATOSHI_DISMISS_LABEL,
            SATOSHI_MISSIONS_DONE,
        ]
    }

    #[test]
    fn toggle_summon_flips_and_reports_state() {
        let mut ecs = hecs::World::new();
        assert_eq!(toggle_summon(&mut ecs), None, "no Satoshi to summon → None");
        let id = spawn_satoshi(&mut ecs, Vec3::new(5.0, 64.0, 5.0));
        assert_eq!(toggle_summon(&mut ecs), Some(true), "first press summons");
        assert!(ecs.get::<&Summoned>(id).is_ok());
        assert_eq!(toggle_summon(&mut ecs), Some(false), "second press dismisses");
        assert!(ecs.get::<&Summoned>(id).is_err());
    }

    #[test]
    fn tick_summon_walks_toward_player_then_stops() {
        let mut world = World::new();
        flat_ground(&mut world, 64);
        world.satoshi.home = Some([HOUSE_OX, 65, HOUSE_OZ]);
        let mut ecs = hecs::World::new();
        let start = Vec3::new(HOUSE_OX as f32 + 0.5, 65.0, HOUSE_OZ as f32 + 0.5);
        let id = spawn_satoshi(&mut ecs, start);
        let _ = ecs.insert_one(id, Summoned);
        let player = Vec3::new(HOUSE_OX as f32 + 6.0, 65.0, HOUSE_OZ as f32 + 0.5);
        let before = ecs.get::<&crate::entity::Position>(id).unwrap().0.x;
        tick_summon(&world, &mut ecs, player);
        let after = ecs.get::<&crate::entity::Position>(id).unwrap().0.x;
        assert!(after > before, "a summoned Satoshi steps toward the player");
        // Many ticks → he settles ~SUMMON_STOP away, not on top of the player.
        for _ in 0..200 {
            tick_summon(&world, &mut ecs, player);
        }
        let p = ecs.get::<&crate::entity::Position>(id).unwrap().0;
        let d = ((p.x - player.x).powi(2) + (p.z - player.z).powi(2)).sqrt();
        assert!((d - SUMMON_STOP).abs() < 0.25, "stops ~{SUMMON_STOP} from player, got {d}");
    }

    #[test]
    fn dismissed_satoshi_returns_to_his_floor_not_his_roof() {
        // Regression: dismiss used to target the origin CORNER (a roofed log
        // post) and ground-snap via highest_block, which stood him ON the roof.
        // He must walk back to his centre stand spot, on his floor.
        let mut world = World::new();
        flat_ground(&mut world, 64);
        let mut ecs = hecs::World::new();
        tick_spawn(&mut world, &mut ecs); // builds the hut (roof + walls) + spawns him
        let id = ecs
            .query::<&SatoshiMarker>()
            .iter()
            .next()
            .map(|(id, _)| id)
            .unwrap();
        let floor_y = world.satoshi.home.unwrap()[1];
        // Summon him well outside the hut, then dismiss and let him walk home.
        let player = Vec3::new(HOUSE_OX as f32 + 14.0, floor_y as f32, HOUSE_OZ as f32 + 0.5);
        let _ = ecs.insert_one(id, Summoned);
        for _ in 0..600 {
            tick_summon(&world, &mut ecs, player);
        }
        let _ = ecs.remove_one::<Summoned>(id);
        for _ in 0..600 {
            tick_summon(&world, &mut ecs, player);
        }
        let p = ecs.get::<&crate::entity::Position>(id).unwrap().0;
        assert!(
            (p.y - (floor_y as f32 + 1.0)).abs() < 0.01,
            "stands on his floor (y={}), not the roof — got y={}",
            floor_y + 1,
            p.y
        );
        let (cx, cz) = centre();
        let d = ((p.x - (cx as f32 + 0.5)).powi(2) + (p.z - (cz as f32 + 0.5)).powi(2)).sqrt();
        assert!(d < 0.5, "returns to his centre stand spot, got dist {d}");
    }

    #[test]
    fn gifts_are_not_lost_when_the_inventory_is_full() {
        // Regression: the one-shot gift flags were set even when add_item handed
        // the item back (full inventory), silently eating the gift — worst case
        // the UNIQUE starter plan. Flags must stay unset until the gift lands.
        let mut state = SatoshiState::default();
        // Fill every slot (add_item returns the leftover once it's full).
        let mut full = crate::inventory::Inventory::new();
        while full
            .add_item(crate::item::ItemStack::new_block(block::STONE, 64))
            .is_none()
        {}
        assert!(!give_welcome_gift(&mut state, &mut full), "full inv → not gifted");
        assert!(!state.has_gifted, "flag stays unset so it can be re-offered");
        assert!(!give_schematic(&mut state, &mut full), "full inv → plan not gifted");
        assert!(!state.schematic_gifted, "plan flag stays unset — it's unique");
        // A roomy inventory → the gifts land and the flags are set.
        let mut roomy = crate::inventory::Inventory::new();
        assert!(give_welcome_gift(&mut state, &mut roomy), "room → gifted");
        assert!(state.has_gifted);
        assert!(give_schematic(&mut state, &mut roomy), "room → plan gifted");
        assert!(state.schematic_gifted);
    }

    /// LOAD-BEARING compliance gate: Satoshi's currency is fun and care — never
    /// money or earning. Whole-word match so "Satoshi" passes while
    /// "sats"/"earn"/… are caught.
    #[test]
    fn corpus_has_no_money_or_earning_words() {
        let banned: &[&str] = &[
            "sat", "sats", "bitcoin", "bitcoins", "earn", "earns", "earned",
            "earning", "earnings", "payout", "payouts", "wallet", "wallets",
            "money", "reward", "rewards", "rewarded",
        ];
        for s in authored_strings() {
            for word in s.split(|c: char| !c.is_alphanumeric()) {
                if word.is_empty() {
                    continue;
                }
                let w = word.to_ascii_lowercase();
                assert!(
                    !banned.contains(&w.as_str()),
                    "Satoshi corpus must never use a money/earning word — found {w:?} in {s:?}",
                );
            }
        }
    }

    #[test]
    fn spawn_pass_places_exactly_one_satoshi_in_a_house() {
        let mut world = World::new();
        let mut ecs = hecs::World::new();
        flat_ground(&mut world, 64);

        tick_spawn(&mut world, &mut ecs);

        assert_eq!(
            ecs.query::<&SatoshiMarker>().iter().count(),
            1,
            "exactly one Satoshi after the first spawn pass"
        );
        assert_eq!(world.satoshi.home, Some([HOUSE_OX, 65, HOUSE_OZ]));
        let (cx, cz) = centre();
        assert_eq!(world.get_block(cx, 65, cz), block::OAK_PLANKS, "house floor laid");
        assert_eq!(
            world.get_block(HOUSE_OX + 1, 66, HOUSE_OZ + 1),
            block::TORCH,
            "the hut is lit from within (interior hearth torch)"
        );

        tick_spawn(&mut world, &mut ecs);
        assert_eq!(
            ecs.query::<&SatoshiMarker>().iter().count(),
            1,
            "spawn pass is idempotent — still exactly one Satoshi"
        );
    }

    #[test]
    fn respawns_at_home_after_a_reload_drops_the_entity() {
        let mut world = World::new();
        let mut ecs = hecs::World::new();
        flat_ground(&mut world, 64);
        tick_spawn(&mut world, &mut ecs);
        ecs = hecs::World::new(); // simulate a reload: entity gone, `home` kept
        assert!(!satoshi_exists(&ecs));

        tick_spawn(&mut world, &mut ecs);
        assert!(satoshi_exists(&ecs), "Satoshi re-spawns at his home on reload");
    }

    #[test]
    fn disabled_world_never_spawns_satoshi() {
        // An existing/flat/etc. world (enabled=false) must never get Satoshi.
        let mut world = World::new();
        let mut ecs = hecs::World::new();
        for z in 0..12 {
            for x in 0..12 {
                world.set_block(x, 64, z, block::GRASS);
            }
        }
        // NOTE: enabled deliberately left false.
        tick_spawn(&mut world, &mut ecs);
        assert!(!satoshi_exists(&ecs), "no Satoshi in a world not created to host him");
        assert_eq!(world.satoshi.home, None);
    }

    #[test]
    fn skips_the_workshop_world() {
        let mut world = World::new();
        world.is_workshop = true;
        let mut ecs = hecs::World::new();
        flat_ground(&mut world, 64); // sets enabled=true
        tick_spawn(&mut world, &mut ecs);
        assert!(!satoshi_exists(&ecs), "no Satoshi in the Workshop editor");
        assert_eq!(world.satoshi.home, None);
    }

    #[test]
    fn welcome_gift_lands_exactly_once() {
        use crate::item::MaterialId;
        let mut state = SatoshiState::default();
        let mut inv = crate::inventory::Inventory::new();
        assert!(give_welcome_gift(&mut state, &mut inv));
        let after = inv.count_material(MaterialId::Bread);
        assert!(after > 0 && state.has_greeted && state.has_gifted);
        assert!(!give_welcome_gift(&mut state, &mut inv), "no double food gift");
        assert_eq!(inv.count_material(MaterialId::Bread), after);
    }

    #[test]
    fn schematic_gift_lands_exactly_once() {
        let mut state = SatoshiState::default();
        let mut inv = crate::inventory::Inventory::new();
        assert!(give_schematic(&mut state, &mut inv));
        assert!(state.schematic_gifted);
        let plans = inv
            .slots_iter()
            .flatten()
            .filter(|s| matches!(s.item, crate::item::Item::Plan(_)))
            .count();
        assert_eq!(plans, 1, "exactly one starter-hut plan gifted");
        assert!(!give_schematic(&mut state, &mut inv), "no double schematic gift");
    }

    #[test]
    fn starter_plan_matches_the_hut_and_is_buildable() {
        let plan = starter_hut_plan();
        assert_eq!(plan.width, HUT_W);
        assert_eq!(plan.height, HUT_H);
        assert_eq!(plan.cells, hut_cells(), "gifted plan == the hut he lives in");
        assert!(!plan.cells.is_empty());
    }

    #[test]
    fn view_flows_survival_food_then_schematic_creative_straight() {
        let mut s = SatoshiState::default();
        // Survival: food first, then schematic, then idle.
        assert_eq!(view_for(&s, false), SatoshiView::Greeting);
        s.has_gifted = true;
        assert_eq!(view_for(&s, false), SatoshiView::SchematicOffer);
        s.schematic_gifted = true;
        assert_eq!(view_for(&s, false), SatoshiView::Idle);
        // Creative: straight to the schematic (no food), then idle.
        let mut c = SatoshiState::default();
        assert_eq!(view_for(&c, true), SatoshiView::SchematicOffer);
        c.schematic_gifted = true;
        assert_eq!(view_for(&c, true), SatoshiView::Idle);
    }

    #[test]
    fn frequency_budget_goes_pull_only_after_wave_off() {
        let s = SatoshiState::default();
        assert!(should_initiate(&s, 0));
        let waved = SatoshiState { waved_off: true, ..Default::default() };
        assert!(!should_initiate(&waved, 1_000_000));
        let greeted = SatoshiState { has_greeted: true, ..Default::default() };
        assert!(!should_initiate(&greeted, 1_000_000));
        let cooled = SatoshiState { last_initiated_tick: 1_000, ..Default::default() };
        assert!(!should_initiate(&cooled, 1_000 + SATOSHI_INITIATE_COOLDOWN_TICKS - 1));
        assert!(should_initiate(&cooled, 1_000 + SATOSHI_INITIATE_COOLDOWN_TICKS));
    }
}
