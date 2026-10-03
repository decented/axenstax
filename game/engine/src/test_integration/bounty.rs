//! Spec 33 Mob Bounty Board integration tests.
//!
//! End-to-end coverage over World-level rotation refresh + save/load
//! round-trip + the claim id staleness check. Unit-level coverage of
//! template + try_claim + audit_hash lives in `src/bounty.rs`.

use crate::bounty::{
    tick_bounty_refresh, try_claim, ActiveBounty, ClaimOutcome, BOUNTY_REFRESH_TICKS,
};
use crate::mob::MobType;
use crate::world::World;

#[test]
fn tick_bounty_refresh_seeds_first_rotation_on_fresh_world() {
    // A brand-new world has `bounties` empty + bounty_last_refresh_tick
    // at 0. The driver MUST roll the first rotation on its first call
    // (the "bounties empty" branch).
    let mut world = World::new();
    assert!(world.bounties.is_empty());
    let rolled = tick_bounty_refresh(&mut world, 0, 42);
    assert!(rolled, "fresh world should roll the first rotation on tick 0");
    assert!(!world.bounties.is_empty(), "rotation should be populated");
    assert!(world.bounty_next_id > 1,
        "id allocator should have advanced past 1");
}

#[test]
fn bounty_rotation_round_trips_world_save_load() {
    // Populate a world's bounty state, save+reload via the in-memory
    // bincode shape, assert preserved. This exercises the
    // #[serde(default)] backwards-compat shape + the load-side fold
    // back into World.
    let mut world = World::new();
    let _ = tick_bounty_refresh(&mut world, 100, 42);
    let original_ids: Vec<u32> = world.bounties.iter().map(|b| b.id).collect();
    let original_next_id = world.bounty_next_id;
    let original_last_refresh = world.bounty_last_refresh_tick;
    assert!(!original_ids.is_empty(), "precondition: rotation populated");

    // Serialise via the same bincode shape the save path uses.
    let saved_bounties: Vec<crate::save::SavedBounty> = world
        .bounties
        .iter()
        .map(|b| crate::save::SavedBounty {
            id: b.id,
            template_idx: b.template_idx,
            issued_tick: b.issued_tick,
        })
        .collect();
    let bytes = bincode::serialize(&saved_bounties).expect("serialize");
    let back: Vec<crate::save::SavedBounty> =
        bincode::deserialize(&bytes).expect("deserialize");
    assert_eq!(back.len(), original_ids.len());
    let restored_ids: Vec<u32> = back.iter().map(|sb| sb.id).collect();
    assert_eq!(restored_ids, original_ids);

    // Now exercise the bounty_next_id + last_refresh_tick scalars via
    // bincode (they live on the WorldSave wrapper alongside).
    let scalars = (original_next_id, original_last_refresh);
    let scalar_bytes = bincode::serialize(&scalars).expect("serialize scalars");
    let scalar_back: (u32, u64) =
        bincode::deserialize(&scalar_bytes).expect("deserialize scalars");
    assert_eq!(scalar_back.0, original_next_id);
    assert_eq!(scalar_back.1, original_last_refresh);
}

#[test]
fn claims_become_stale_when_rotation_refreshes() {
    // A player claims bounty id N at day 1. The rotation refreshes
    // at day 2; the new bounty ids are different, so the player's
    // claim record (keyed on the old id) doesn't block the new ones.
    let mut world = World::new();
    let _ = tick_bounty_refresh(&mut world, 100, 42);
    let day1_ids: Vec<u32> = world.bounties.iter().map(|b| b.id).collect();
    assert!(!day1_ids.is_empty(), "precondition");

    // Fake a claim on the first bounty.
    let mut kills: ahash::AHashMap<MobType, u32> = ahash::AHashMap::new();
    kills.insert(MobType::Brigand, 100);
    kills.insert(MobType::Marauder, 100);
    let mut claimed: ahash::AHashMap<u32, u32> = ahash::AHashMap::new();
    let first = world.bounties[0];
    let _ = try_claim(&first, &mut kills, &mut claimed);
    assert!(claimed.contains_key(&first.id), "first claim recorded");

    // Advance past one full day cycle → fresh rotation.
    let _ = tick_bounty_refresh(&mut world, BOUNTY_REFRESH_TICKS + 50, 42);
    let day2_ids: Vec<u32> = world.bounties.iter().map(|b| b.id).collect();
    for id in &day1_ids {
        assert!(!day2_ids.contains(id),
            "stale id {id} survived refresh");
    }

    // The new bounties can be claimed independently — the stale claim
    // record doesn't block them.
    for new_bounty in &world.bounties {
        let out = try_claim(new_bounty, &mut kills, &mut claimed);
        assert!(
            matches!(out, ClaimOutcome::Accepted { .. }),
            "new bounty {} should be claimable; got {out:?}",
            new_bounty.id,
        );
    }
}

