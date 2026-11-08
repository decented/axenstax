//! Living world S3 — the Feast (spec `docs/superpowers/specs/2026-11-08-living-world-tier1.md` §3).
//!
//! ONE rule for every path. Using a [`block::FEAST_TABLE`] while holding a
//! stack shares the whole stack out among the table's **guests** and turns the
//! table into a [`block::FEAST_MARKER`] whose `block_meta` byte is the guest
//! count. This module is the pure part: who counts as a guest
//! ([`gather`]), how the stack splits ([`hold`]), what the feast is called
//! ([`name_at`]), and the two world effects every path shares ([`convert`],
//! [`spawn_drops`]). The CALLERS find the guests, each from what it knows:
//!
//! - **Single-player and a host's own seats** (`GameState`'s right-click arm):
//!   the other local seats (`GameState.players`), the joiners the host draws
//!   (`GameState.remote_players`) and the ECS villagers.
//! - **The authority, for a joiner's `ItemAction::UseBlock`**
//!   (`HostedServer::serve_feast`): `GameServer.players` that are present and
//!   alive, except the holder (so a lent host's own seats count), and its ECS.
//!   The authority spawns every player-guest drop, so a lent host and a
//!   dedicated server behave the same.
//!
//! Guests (decision D11): other players within [`PLAYER_RADIUS`] blocks of the
//! table and villagers or peddlers within [`VILLAGER_RADIUS`] (the nearest
//! [`MAX_VILLAGERS`]). The holder is never a guest. With none, nothing happens
//! and nothing is used up.
//!
//! The marker's meta byte is the guest count and needs no bit layout: every
//! consumer of `block_meta` is keyed on its own block id (water level, door
//! half, device facing, rail power, snow depth), and none of them is this
//! block. It travels as `BlockChange.meta`, in the B2a chunk push's
//! `block_meta` rows and in `WorldSave.block_meta`, like any meta byte.

use glam::Vec3;

use crate::block;
use crate::item::ItemStack;
use crate::world::World;

/// How far from the table another player may be and still be a guest.
pub const PLAYER_RADIUS: f32 = 48.0;
/// How far from the table a villager or peddler may be and still be a guest.
pub const VILLAGER_RADIUS: f32 = 32.0;
/// At most this many villagers are guests (the nearest).
pub const MAX_VILLAGERS: usize = 12;

/// The toast when there is no-one to feed.
pub const NEEDS_GUESTS_TOAST: &str = "A feast needs guests. Bring friends, or find a village.";
/// The toast when the hand is empty.
pub const NOTHING_TO_SHARE_TOAST: &str = "Hold something to share at the feast.";

/// Who a guest is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestKind {
    /// A player: the caller's own index for them (a `GameServer` slot, or a
    /// seat / remote-player index on the local path). Only the caller reads it.
    Player(usize),
    /// A villager or peddler in the ECS.
    Villager(hecs::Entity),
}

/// One guest: who, and where they stand.
#[derive(Clone, Copy, Debug)]
pub struct Guest {
    pub kind: GuestKind,
    pub pos: Vec3,
}

/// Why a feast could not be held. Nothing is used up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No guest within reach.
    NeedsGuests,
    /// Nothing in hand to share.
    NothingToShare,
}

impl Refusal {
    /// The toast this refusal shows.
    pub fn toast(self) -> &'static str {
        match self {
            Refusal::NeedsGuests => NEEDS_GUESTS_TOAST,
            Refusal::NothingToShare => NOTHING_TO_SHARE_TOAST,
        }
    }
}

/// One player guest's share, dropped at their feet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerShare {
    pub player: usize,
    pub pos: Vec3,
    pub count: u8,
}

/// A feast that can be held: what it costs, who gets what, and what it is called.
#[derive(Clone, Debug, PartialEq)]
pub struct Feast {
    /// The whole held stack (units used up).
    pub pay: u8,
    /// Guests fed, players and villagers (the marker's meta byte).
    pub guests: u8,
    /// Each player guest's share, nearest first, zero shares left out.
    pub shares: Vec<PlayerShare>,
    /// Units that went to villagers: used up there.
    pub villager_units: u32,
    /// "The Feast of <Name>".
    pub name: String,
}

