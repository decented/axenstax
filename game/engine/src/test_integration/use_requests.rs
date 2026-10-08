//! C3c-2 (2026-10-08, protocol v81) — a joiner's bow, slingshot,
//! cart placement, fishing and campfire lighting are requests the server
//! runs (Spec 04 §4.2f "Use requests").
//!
//! Driven through the C3b-2 rig (`block_use::Rig`): a REAL `HostedServer`
//! over the in-process transport (a dedicated server, or a lending host) and
//! a joiner whose client half is the real one — its window, its requests in
//! flight with their claims (`JoinerActions`), the outcomes and grants
//! applied in arrival order, its window events reported on its next input.
//! A request is sent as `GameState`'s joined arms send it (`send_shot`,
//! `send_place_cart`, `send_fishing`, `send_light`); what the joined client's
//! own sim does (nothing) is the GPU harness's to drive.

use glam::Vec3;

use crate::block;
use crate::block_use::Lighter;
use crate::combat::Attacker;
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::{Item, ItemStack, MaterialId};
use crate::item_actions::ItemNote;
use crate::joiner_actions::{Asked, Pending};
use crate::protocol::{self, ItemAction, ItemActionOutcomePacket, ShotWeapon};
use crate::transport::ClientTransport;

use super::block_use::{Client, Rig, accepted, mat, refused};

impl Client {
    /// Send item action `action` claimed as `asked` (of `held`) from hotbar
    /// slot `hot`, as `GameState::send_use_request` does: claimed, recorded,
    /// sent. `false` when the claim stops it (nothing sent).
    fn request(&mut self, asked: Asked, held: Option<Item>, hot: usize, action: ItemAction) -> bool {
        if !self.actions.can_afford(&self.inv, &self.ui, asked, held.as_ref()) {
            return false;
        }
        let seq = self.actions.record(Pending { kind: asked, mob: None, hotbar_slot: hot, held }, self.input_seq + 1);
        let pkt = protocol::ItemActionPacket { seq, action, events_applied: self.events };
        self.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &pkt));
        true
    }

    /// The held claim for hotbar slot `hot`: (item, kind, id, full).
    fn claim(&self, hot: usize) -> (Option<Item>, u8, u16, protocol::WireItem) {
        let held = self.inv.hotbar_slot(hot).map(|s| s.item.clone());
        let (kind, id) = held.as_ref().map_or(protocol::ItemRef::Empty.to_wire(), |i| crate::inventory::item_to_ref(i).to_wire());
        let full = held.as_ref().map_or(protocol::WireItem::None, crate::inventory::item_to_wire_full);
        (held, kind, id, full)
    }

    /// `GameState::send_shot`: the weapon in slot `hot` along `yaw` /
    /// `pitch`, claiming one of its ammo.
    fn shoot(&mut self, weapon: ShotWeapon, hot: usize, yaw: f32, pitch: f32) -> bool {
        let (held, held_kind, held_id, held_full) = self.claim(hot);
        let Some(Item::Tool(tool)) = held else { return false };
        let ammo = Item::Material(crate::shot::ammo_for(weapon));
        let action = ItemAction::Shoot {
            weapon,
            hotbar_slot: hot as u8,
            held_kind,
            held_id,
            held_full,
            yaw,
            pitch,
            charge: crate::shot::max_charge(weapon),
        };
        self.request(Asked::Shoot { weapon, material: tool.material }, Some(ammo), hot, action)
    }

    /// `GameState::send_place_cart`.
    fn place_cart(&mut self, cell: [i32; 3], hot: usize) -> bool {
        let (held, held_kind, held_id, held_full) = self.claim(hot);
        let action = ItemAction::PlaceCart { cell, hotbar_slot: hot as u8, held_kind, held_id, held_full };
        self.request(Asked::PlaceCart { cell }, held, hot, action)
    }

    /// `GameState::send_fishing`: a cast, or with `reel` a reel.
    fn fish(&mut self, reel: bool, hot: usize) -> bool {
        let (held, held_kind, held_id, held_full) = self.claim(hot);
        let slot = hot as u8;
        let (asked, action) = if reel {
            (Asked::Reel, ItemAction::Reel { hotbar_slot: slot, held_kind, held_id, held_full })
        } else {
            (Asked::Cast, ItemAction::Cast { hotbar_slot: slot, held_kind, held_id, held_full })
        };
        self.request(asked, held, hot, action)
    }

    /// `GameState::send_light`: a campfire `UseBlock` with the lighter in
    /// slot `hot`.
    fn light(&mut self, cell: [i32; 3], hot: usize) -> bool {
        let (held, held_kind, held_id, held_full) = self.claim(hot);
        let Some(lighter) = crate::block_use::lighter_of(held.as_ref()) else { return false };
        let action = ItemAction::UseBlock { cell, hotbar_slot: hot as u8, held_kind, held_id, held_full };
        self.request(Asked::Light { cell, lighter }, held, hot, action)
    }
}

