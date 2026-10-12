//! C3d run 18 — Decision 20, the parts shared by the client and the server:
//! the toggle footprint (one rule for the Mirror click, the Strict client's
//! prediction and the Strict server arm), the door-place cells, and the
//! Strict client's typed `DoorPlace` / `ToggleBlock` routes, and (run 19)
//! the Mirror joiner's `ToggleBlock`.
use crate::{
    armour::ArmourItem,
    block::{BlockId, OAK_DOOR},
    craft_ui::CraftingUi,
    inventory::Inventory,
    remote_client::RemoteClient,
    world::World,
};

/// How a toggle sets the open bit across its footprint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenRule {
    /// Every cell's own open bit flipped (the as-built click, kept
    /// byte-for-byte on Mirror, the host and single-player).
    FlipEach,
    /// Decision 20's canonical bit, derived once: every cell set to the
    /// primary's flipped open bit (a Strict seat, client and server alike).
    Canonical,
}

/// The cell above or below a door half `cell` whose meta is `m`: where its
/// matching half stands.
fn other_half(cell: [i32; 3], m: u8) -> [i32; 3] {
    let dy = if crate::block_shape::door_is_top(m) { -1 } else { 1 };
    [cell[0], cell[1] + dy, cell[2]]
}

/// The complete bounded footprint of a toggle click on the toggleable
/// `primary` (a door, fence gate or trapdoor), primary first: for a door,
/// its other half, and each horizontally adjacent door at the primary's
/// height with ITS other half (#29, double doors open together). Only
/// `OAK_DOOR` companions count; a missing one is skipped (the shared
/// structural rule). At most 10 cells, none repeated. Moved verbatim from
/// the game loop's toggle arm (C3d run 18).
pub fn toggle_footprint(world: &World, primary: [i32; 3]) -> Vec<[i32; 3]> {
    let mut cells = vec![primary];
    if world.get_block(primary[0], primary[1], primary[2]) != OAK_DOOR { return cells; }
    let door = |c: [i32; 3]| world.get_block(c[0], c[1], c[2]) == OAK_DOOR;
    let pair = other_half(primary, world.meta_at(primary[0], primary[1], primary[2]));
    if door(pair) { cells.push(pair); }
    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        let n = [primary[0] + dx, primary[1], primary[2] + dz];
        if !door(n) { continue; }
        cells.push(n);
        let np = other_half(n, world.meta_at(n[0], n[1], n[2]));
        if door(np) { cells.push(np); }
    }
    cells
}

/// C3d run 19 (run-18 review M1/L2) — the cells a refused or superseded
/// toggle puts back as the server holds them: the server's footprint, and,
/// when the client clicked a door, every cell its own footprint could hold
/// (the primary, the cells above and below it, the four same-height
/// neighbours and the cells above and below each: at most 15), whatever the
/// server's cells hold now — so a companion only the client predicted is
/// put back too. A toggle changes no window: this send-back undoes it whole.
pub fn toggle_sendback_cells(world: &World, primary: [i32; 3], expected: BlockId) -> Vec<[i32; 3]> {
    let mut cells = toggle_footprint(world, primary);
    if expected == OAK_DOOR {
        for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            for dy in [0, -1, 1] {
                let c = [primary[0] + dx, primary[1] + dy, primary[2] + dz];
                if !cells.contains(&c) { cells.push(c); }
            }
        }
    }
    cells
}

/// C3d run 20 — the play modes in which a door, gate or trapdoor click
/// toggles it: exactly the single-player toggle arm's policy (main
/// `bdd4fe87`'s arm has no play-mode gate), because a toggle is a USE, not
/// a world edit — an Adventure player opens doors, as in Minecraft's
/// Adventure mode. The client's preflight ([`toggle_preflight`]) and the
/// server (`HostedServer::toggle_cell_permitted`) both ask this one rule.
/// Spectator is kept as main has it (it toggles too) — an owner question,
/// not changed here. Exhaustive, so a new mode must decide.
pub fn toggle_mode_allows(mode: crate::play_mode::PlayMode) -> bool {
    use crate::play_mode::PlayMode;
    match mode {
        PlayMode::Survival | PlayMode::Creative | PlayMode::Adventure | PlayMode::Spectator => true,
    }
}