/// The marker's meta byte for `guests` guests (0-255).
pub fn guest_meta(guests: usize) -> u8 {
    guests.min(usize::from(u8::MAX)) as u8
}

fn dist_sq(a: Vec3, b: Vec3) -> f32 {
    a.distance_squared(b)
}

/// The guests for a table at `table` (the cell's centre): `players` within
/// [`PLAYER_RADIUS`] and `villagers` within [`VILLAGER_RADIUS`] (the nearest
/// [`MAX_VILLAGERS`]), nearest first. The caller leaves the holder out of
/// `players`. Non-finite positions are never guests.
pub fn gather(
    table: Vec3,
    players: impl IntoIterator<Item = (usize, Vec3)>,
    villagers: impl IntoIterator<Item = (hecs::Entity, Vec3)>,
) -> Vec<Guest> {
    let mut near_players: Vec<Guest> = players
        .into_iter()
        .filter(|(_, p)| p.is_finite() && dist_sq(*p, table) <= PLAYER_RADIUS * PLAYER_RADIUS)
        .map(|(id, pos)| Guest { kind: GuestKind::Player(id), pos })
        .collect();
    let mut near_villagers: Vec<Guest> = villagers
        .into_iter()
        .filter(|(_, p)| p.is_finite() && dist_sq(*p, table) <= VILLAGER_RADIUS * VILLAGER_RADIUS)
        .map(|(e, pos)| Guest { kind: GuestKind::Villager(e), pos })
        .collect();
    sort_nearest(table, &mut near_villagers);
    near_villagers.truncate(MAX_VILLAGERS);
    near_players.append(&mut near_villagers);
    sort_nearest(table, &mut near_players);
    near_players
}

fn kind_order(k: &GuestKind) -> (u8, u64) {
    match k {
        GuestKind::Player(i) => (0, *i as u64),
        GuestKind::Villager(e) => (1, e.to_bits().get()),
    }
}

/// Nearest first; ties by player before villager, then by id: deterministic.
fn sort_nearest(table: Vec3, guests: &mut [Guest]) {
    guests.sort_by(|a, b| {
        dist_sq(a.pos, table)
            .total_cmp(&dist_sq(b.pos, table))
            .then_with(|| kind_order(&a.kind).cmp(&kind_order(&b.kind)))
    });
}

/// The centre of the cell `cell`.
pub fn cell_centre(cell: (i32, i32, i32)) -> Vec3 {
    Vec3::new(cell.0 as f32 + 0.5, cell.1 as f32 + 0.5, cell.2 as f32 + 0.5)
}

/// Hold a feast at `cell` with `held`, among `guests`. Splits the whole stack
/// evenly; the remainder goes one apiece to the nearest guests. Villagers'
/// shares are used up, players' are dropped at their feet by the caller
/// ([`spawn_drops`]). `guests` need not be sorted.
pub fn hold(cell: (i32, i32, i32), held: Option<&ItemStack>, guests: &[Guest]) -> Result<Feast, Refusal> {
    if guests.is_empty() {
        return Err(Refusal::NeedsGuests);
    }
    let stack = held.filter(|s| s.count > 0).ok_or(Refusal::NothingToShare)?;
    let mut sorted = guests.to_vec();
    sort_nearest(cell_centre(cell), &mut sorted);
    let n = usize::from(stack.count);
    let (base, rem) = (n / sorted.len(), n % sorted.len());
    let mut shares = Vec::new();
    let mut villager_units = 0u32;
    for (i, g) in sorted.iter().enumerate() {
        let units = base + usize::from(i < rem);
        if units == 0 {
            continue;
        }
        match g.kind {
            GuestKind::Player(player) => shares.push(PlayerShare { player, pos: g.pos, count: units as u8 }),
            GuestKind::Villager(_) => villager_units += units as u32,
        }
    }
    Ok(Feast {
        pay: stack.count,
        guests: guest_meta(sorted.len()),
        shares,
        villager_units,
        name: name_at(cell.0, cell.2),
    })
}