impl Rig {
    /// Joiner `n` sends what `send` sends; the server answers once and the
    /// window events land on both sides.
    fn ask(&mut self, n: usize, send: impl FnOnce(&mut Client) -> bool) -> ItemActionOutcomePacket {
        let before = self.cs[n].outcomes.len();
        assert!(send(&mut self.cs[n]), "sent");
        self.ticks(3);
        assert_eq!(self.cs[n].outcomes.len(), before + 1, "answered once");
        self.cs[n].last().clone()
    }

    /// Spend joiner `n`'s believed bound, so a pay its server copy can't
    /// cover is refused.
    fn spend_bound(&mut self, n: usize) {
        let now = self.hs.server.tick_counter + 1;
        let slot = self.cs[n].slot;
        let b = &mut self.hs.server.players[slot].container_sent.believed;
        while b.try_take(1, now) {}
    }

    /// The projectiles in the world the server simulates, with their
    /// position and owner.
    fn projectiles(&self) -> Vec<(Vec3, crate::entity::ProjectileEntity)> {
        self.ecs()
            .query::<(&crate::entity::Position, &crate::entity::ProjectileEntity)>()
            .iter()
            .map(|(_, (p, pe))| (p.0, crate::entity::ProjectileEntity { damage: pe.damage, is_blunt: pe.is_blunt, owner: pe.owner.clone() }))
            .collect()
    }

    /// Joiner `n`'s body's eye, on the server.
    fn eye(&self, n: usize) -> Vec3 {
        self.hs.server.players[self.cs[n].slot].player.eye_pos()
    }

    /// The tool in slot `slot` of joiner `n`'s client window.
    fn tool_at(&self, n: usize, slot: usize) -> Option<Tool> {
        match self.cs[n].inv.slot(slot).map(|s| &s.item) {
            Some(Item::Tool(t)) => Some(*t),
            _ => None,
        }
    }
}

fn tool(tool_type: ToolType) -> Item {
    Item::Tool(Tool::new(tool_type, ToolMaterial::Wood))
}

/// The yaw and pitch (`camera::forward_from`) that look from `eye` at `at`.
fn aim(eye: Vec3, at: Vec3) -> (f32, f32) {
    let d = (at - eye).normalize();
    ((-d.x).atan2(-d.z), d.y.asin())
}

// ─── Bow and slingshot ─────────────────────────────────────────────────────