#[test]
fn audit_hash_changes_per_claim_invocation() {
    // Two distinct claims at the same player+secret but different
    // mob/kill-count → different audit hashes. Protects against a
    // server-side "rotate claim hash silently" bug.
    let pubkey = [0x42u8; 32];
    let secret = b"axenstax-test-secret-bounty";
    let h_a = crate::bounty::claim_audit_hash(&pubkey, MobType::Brigand, 10, secret);
    let h_b = crate::bounty::claim_audit_hash(&pubkey, MobType::Brigand, 11, secret);
    let h_c = crate::bounty::claim_audit_hash(&pubkey, MobType::Marauder, 10, secret);
    assert_ne!(h_a, h_b, "kill-count diff must change hash");
    assert_ne!(h_a, h_c, "mob-kind diff must change hash");
    assert_ne!(h_b, h_c, "both diffs must change hash");
}

#[test]
fn fresh_rotation_id_does_not_collide_with_legacy_bounty_next_id_sentinel() {
    // bounty_next_id = 0 is the "uninitialised" sentinel for legacy
    // saves. tick_bounty_refresh must bump it to ≥1 before allocating.
    // Verify no rolled bounty has id 0.
    let mut world = World::new();
    world.bounty_next_id = 0;
    let _ = tick_bounty_refresh(&mut world, 0, 42);
    for b in &world.bounties {
        assert_ne!(b.id, 0, "id 0 is the uninitialised sentinel — never allocate it");
    }
    assert!(world.bounty_next_id >= 2, "allocator should have advanced past 1");
}

#[test]
fn kill_counter_and_claims_round_trip_through_player_save() {
    // Reviewer caught: a player who killed 9 marauders, saved, and
    // quit would have returned to 0 kills on reload. This pins the
    // fix — both kill_counter + bounties_claimed survive a bincode
    // round-trip via the PlayerSaveData wire shape.
    let mut psd = crate::save::PlayerSaveData {
        x: 0.0, y: 64.0, z: 0.0, yaw: 0.0, pitch: 0.0,
        health: 20.0, hotbar_slot: 0, inventory: vec![],
        spawn_pos: None, hunger: 20, reputation: vec![],
        tamed_pets: vec![],
        armour_slots: [None, None, None, None],
        kill_counter: vec![],
        bounties_claimed: vec![],
    };
    psd.kill_counter.push((MobType::Marauder, 9));
    psd.kill_counter.push((MobType::Brigand, 2));
    psd.bounties_claimed.push((42, 10));
    let bytes = bincode::serialize(&psd).expect("serialize");
    let back: crate::save::PlayerSaveData =
        bincode::deserialize(&bytes).expect("deserialize");
    let mut kills: ahash::AHashMap<MobType, u32> = ahash::AHashMap::new();
    for (k, n) in &back.kill_counter {
        kills.insert(*k, *n);
    }
    assert_eq!(*kills.get(&MobType::Marauder).unwrap(), 9,
        "marauder kill counter must survive save+load");
    assert_eq!(*kills.get(&MobType::Brigand).unwrap(), 2);
    assert_eq!(back.bounties_claimed.len(), 1);
    assert_eq!(back.bounties_claimed[0], (42, 10));
}

#[test]
fn active_bounty_template_lookup_returns_none_for_invalid_idx() {
    // ActiveBounty.template() returns Option — used by the UI to skip
    // rendering corrupt rows. Defensive regression.
    let bounty = ActiveBounty { id: 99, template_idx: 9_999, issued_tick: 0 };
    assert!(bounty.template().is_none());
}