const FIRST: [&str; 16] = [
    "Long", "Old", "Bright", "Birch", "Mill", "Quiet", "Green", "Amber", "Wren", "Hazel", "Fern", "Heron",
    "Willow", "Barley", "Linden", "Sunny",
];
const SECOND: [&str; 16] = [
    "Hollow", "Rise", "Meadow", "Bridge", "Square", "Ford", "Reach", "Common", "Hall", "Bank", "Cross", "Wood",
    "Hill", "Mere", "Lane", "Steading",
];

/// The feast's name, from the table's column: "The Feast of Long Hollow".
/// Deterministic (the same column always gives the same name, on every client
/// and the authority) and drawn from two short word lists.
pub fn name_at(x: i32, z: i32) -> String {
    let mut h = (x as u32 as u64).wrapping_mul(0x9E37_79B1_85EB_CA87) ^ (z as u32 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    let first = FIRST[(h & 15) as usize];
    let second = SECOND[((h >> 8) & 15) as usize];
    format!("The Feast of {first} {second}")
}

/// "The Feast of Long Hollow, 5 guests." (the holder's toast, and a Feast
/// Marker's when used).
pub fn toast(name: &str, guests: u8) -> String {
    format!("{name}, {guests} {}.", if guests == 1 { "guest" } else { "guests" })
}

/// A Feast Table or a Feast Marker: the two blocks a right-click is for the
/// feast (hold it, or read the marker) and never for eating held food.
pub fn is_feast_block(b: block::BlockId) -> bool {
    b == block::FEAST_TABLE || b == block::FEAST_MARKER
}

/// The villagers and peddlers in `ecs` with their positions, for [`gather`].
pub fn villagers_in(ecs: &hecs::World) -> Vec<(hecs::Entity, Vec3)> {
    ecs.query::<(&crate::entity::Position, &crate::entity::MobKind)>()
        .iter()
        .filter(|(_, (_, k))| crate::villager::is_villager_kind(k.0))
        .map(|(e, (p, _))| (e, p.0))
        .collect()
}

/// The world half of a held feast, shared by every path: the table at `cell`
/// becomes a Feast Marker carrying the guest count as its meta byte, and the
/// feast is noted for the clients' confetti and bloom
/// (`World::feast_events`, drained by `GameState`). Returns the block change
/// to broadcast. The placed-by-a-player bit is left as the table had it.
pub fn convert(world: &mut World, cell: (i32, i32, i32), guests: u8) -> crate::protocol::BlockChange {
    world.set_block(cell.0, cell.1, cell.2, block::FEAST_MARKER);
    world.set_meta(cell, guests);
    world.note_feast(cell, guests);
    crate::protocol::BlockChange { x: cell.0, y: cell.1, z: cell.2, new_block: block::FEAST_MARKER, meta: guests }
}

/// Drop each player guest's share of `held` as a real item at their feet.
pub fn spawn_drops(ecs: &mut hecs::World, held: &ItemStack, feast: &Feast) {
    for (k, share) in feast.shares.iter().enumerate() {
        let stack = ItemStack { item: held.item.clone(), count: share.count };
        crate::entity::spawn_item(ecs, share.pos, stack, 0xFEA5_7000u32.wrapping_add(k as u32 * 7919));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::MaterialId;

    fn bread(n: u8) -> ItemStack {
        ItemStack::new_material(MaterialId::Bread, n)
    }

    fn player(id: usize, x: f32) -> (usize, Vec3) {
        (id, Vec3::new(x, 64.0, 0.5))
    }

    fn ecs_with_villagers(xs: &[f32]) -> (hecs::World, Vec<(hecs::Entity, Vec3)>) {
        let mut ecs = hecs::World::new();
        for &x in xs {
            ecs.spawn((
                crate::entity::Position(Vec3::new(x, 64.0, 0.5)),
                crate::entity::MobKind(crate::mob::MobType::Villager),
            ));
        }
        let v = villagers_in(&ecs);
        (ecs, v)
    }

    const CELL: (i32, i32, i32) = (0, 64, 0);

    #[test]
    fn guests_are_players_within_48_and_villagers_within_32() {
        let (_ecs, villagers) = ecs_with_villagers(&[31.0, 33.0]);
        let guests = gather(
            cell_centre(CELL),
            [player(1, 47.0), player(2, 49.0), player(3, -20.0)],
            villagers,
        );
        let players: Vec<usize> = guests
            .iter()
            .filter_map(|g| if let GuestKind::Player(i) = g.kind { Some(i) } else { None })
            .collect();
        assert_eq!(players.len(), 2, "49 blocks is out of reach: {players:?}");
        assert!(players.contains(&1) && players.contains(&3) && !players.contains(&2));
        let villagers = guests.iter().filter(|g| matches!(g.kind, GuestKind::Villager(_))).count();
        assert_eq!(villagers, 1, "the villager at 33 blocks is not a guest");
    }

    #[test]
    fn villagers_are_capped_at_twelve_nearest() {
        let xs: Vec<f32> = (0..20).map(|i| 1.0 + i as f32).collect();
        let (_ecs, villagers) = ecs_with_villagers(&xs);
        let guests = gather(cell_centre(CELL), std::iter::empty(), villagers);
        assert_eq!(guests.len(), MAX_VILLAGERS);
        let farthest = guests.iter().map(|g| g.pos.x).fold(f32::MIN, f32::max);
        assert!(farthest <= 12.0, "the twelve nearest stay: farthest at {farthest}");
        // Players are never capped.
        let many: Vec<_> = (0..30).map(|i| player(i, 2.0 + i as f32 * 0.5)).collect();
        assert_eq!(gather(cell_centre(CELL), many, std::iter::empty()).len(), 30);
    }

    #[test]
    fn guests_come_nearest_first_and_non_finite_positions_never_count() {
        let guests = gather(
            cell_centre(CELL),
            [player(1, 30.0), player(2, 5.0), (3, Vec3::new(f32::NAN, 64.0, 0.0)), player(4, 12.0)],
            std::iter::empty(),
        );
        let order: Vec<_> = guests.iter().map(|g| g.kind).collect();
        assert_eq!(order, vec![GuestKind::Player(2), GuestKind::Player(4), GuestKind::Player(1)]);
    }

    #[test]
    fn no_guests_refuses_and_an_empty_hand_refuses() {
        assert_eq!(hold(CELL, Some(&bread(8)), &[]), Err(Refusal::NeedsGuests));
        let g = [Guest { kind: GuestKind::Player(1), pos: Vec3::new(3.0, 64.0, 0.0) }];
        assert_eq!(hold(CELL, None, &g), Err(Refusal::NothingToShare));
        assert_eq!(hold(CELL, Some(&bread(0)), &g), Err(Refusal::NothingToShare));
        assert_eq!(Refusal::NeedsGuests.toast(), "A feast needs guests. Bring friends, or find a village.");
    }

    #[test]
    fn the_whole_stack_splits_evenly_and_the_remainder_goes_to_the_nearest() {
        let guests = [
            Guest { kind: GuestKind::Player(7), pos: Vec3::new(20.0, 64.0, 0.0) },
            Guest { kind: GuestKind::Player(3), pos: Vec3::new(4.0, 64.0, 0.0) },
            Guest { kind: GuestKind::Player(5), pos: Vec3::new(9.0, 64.0, 0.0) },
        ];
        // 10 over 3: 3 each, the one left over to the nearest (player 3).
        let f = hold(CELL, Some(&bread(10)), &guests).unwrap();
        assert_eq!(f.pay, 10);
        assert_eq!(f.guests, 3);
        let got: Vec<(usize, u8)> = f.shares.iter().map(|s| (s.player, s.count)).collect();
        assert_eq!(got, vec![(3, 4), (5, 3), (7, 3)]);
        assert_eq!(f.villager_units, 0);
        assert_eq!(f.shares.iter().map(|s| u32::from(s.count)).sum::<u32>(), 10);
        // 11 over 3: the two left over go to the two nearest.
        let f = hold(CELL, Some(&bread(11)), &guests).unwrap();
        let got: Vec<(usize, u8)> = f.shares.iter().map(|s| (s.player, s.count)).collect();
        assert_eq!(got, vec![(3, 4), (5, 4), (7, 3)]);
    }

    #[test]
    fn villager_shares_are_used_up_and_a_small_stack_feeds_the_nearest_only() {
        let (_ecs, villagers) = ecs_with_villagers(&[2.0, 3.0]);
        let guests = gather(cell_centre(CELL), [player(1, 10.0)], villagers);
        let f = hold(CELL, Some(&bread(9)), &guests).unwrap();
        assert_eq!(f.guests, 3);
        assert_eq!(f.shares.len(), 1);
        assert_eq!(f.shares[0].count, 3, "a third for the player");
        assert_eq!(f.villager_units, 6, "two thirds used up by the villagers");
        // One loaf, three guests: only the nearest is served, but all three count.
        let f = hold(CELL, Some(&bread(1)), &guests).unwrap();
        assert_eq!(f.guests, 3);
        assert_eq!(f.shares.iter().map(|s| u32::from(s.count)).sum::<u32>() + f.villager_units, 1);
    }

    #[test]
    fn the_guest_count_is_capped_for_the_meta_byte() {
        assert_eq!(guest_meta(0), 0);
        assert_eq!(guest_meta(200), 200);
        assert_eq!(guest_meta(300), 255);
        let many: Vec<Guest> = (0..300)
            .map(|i| Guest { kind: GuestKind::Player(i), pos: Vec3::new(1.0 + i as f32 * 0.1, 64.0, 0.0) })
            .collect();
        assert_eq!(hold(CELL, Some(&bread(64)), &many).unwrap().guests, 255);
    }

    #[test]
    fn the_name_is_deterministic_and_varies_with_the_place() {
        assert_eq!(name_at(10, -4), name_at(10, -4));
        assert!(name_at(10, -4).starts_with("The Feast of "));
        let names: std::collections::HashSet<String> =
            (0..64).flat_map(|x| (0..8).map(move |z| name_at(x * 13, z * 7))).collect();
        assert!(names.len() > 40, "{} distinct names over 512 places", names.len());
    }

    #[test]
    fn no_name_or_toast_carries_a_money_word() {
        let banned: std::collections::HashSet<&str> = crate::copy_lint::BANNED_MONEY_WORDS.iter().copied().collect();
        let mut words: Vec<String> = FIRST.iter().chain(SECOND.iter()).map(|w| w.to_lowercase()).collect();
        for t in [NEEDS_GUESTS_TOAST, NOTHING_TO_SHARE_TOAST, &toast("The Feast of Long Hollow", 5)] {
            words.extend(t.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase));
        }
        for w in words {
            assert!(!banned.contains(w.as_str()), "{w:?} is a banned money word");
        }
    }

    #[test]
    fn the_toast_counts_guests_in_plain_words() {
        assert_eq!(toast("The Feast of Long Hollow", 5), "The Feast of Long Hollow, 5 guests.");
        assert_eq!(toast("The Feast of Long Hollow", 1), "The Feast of Long Hollow, 1 guest.");
    }

    #[test]
    fn convert_turns_the_table_into_a_marker_with_its_meta_and_notes_the_feast() {
        let mut w = World::new();
        w.set_block(5, 64, 5, block::FEAST_TABLE);
        let bc = convert(&mut w, (5, 64, 5), 7);
        assert_eq!(w.get_block(5, 64, 5), block::FEAST_MARKER);
        assert_eq!(w.meta_at(5, 64, 5), 7);
        assert_eq!((bc.new_block, bc.meta), (block::FEAST_MARKER, 7));
        assert_eq!(w.feast_events, vec![(5, 64, 5, 7)]);
        assert_eq!(w.feast_markers(), vec![(5, 5, 7)]);
    }

    #[test]
    fn drops_land_at_each_player_guests_feet_with_their_share() {
        let guests = [
            Guest { kind: GuestKind::Player(1), pos: Vec3::new(4.0, 64.0, 0.0) },
            Guest { kind: GuestKind::Player(2), pos: Vec3::new(9.0, 64.0, 0.0) },
        ];
        let held = bread(5);
        let f = hold(CELL, Some(&held), &guests).unwrap();
        let mut ecs = hecs::World::new();
        spawn_drops(&mut ecs, &held, &f);
        let mut found: Vec<(i32, u8)> = ecs
            .query::<(&crate::entity::Position, &crate::entity::ItemEntity)>()
            .iter()
            .map(|(_, (p, d))| (p.0.x.round() as i32, d.stack.count))
            .collect();
        found.sort();
        assert_eq!(found, vec![(4, 3), (9, 2)]);
    }
}