/// C3d run 19 (run-18 review M1b) — what a joined client can know before
/// it sends a toggle the server would refuse: the play mode lets it toggle
/// ([`toggle_mode_allows`]; run 20: every mode, as single-player), and the
/// primary's centre is within an edit's reach of its eye with the item it
/// holds (the server's own rule, `block_change_within_reach`). `Err` is the
/// toast; nothing is sent or predicted. (A foreign plot is not checked
/// here: the client can't tell its own signed-in plots from others'; the
/// server's refusal of one is a cheap send-back.)
pub fn toggle_preflight(play_mode: crate::play_mode::PlayMode, eye: glam::Vec3, primary: [i32; 3], held: Option<&crate::item::Item>) -> Result<(), &'static str> {
    if !toggle_mode_allows(play_mode) { return Err("You can't open that in this game mode."); }
    let d = glam::Vec3::new(primary[0] as f32 + 0.5, primary[1] as f32 + 0.5, primary[2] as f32 + 0.5) - eye;
    let (kind, id) = held.map_or(crate::protocol::ItemRef::Empty.to_wire(), |item| crate::inventory::item_to_ref(item).to_wire());
    if !crate::hosted_server::block_change_within_reach(d.length_squared(), kind, id, false) { return Err("That's too far away to open."); }
    Ok(())
}

/// The metas a toggle click on `primary` leaves, cell by cell, under `rule`
/// ([`toggle_footprint`]'s cells, primary first). Applying them is the
/// whole toggle (the caller re-meshes or broadcasts each cell).
pub fn toggle_metas(world: &World, primary: [i32; 3], rule: OpenRule) -> Vec<([i32; 3], u8)> {
    let open = !crate::block_shape::is_open(world.meta_at(primary[0], primary[1], primary[2]));
    toggle_footprint(world, primary)
        .into_iter()
        .map(|c| {
            let m = world.meta_at(c[0], c[1], c[2]);
            let new = match rule {
                OpenRule::FlipEach => crate::block_shape::toggled_open(m),
                OpenRule::Canonical if crate::block_shape::is_open(m) != open => crate::block_shape::toggled_open(m),
                OpenRule::Canonical => m,
            };
            (c, new)
        })
        .collect()
}

/// A door placement's two cells and metas from its bottom and facing bits
/// (`meta::Facing::to_bits`): `None` unless the facing is horizontal.
pub fn door_halves(bottom: [i32; 3], facing: u8) -> Option<[([i32; 3], u8); 2]> {
    let f = crate::meta::Facing::from_bits(facing);
    if f.to_bits() != facing || !matches!(f, crate::meta::Facing::North | crate::meta::Facing::South | crate::meta::Facing::West | crate::meta::Facing::East) {
        return None;
    }
    let m = crate::meta::with_facing(0, f);
    Some([(bottom, m), ([bottom[0], bottom[1] + 1, bottom[2]], crate::use_edits::door_top_meta(m))])
}