/// A joiner's bow and slingshot shots are the SERVER's: one ammo leaves the
/// server's copy and the client's window (an owed take), the weapon wears on
/// both, the real projectile flies in the server's world from the server's
/// own eye for that player and is owned by the joiner (`Attacker::Remote`),
/// so it strikes a server mob and credits the joiner. A shot inside the
/// weapon's cooldown is refused.
#[test]
fn a_joiners_shots_are_the_servers_and_land_on_its_mobs() {
    let mut rig = Rig::dedicated("shoot", 1);
    let slot = rig.cs[0].slot;
    let generation = rig.hs.server.players[slot].attach_gen;
    rig.give(0, 0, tool(ToolType::Bow), 1);
    rig.give(0, 1, tool(ToolType::Slingshot), 1);
    rig.give(0, 12, mat(MaterialId::Arrow), 3);
    rig.give(0, 20, mat(MaterialId::RubberBall), 2);
    let fresh = Tool::new(ToolType::Bow, ToolMaterial::Wood).durability;
    // Straight up first: the arrow starts at the SERVER's eye for us.
    let eye = rig.eye(0);
    assert!(rig.cs[0].shoot(ShotWeapon::Bow, 0, 0.0, 1.5));
    rig.tick();
    let shots = rig.projectiles();
    assert_eq!(shots.len(), 1, "one real arrow in the server's world");
    let (pos, shot) = &shots[0];
    assert!((*pos - eye).length() < 2.5, "from the server's eye: {pos:?} vs {eye:?}");
    assert!(!shot.is_blunt);
    assert_eq!(shot.owner.as_ref().map(|s| s.who), Some(Attacker::Remote { slot, generation }));
    rig.ticks(3);
    let out = rig.cs[0].last().clone();
    accepted(&out, 1);
    assert!(out.wear_held, "the bow wears");
    assert_eq!(rig.cs[0].count(&mat(MaterialId::Arrow)), 2, "one arrow spent");
    assert_eq!(rig.tool_at(0, 0).map(|t| t.durability), Some(fresh - 1));
    rig.assert_lockstep(0, "a shot");
    // Inside the cooldown: refused, nothing spent.
    rig.hs.server.players[slot].next_shot_tick = rig.hs.server.tick_counter + 20;
    refused(&rig.ask(0, |c| c.shoot(ShotWeapon::Bow, 0, 0.0, 1.5)), ItemNote::TooSoon);
    assert_eq!(rig.cs[0].count(&mat(MaterialId::Arrow)), 2);
    rig.hs.server.players[slot].next_shot_tick = 0;
    // A cow ahead: the arrow strikes it, credited to the joiner.
    let cow_at = Vec3::new(rig.at.x, rig.at.y, rig.at.z + 3.0);
    let cow = crate::entity::spawn_mob(&mut rig.hs.server.ecs, crate::mob::MobType::Cow, cow_at);
    let full = rig.ecs().get::<&crate::combat::Health>(cow).unwrap().current;
    let (yaw, pitch) = aim(rig.eye(0), cow_at + Vec3::new(0.0, 0.6, 0.0));
    accepted(&rig.ask(0, |c| c.shoot(ShotWeapon::Bow, 0, yaw, pitch)), 1);
    rig.ticks(5);
    let hp = rig.ecs().get::<&crate::combat::Health>(cow).map(|h| h.current).unwrap_or(0.0);
    assert!(hp < full, "the server's cow took the arrow: {hp} of {full}");
    assert_eq!(rig.ecs().get::<&crate::combat::LastAttacker>(cow).unwrap().0, Attacker::Remote { slot, generation });
    // The slingshot fires a rubber ball (blunt) the same way.
    rig.ticks(10);
    let balls = rig.cs[0].count(&mat(MaterialId::RubberBall));
    accepted(&rig.ask(0, |c| c.shoot(ShotWeapon::Slingshot, 1, 0.0, 1.5)), 1);
    assert_eq!(rig.cs[0].count(&mat(MaterialId::RubberBall)), balls - 1);
    assert!(rig.projectiles().iter().any(|(_, p)| p.is_blunt), "a rubber ball in flight");
    rig.assert_lockstep(0, "a slingshot shot");
}

/// A shot with no ammo the server's copy holds or can believe is refused
/// (`NoAmmo`) and spends nothing; a claim that isn't the weapon is refused
/// too.
#[test]
fn a_shot_with_no_ammo_is_refused_and_spends_nothing() {
    let mut rig = Rig::dedicated("shoot-dry", 1);
    rig.give(0, 0, tool(ToolType::Bow), 1);
    // The client says it has an arrow; the server's copy has none and the
    // bound is spent.
    rig.cs[0].inv.set_slot(9, Some(ItemStack::new_material(MaterialId::Arrow, 1)));
    rig.spend_bound(0);
    refused(&rig.ask(0, |c| c.shoot(ShotWeapon::Bow, 0, 0.0, 1.5)), ItemNote::NoAmmo);
    assert!(rig.projectiles().is_empty(), "nothing fired");
    assert_eq!(rig.cs[0].count(&mat(MaterialId::Arrow)), 1, "the client keeps its arrow");
    let fresh = Tool::new(ToolType::Bow, ToolMaterial::Wood).durability;
    assert_eq!(rig.tool_at(0, 0).map(|t| t.durability), Some(fresh), "the bow didn't wear");
    // A slingshot claimed as a bow: not the weapon it names.
    rig.give(0, 1, tool(ToolType::Slingshot), 1);
    rig.give(0, 10, mat(MaterialId::Arrow), 4);
    let (_, held_kind, held_id, held_full) = rig.cs[0].claim(1);
    let action = ItemAction::Shoot { weapon: ShotWeapon::Bow, hotbar_slot: 1, held_kind, held_id, held_full, yaw: 0.0, pitch: 0.0, charge: 0 };
    let asked = Asked::Shoot { weapon: ShotWeapon::Bow, material: ToolMaterial::Wood };
    refused(&rig.ask(0, |c| c.request(asked, Some(mat(MaterialId::Arrow)), 1, action)), ItemNote::NothingToTake);
    assert!(rig.projectiles().is_empty());
}