/// The Strict client's door placement from hotbar `slot` (its window
/// judging the slot a door): ONE reserved compound `DoorPlace` naming the
/// bottom, its window half (one door taken, refill included) predicted by
/// the server's own rule, then sent. Returns the block the window gave.
#[allow(clippy::too_many_arguments)]
pub fn place_door_joined(
    remote: &mut RemoteClient,
    ui: &mut CraftingUi,
    inv: &mut Inventory,
    armour: &mut [Option<ArmourItem>; 4],
    slot: usize,
    bottom: [i32; 3],
    facing: u8,
    registry: &crate::block::BlockRegistry,
) -> Result<Option<BlockId>, String> {
    let slot_u8 = u8::try_from(slot).ok().filter(|s| *s < 9).ok_or("A door placement needs a hotbar slot")?;
    let halves = door_halves(bottom, facing).ok_or("A door faces a horizontal direction")?;
    ui.sync_auto_refill_joined(remote, inv, armour)?;
    crate::window_prediction::retain_private_window(inv, &ui.cursor_item, &ui.grid)?;
    let raw = crate::staged_window::MarkerWindow::capture_parts(inv, armour, &ui.cursor_item, &ui.grid, registry);
    let mut after = raw.clone();
    crate::strict_edits::place_window(after.view().inv, slot);
    let recipe = crate::strict_edits::EditRecipe::new_place(slot_u8, &raw, &after, ui.station())?;
    let bytes = bincode::serialize(&recipe).map_err(|e| e.to_string())?.into_boxed_slice();
    let intent = crate::action_wire::ActionIntent::DoorPlace { bottom, facing, hotbar_slot: slot_u8 };
    let action = remote.reserve_action(intent, bytes, vec![halves[0].0, halves[1].0].into_boxed_slice())?;
    let placed = crate::strict_edits::place_window(inv, slot);
    if let Err(error) = remote.send_reserved_action(action) {
        log::warn!("Reserved door placement {} could not be sent: {error}", action.seq);
    }
    Ok(placed)
}

/// The Strict client's toggle click on `primary` (holding `expected`,
/// hotbar `slot` selected): the auto-refill setting first, as before every
/// typed action, so both copies' windows agree; then [`reserve_toggle`].
/// The caller predicts [`toggle_metas`] with [`OpenRule::Canonical`] (the
/// server's rule). No window change.
#[allow(clippy::too_many_arguments)]
pub fn toggle_joined(remote: &mut RemoteClient, ui: &mut CraftingUi, inv: &Inventory, armour: &[Option<ArmourItem>; 4], world: &World, primary: [i32; 3], expected: BlockId, slot: usize) -> Result<(), String> {
    ui.sync_auto_refill_joined(remote, inv, armour)?;
    reserve_toggle(remote, world, primary, expected, slot)
}