/// Two shots in flight with one arrow: the second is judged against the
/// server's copy as it will be once the first's waiting take lands
/// (`window_events::effective_window`), so before the client has even
/// acknowledged the first it finds no arrow — and with the believed bound
/// spent, it is refused (`NoAmmo`). One arrow, one projectile.
#[test]
fn a_second_shot_in_flight_cannot_spend_the_first_shots_arrow() {
    let mut rig = Rig::dedicated("shoot-race", 1);
    let slot = rig.cs[0].slot;
    rig.give(0, 0, tool(ToolType::Bow), 1);
    rig.give(0, 9, mat(MaterialId::Arrow), 1);
    rig.spend_bound(0);
    // A modified client fires twice on one arrow; the first's outcome isn't
    // acknowledged before the second is read.
    assert!(rig.cs[0].shoot(ShotWeapon::Bow, 0, 0.0, 1.5));
    rig.hs.tick();
    assert_eq!(rig.hs.server.players[slot].window_events.waiting(), 2, "the take and the wear wait for the client");
    rig.hs.server.players[slot].next_shot_tick = 0;
    let (_, held_kind, held_id, held_full) = rig.cs[0].claim(0);
    let action = ItemAction::Shoot { weapon: ShotWeapon::Bow, hotbar_slot: 0, held_kind, held_id, held_full, yaw: 0.0, pitch: 1.5, charge: 0 };
    let pkt = protocol::ItemActionPacket { seq: 900, action, events_applied: 0 };
    rig.cs[0].transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &pkt));
    rig.hs.tick();
    rig.ticks(3);
    let outs = &rig.cs[0].outcomes;
    assert_eq!(outs.len(), 2);
    accepted(&outs[0], 1);
    refused(&outs[1], ItemNote::NoAmmo);
    assert_eq!(rig.projectiles().len(), 1, "one arrow, one projectile");
    let sp = &rig.hs.server.players[slot];
    let slots = |inv: &crate::inventory::Inventory| inv.slots_iter().map(|s| s.cloned()).collect::<Vec<_>>();
    assert_eq!(slots(&sp.inventory), slots(&rig.cs[0].inv), "both copies spent the one arrow, once");
    assert_eq!(sp.window_events.waiting(), 0);
    assert_eq!(sp.possession.use_refused, 1, "the second claim is the refused, logged one");
}

/// On a lending host the shot is spawned in the host's lent world, where
/// the host client's projectile tick flies it — and its strike credits the
/// joiner, never a local seat.
#[test]
fn a_shot_on_a_lending_host_flies_in_the_hosts_world_and_credits_the_joiner() {
    let mut rig = Rig::lent("shoot", 1);
    let slot = rig.cs[0].slot;
    rig.give(0, 0, tool(ToolType::Bow), 1);
    rig.give(0, 9, mat(MaterialId::Arrow), 1);
    let cow_at = Vec3::new(rig.at.x, rig.at.y, rig.at.z + 3.0);
    let cow = crate::entity::spawn_mob(&mut rig.host.as_mut().unwrap().ecs, crate::mob::MobType::Cow, cow_at);
    let (yaw, pitch) = aim(rig.eye(0), cow_at + Vec3::new(0.0, 0.6, 0.0));
    accepted(&rig.ask(0, |c| c.shoot(ShotWeapon::Bow, 0, yaw, pitch)), 1);
    assert_eq!(rig.projectiles().len(), 1, "in the host's world, not a second copy");
    let host = rig.host.as_mut().unwrap();
    let registry = block::BlockRegistry::new();
    for _ in 0..6 {
        // The host client's tick, with its joiners' sneak by slot.
        let sneaking: std::collections::HashMap<usize, bool> = [(slot, false)].into_iter().collect();
        crate::entity::tick_projectiles(&mut host.ecs, &host.world, &registry, &sneaking);
    }
    let hit = host.ecs.get::<&crate::combat::LastAttacker>(cow).map(|l| l.0).ok();
    assert!(matches!(hit, Some(Attacker::Remote { slot: s, .. }) if s == slot), "credited to the joiner: {hit:?}");
    rig.assert_lockstep(0, "a lent shot");
}