/// C3d run 19 — a joined toggle click on `primary`: ONE reserved
/// `ToggleBlock` naming the clicked cell, the block the client saw there
/// and its selected hotbar `slot`, sent. A Mirror joiner calls this alone
/// (no window op: a Mirror seat keeps its refill setting on its own path)
/// and predicts [`OpenRule::FlipEach`] exactly as before; the server
/// applies the same rule to its world and every other player hears it.
pub fn reserve_toggle(remote: &mut RemoteClient, world: &World, primary: [i32; 3], expected: BlockId, slot: usize) -> Result<(), String> {
    let hotbar_slot = u8::try_from(slot).ok().filter(|s| *s < 9).ok_or("A toggle needs a hotbar slot")?;
    let cells = toggle_footprint(world, primary).into_boxed_slice();
    let intent = crate::action_wire::ActionIntent::ToggleBlock { cell: primary, expected_block: expected, hotbar_slot };
    let action = remote.reserve_action(intent, Box::new([]), cells)?;
    if let Err(error) = remote.send_reserved_action(action) {
        log::warn!("Reserved toggle {} could not be sent: {error}", action.seq);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{AIR, OAK_FENCE_GATE, STONE};
    use crate::meta::Facing;

    /// The game loop's toggle arm as it stood before run 18 moved it (the
    /// "before"), on a world.
    fn old_toggle(world: &mut World, pos: [i32; 3]) {
        let target_blk = world.get_block(pos[0], pos[1], pos[2]);
        let m = world.meta_at(pos[0], pos[1], pos[2]);
        world.set_meta((pos[0], pos[1], pos[2]), crate::block_shape::toggled_open(m));
        if target_blk == OAK_DOOR {
            let toggle_half = |s: &mut World, x: i32, y: i32, z: i32| {
                if s.get_block(x, y, z) == OAK_DOOR {
                    let hm = s.meta_at(x, y, z);
                    s.set_meta((x, y, z), crate::block_shape::toggled_open(hm));
                }
            };
            let cy = if crate::block_shape::door_is_top(m) { pos[1] - 1 } else { pos[1] + 1 };
            toggle_half(world, pos[0], cy, pos[2]);
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, nz) = (pos[0] + dx, pos[2] + dz);
                if world.get_block(nx, pos[1], nz) == OAK_DOOR {
                    let nm = world.meta_at(nx, pos[1], nz);
                    let ncy = if crate::block_shape::door_is_top(nm) { pos[1] - 1 } else { pos[1] + 1 };
                    toggle_half(world, nx, pos[1], nz);
                    toggle_half(world, nx, ncy, nz);
                }
            }
        }
    }

    pub(crate) fn door(world: &mut World, bottom: [i32; 3], f: Facing, open: bool) {
        let mut m = crate::meta::with_facing(0, f);
        if open { m = crate::block_shape::toggled_open(m); }
        world.set_block(bottom[0], bottom[1], bottom[2], OAK_DOOR);
        world.set_meta((bottom[0], bottom[1], bottom[2]), m);
        world.set_block(bottom[0], bottom[1] + 1, bottom[2], OAK_DOOR);
        world.set_meta((bottom[0], bottom[1] + 1, bottom[2]), crate::use_edits::door_top_meta(m));
    }

    /// Layouts: (builder, clicked cell).
    fn layouts() -> Vec<(Box<dyn Fn(&mut World)>, [i32; 3])> {
        vec![
            (Box::new(|w: &mut World| door(w, [5, 70, 5], Facing::North, false)), [5, 70, 5]),
            (Box::new(|w: &mut World| door(w, [5, 70, 5], Facing::East, false)), [5, 71, 5]),
            (Box::new(|w: &mut World| { door(w, [5, 70, 5], Facing::North, false); door(w, [6, 70, 5], Facing::North, false); }), [6, 71, 5]),
            (Box::new(|w: &mut World| { door(w, [5, 70, 5], Facing::West, false); door(w, [5, 70, 6], Facing::West, false); }), [5, 70, 5]),
            // Mixed: one open beside one shut (FlipEach and Canonical differ).
            (Box::new(|w: &mut World| { door(w, [5, 70, 5], Facing::North, true); door(w, [6, 70, 5], Facing::North, false); }), [5, 70, 5]),
            // Mismatched halves of one door.
            (Box::new(|w: &mut World| { door(w, [5, 70, 5], Facing::South, false); let m = w.meta_at(5, 71, 5); w.set_meta((5, 71, 5), crate::block_shape::toggled_open(m)); }), [5, 70, 5]),
            // A lone half (missing companion), a non-door neighbour.
            (Box::new(|w: &mut World| { door(w, [5, 70, 5], Facing::North, false); w.set_block(5, 71, 5, AIR); w.set_block(6, 70, 5, STONE); }), [5, 70, 5]),
            (Box::new(|w: &mut World| { w.set_block(5, 70, 5, OAK_FENCE_GATE); w.set_meta((5, 70, 5), crate::meta::with_facing(0, Facing::North)); door(w, [6, 70, 5], Facing::North, false); }), [5, 70, 5]),
            (Box::new(|w: &mut World| { w.set_block(5, 70, 5, crate::block::OAK_TRAPDOOR); }), [5, 70, 5]),
        ]
    }

    fn snapshot(w: &World) -> Vec<(BlockId, u8)> {
        let mut v = Vec::new();
        for x in 3..=8 { for y in 68..=73 { for z in 3..=8 { v.push((w.get_block(x, y, z), w.meta_at(x, y, z))); } } }
        v
    }

    /// The move kept the Mirror click byte-for-byte: on every layout the
    /// shared rule's `FlipEach` leaves the world exactly as the old arm did.
    #[test]
    fn c3d_toggle_flip_each_is_the_old_game_loop_arm() {
        for (k, (build, click)) in layouts().into_iter().enumerate() {
            let (mut before, mut after) = (World::new(), World::new());
            build(&mut before);
            build(&mut after);
            old_toggle(&mut before, click);
            for (c, m) in toggle_metas(&after, click, OpenRule::FlipEach) { after.set_meta((c[0], c[1], c[2]), m); }
            assert_eq!(snapshot(&before), snapshot(&after), "layout {k}: the Mirror toggle result is unchanged by the move");
        }
    }

    /// Canonical: every footprint cell ends at the primary's flipped bit,
    /// and agrees with `FlipEach` wherever the footprint agreed already.
    #[test]
    fn c3d_toggle_canonical_bit_comes_from_the_primary() {
        let mut w = World::new();
        door(&mut w, [5, 70, 5], Facing::North, true);
        door(&mut w, [6, 70, 5], Facing::North, false);
        let canon = toggle_metas(&w, [5, 70, 5], OpenRule::Canonical);
        assert_eq!(canon.len(), 4, "both halves of both doors");
        assert!(canon.iter().all(|(_, m)| !crate::block_shape::is_open(*m)), "all shut: the primary was open");
        let mut agree = World::new();
        door(&mut agree, [5, 70, 5], Facing::North, false);
        door(&mut agree, [6, 70, 5], Facing::North, false);
        assert_eq!(toggle_metas(&agree, [6, 71, 5], OpenRule::Canonical), toggle_metas(&agree, [6, 71, 5], OpenRule::FlipEach));
    }

    /// Run 19 — the send-back covers a companion only the client holds,
    /// and the preflight refuses an out-of-reach door (and, run 20, lets an
    /// Adventure player toggle).
    #[test]
    fn c3d_toggle_sendback_and_preflight() {
        let mut w = World::new();
        w.set_block(5, 70, 5, STONE);
        let back = toggle_sendback_cells(&w, [5, 70, 5], OAK_DOOR);
        assert_eq!(back.len(), 15, "the whole bounded candidate set");
        assert!(back.contains(&[6, 71, 5]) && back.contains(&[5, 69, 5]));
        assert_eq!(toggle_sendback_cells(&w, [5, 70, 5], OAK_FENCE_GATE), vec![[5, 70, 5]]);
        let eye = glam::Vec3::new(5.5, 71.6, 2.5);
        assert!(toggle_preflight(crate::play_mode::PlayMode::Survival, eye, [5, 70, 5], None).is_ok());
        // Run 20 (changed from run 19's `is_err`): a toggle is a use, so
        // Adventure (and, as main has it, Spectator) passes the preflight.
        assert!(toggle_preflight(crate::play_mode::PlayMode::Adventure, eye, [5, 70, 5], None).is_ok());
        assert!(toggle_preflight(crate::play_mode::PlayMode::Spectator, eye, [5, 70, 5], None).is_ok());
        assert!(toggle_preflight(crate::play_mode::PlayMode::Survival, eye, [5, 70, 12], None).is_err(), "9.5 blocks bare-handed");
    }

    #[test]
    fn c3d_door_halves_reject_vertical_facings() {
        assert!(door_halves([0, 70, 0], Facing::Up.to_bits()).is_none());
        assert!(door_halves([0, 70, 0], 7).is_none());
        let [(b, bm), (t, tm)] = door_halves([0, 70, 0], Facing::East.to_bits()).unwrap();
        assert_eq!((b, t), ([0, 70, 0], [0, 71, 0]));
        assert!(!crate::block_shape::door_is_top(bm) && crate::block_shape::door_is_top(tm));
    }
}