// ─── Carts ─────────────────────────────────────────────────────────────────

/// A joiner's cart is the SERVER's: placed on the rail in the server's
/// world (the hull of the item), the item taken from both copies; a second
/// cart on the same rail is refused by the server's own guard
/// (`cart::cart_here`), and a cell that is no rail too.
#[test]
fn a_joiners_cart_is_the_servers_and_one_rail_holds_one() {
    for lent in [false, true] {
        let mut rig = if lent { Rig::lent("cart", 1) } else { Rig::dedicated("cart", 1) };
        let rail = rig.place(2, crate::rail::TRACK);
        rig.give(0, 0, mat(MaterialId::IronCart), 2);
        accepted(&rig.ask(0, |c| c.place_cart(rail, 0)), 1);
        let carts: Vec<_> = rig.ecs().query::<&crate::cart::CartData>().iter().map(|(_, c)| (c.cell, c.hull)).collect();
        assert_eq!(carts, vec![((rail[0], rail[1], rail[2]), crate::cart::Hull::Iron)], "lent {lent}");
        assert_eq!(rig.cs[0].count(&mat(MaterialId::IronCart)), 1);
        rig.assert_lockstep(0, "a cart placed");
        refused(&rig.ask(0, |c| c.place_cart(rail, 0)), ItemNote::CartHere);
        assert_eq!(rig.cs[0].count(&mat(MaterialId::IronCart)), 1, "nothing taken");
        let stone = rig.place(-2, block::STONE);
        refused(&rig.ask(0, |c| c.place_cart(stone, 0)), ItemNote::NotThatBlock);
        assert_eq!(rig.ecs().query::<&crate::cart::CartData>().iter().count(), 1, "one cart, one rail");
        rig.assert_lockstep(0, "refusals");
    }
}

// ─── Fishing ───────────────────────────────────────────────────────────────

/// Water in a column ahead of joiner `n`'s eye on the server (its cast
/// ray, at eye height).
fn pond(rig: &mut Rig, n: usize) {
    let eye = rig.eye(n);
    let y = eye.y.floor() as i32;
    for dz in 3..=5 {
        let (x, z) = (eye.x.floor() as i32, eye.z.floor() as i32 + dz);
        rig.world().set_block(x, y, z, block::WATER);
        // A source, so the world's fluid sim keeps it.
        match rig.host.as_mut() {
            Some(h) => h.water.add_source(x, y, z),
            None => rig.hs.server.water.add_source(x, y, z),
        }
    }
}

/// Fishing rolls on the SERVER: a cast at water records the server's bite
/// and answers its wait (`bite_after`); one at no water is refused. A reel
/// before the bite catches and spends nothing; after it, the server's roll
/// is granted and the rod wears on both copies. A reel with no cast is
/// refused.
#[test]
fn a_joiners_cast_and_reel_are_rolled_by_the_server() {
    let mut rig = Rig::dedicated("fish", 1);
    let slot = rig.cs[0].slot;
    let rod = tool(ToolType::FishingRod);
    rig.give(0, 0, rod.clone(), 1);
    let fresh = Tool::new(ToolType::FishingRod, ToolMaterial::Wood).durability;
    // No water: refused, nothing recorded.
    let dry = rig.ask(0, |c| c.fish(false, 0));
    refused(&dry, ItemNote::NoWater);
    assert_eq!(dry.bite_after, 0);
    assert_eq!(rig.hs.server.players[slot].fishing, None);
    // A reel with no line out.
    refused(&rig.ask(0, |c| c.fish(true, 0)), ItemNote::NoLine);
    pond(&mut rig, 0);
    let cast = rig.ask(0, |c| c.fish(false, 0));
    accepted(&cast, 0);
    let wait = u64::from(cast.bite_after);
    assert!((crate::fishing::MIN_WAIT_TICKS..=crate::fishing::MAX_WAIT_TICKS).contains(&wait), "{wait}");
    let bite = rig.hs.server.players[slot].fishing.expect("the server holds the cast");
    // Early: nothing caught or spent, the line comes in.
    refused(&rig.ask(0, |c| c.fish(true, 0)), ItemNote::NothingBit);
    assert_eq!(rig.hs.server.players[slot].fishing, None);
    assert_eq!(rig.tool_at(0, 0).map(|t| t.durability), Some(fresh), "the rod didn't wear");
    assert!(bite > rig.hs.server.tick_counter);
    // Cast again, wait for the server's bite, reel.
    let cast = rig.ask(0, |c| c.fish(false, 0));
    rig.ticks(usize::from(cast.bite_after) + 1);
    let reel = rig.ask(0, |c| c.fish(true, 0));
    accepted(&reel, 0);
    assert!(reel.wear_held, "the rod wears");
    let caught: u32 = [MaterialId::RawFish, MaterialId::Bone, MaterialId::Leather]
        .into_iter()
        .map(|m| rig.cs[0].count(&mat(m)))
        .sum();
    assert!(caught >= 1, "the server's catch was granted");
    assert_eq!(rig.tool_at(0, 0).map(|t| t.durability), Some(fresh - 1));
    rig.assert_lockstep(0, "a catch");
}

/// A full joiner's catch comes back from its client (`GrantUnfit`) and the
/// server spills it at its feet: never lost.
#[test]
fn a_full_joiners_catch_is_spilled_not_lost() {
    let mut rig = Rig::lent("fish-full", 1);
    rig.give(0, 0, tool(ToolType::FishingRod), 1);
    for k in 1..36 {
        rig.give(0, k, Item::Block(block::STONE), 64);
    }
    pond(&mut rig, 0);
    let cast = rig.ask(0, |c| c.fish(false, 0));
    accepted(&cast, 0);
    rig.ticks(usize::from(cast.bite_after) + 1);
    accepted(&rig.ask(0, |c| c.fish(true, 0)), 0);
    rig.ticks(3);
    assert!(rig.cs[0].unfit > 0, "the client couldn't hold it");
    let spilled: u32 =
        [MaterialId::RawFish, MaterialId::Bone, MaterialId::Leather].into_iter().map(|m| rig.ground(&mat(m))).sum();
    assert_eq!(spilled, rig.cs[0].unfit, "every unit that didn't fit is a ground item");
}

// ─── Campfire lighting ─────────────────────────────────────────────────────

/// An unlit campfire at `dz`, with `fuel` ticks of fuel, on the server and
/// in the joiner's view.
fn unlit_fire(rig: &mut Rig, dz: i32, fuel: u32) -> [i32; 3] {
    let cell = rig.place(dz, block::CAMPFIRE_UNLIT);
    let pos = (cell[0], cell[1], cell[2]);
    if fuel > 0 {
        let cf = crate::campfire::CampfireData { fuel_ticks: fuel, ..Default::default() };
        rig.world().insert_campfire(pos, cf.clone());
        rig.cs[0].world.insert_campfire(pos, cf);
    }
    cell
}

/// A stick's friction is rolled on the SERVER and the stick is taken on
/// every resolve, hit or miss; a hit lights the server's fire, which every
/// joiner sees. With no fuel it is refused and the stick kept.
#[test]
fn friction_rolls_on_the_server_and_spends_the_stick_hit_or_miss() {
    let mut rig = Rig::dedicated("friction", 1);
    rig.give(0, 0, mat(MaterialId::Stick), 64);
    let cold = unlit_fire(&mut rig, -2, 0);
    refused(&rig.ask(0, |c| c.light(cold, 0)), ItemNote::FrictionNeedsFuel);
    assert_eq!(rig.cs[0].count(&mat(MaterialId::Stick)), 64, "kept");
    let fire = unlit_fire(&mut rig, 2, 4_000);
    let (mut hits, mut misses) = (0, 0);
    for k in 0..40u32 {
        let out = rig.ask(0, |c| c.light(fire, 0));
        assert!(out.accepted, "{out:?}");
        assert_eq!(out.consume_held, 1, "the stick, hit or miss");
        assert_eq!(rig.cs[0].count(&mat(MaterialId::Stick)), 63 - k);
        let lit = rig.world().get_block(fire[0], fire[1], fire[2]) == block::CAMPFIRE;
        match ItemNote::from_wire(out.note) {
            ItemNote::None => {
                assert!(lit, "a strike lights the server's fire");
                assert_eq!(rig.cs[0].world.get_block(fire[0], fire[1], fire[2]), block::CAMPFIRE, "and the joiner sees it");
                hits += 1;
                rig.set(fire, block::CAMPFIRE_UNLIT);
            }
            ItemNote::NotDryEnough => {
                assert!(!lit, "a miss lights nothing");
                misses += 1;
            }
            other => panic!("{other:?}"),
        }
        if hits > 0 && misses > 0 {
            break;
        }
        rig.ticks(1);
    }
    assert!(hits > 0 && misses > 0, "the server's roll both strikes and misses: {hits}/{misses}");
    rig.assert_lockstep(0, "friction");
}

/// Flint and steel wears on every strike — even with no fuel, when nothing
/// lights — and lights a fuelled fire; the Magnesium Firestarter lights one
/// and never wears, and with no fuel is refused.
#[test]
fn flint_wears_even_without_fuel_and_the_firestarter_never_does() {
    let mut rig = Rig::dedicated("flint", 1);
    rig.give(0, 0, Item::Tool(Tool::new(ToolType::FlintAndSteel, ToolMaterial::Iron)), 1);
    rig.give(0, 1, mat(MaterialId::MagnesiumFirestarter), 1);
    let fresh = Tool::new(ToolType::FlintAndSteel, ToolMaterial::Iron).durability;
    let cold = unlit_fire(&mut rig, -2, 0);
    let out = rig.ask(0, |c| c.light(cold, 0));
    accepted(&ItemActionOutcomePacket { note: 0, ..out.clone() }, 0);
    assert_eq!(ItemNote::from_wire(out.note), ItemNote::NeedsFuel);
    assert!(out.wear_held, "the steel wore down");
    assert_eq!(rig.tool_at(0, 0).map(|t| t.durability), Some(fresh - 1));
    assert_eq!(rig.world().get_block(cold[0], cold[1], cold[2]), block::CAMPFIRE_UNLIT, "nothing lit");
    refused(&rig.ask(0, |c| c.light(cold, 1)), ItemNote::NeedsFuel);
    let fire = unlit_fire(&mut rig, 2, 4_000);
    let out = rig.ask(0, |c| c.light(fire, 0));
    accepted(&out, 0);
    assert!(out.wear_held);
    assert_eq!(rig.tool_at(0, 0).map(|t| t.durability), Some(fresh - 2));
    assert_eq!(rig.world().get_block(fire[0], fire[1], fire[2]), block::CAMPFIRE);
    rig.set(fire, block::CAMPFIRE_UNLIT);
    let out = rig.ask(0, |c| c.light(fire, 1));
    accepted(&out, 0);
    assert!(!out.wear_held, "the Firestarter never wears");
    assert_eq!(rig.cs[0].count(&mat(MaterialId::MagnesiumFirestarter)), 1);
    assert_eq!(rig.world().get_block(fire[0], fire[1], fire[2]), block::CAMPFIRE);
    rig.assert_lockstep(0, "flint and the Firestarter");
}

/// The lighting claims: a stick on its way can't be spent twice; flint and
/// steel claims its wear; the Firestarter claims nothing.
#[test]
fn a_lighting_in_flight_claims_its_stick_or_its_flint() {
    let mut rig = Rig::dedicated("light-claim", 1);
    rig.give(0, 0, mat(MaterialId::Stick), 1);
    let fire = unlit_fire(&mut rig, 2, 4_000);
    assert!(rig.cs[0].light(fire, 0));
    assert!(!rig.cs[0].light(fire, 0), "the only stick is in flight");
    assert_eq!(crate::joiner_actions::uses(Asked::Light { cell: fire, lighter: Lighter::FlintAndSteel }), 1);
    assert_eq!(crate::joiner_actions::uses(Asked::Light { cell: fire, lighter: Lighter::Firestarter }), 0);
    rig.ticks(3);
    assert_eq!(rig.cs[0].outcomes.len(), 1);
}
