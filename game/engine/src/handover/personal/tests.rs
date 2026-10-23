//! Strict participant-record codec: canonical, bounded, never truncated.
use super::*;
use crate::handover::Error;

fn ctx() -> RecordContext {
    RecordContext {
        participant: [7; 32],
        lineage: [1; 16],
        generation: 9,
        transfer: [4; 16],
    }
}
fn sample() -> PersonalRecord {
    let rep = [((3, -4), -60i16), ((-2, 5), 100)];
    let kills = [(MobType::Wolf, 12u32), (MobType::Cow, 3)];
    let claims = [(77u32, 5u32), (2, 1)];
    PersonalRecord::from_maps(
        ctx(),
        rep.iter().map(|(k, v)| (k, v)),
        kills.iter().map(|(k, v)| (k, v)),
        claims.iter().map(|(k, v)| (k, v)),
        Some(LocalRestore {
            pos: [1.5, 70.0, -3.25],
            yaw: 0.5,
            pitch: -0.25,
            hotbar: 8,
            locks: (1 << 35) | 1,
        }),
    )
    .unwrap()
}

#[test]
fn handover_personal_roundtrip_is_canonical_and_exact() {
    let r = sample();
    assert_eq!(r.reputation[0].0, (-2, 5), "sorted, not insertion order");
    assert_eq!(r.claims, vec![(2, 1), (77, 5)]);
    let bytes = r.encode().unwrap();
    let back = PersonalRecord::decode(&bytes, &ctx()).unwrap();
    assert_eq!(back, r);
    assert_eq!(back.encode().unwrap(), bytes, "one canonical encoding");
    assert!(!r.is_default());
    let empty = PersonalRecord::from_maps(ctx(), [], [], [], None).unwrap();
    assert!(empty.is_default());
    let zeros = [((1, 1), 0i16)];
    let z = PersonalRecord::from_maps(ctx(), zeros.iter().map(|(k, v)| (k, v)), [], [], None);
    assert_eq!(z.unwrap().reputation.len(), 1, "zero entries kept, not normalised away");
}

#[test]
fn handover_personal_foreign_context_truncation_trailing_and_version_refused() {
    let bytes = sample().encode().unwrap();
    for other in [
        RecordContext { participant: [8; 32], ..ctx() },
        RecordContext { lineage: [2; 16], ..ctx() },
        RecordContext { generation: 10, ..ctx() },
        RecordContext { transfer: [5; 16], ..ctx() },
    ] {
        assert_eq!(
            PersonalRecord::decode(&bytes, &other),
            Err(Error::Invalid("participant context"))
        );
    }
    for i in 0..bytes.len() {
        assert!(PersonalRecord::decode(&bytes[..i], &ctx()).is_err(), "prefix {i}");
    }
    let mut long = bytes.clone();
    long.push(0);
    assert_eq!(
        PersonalRecord::decode(&long, &ctx()),
        Err(Error::Invalid("trailing bytes"))
    );
    // Schema follows the 4-byte length + domain label.
    let at = 4 + DOMAIN.len();
    let mut unknown = bytes.clone();
    unknown[at + 3] = 3;
    assert_eq!(
        PersonalRecord::decode(&unknown, &ctx()),
        Err(Error::Invalid("participant schema"))
    );
}

/// Offsets inside the payload after the fixed header.
fn header_len() -> usize {
    4 + DOMAIN.len() + 4 + 32 + 16 + 8 + 16
}

#[test]
fn handover_personal_duplicates_range_unknown_kind_and_flags_refused() {
    let r = sample();
    let bytes = r.encode().unwrap();
    let h = header_len();
    // Duplicate reputation key: copy the first key over the second.
    let mut dup = bytes.clone();
    let first = dup[h + 4..h + 12].to_vec();
    dup[h + 14..h + 22].copy_from_slice(&first);
    assert_eq!(
        PersonalRecord::decode(&dup, &ctx()),
        Err(Error::Invalid("participant duplicate key"))
    );
    // Score 101 is outside the reputation range.
    let mut hi = bytes.clone();
    hi[h + 12..h + 14].copy_from_slice(&101i16.to_be_bytes());
    assert_eq!(
        PersonalRecord::decode(&hi, &ctx()),
        Err(Error::Invalid("participant reputation score"))
    );
    // Unknown mob discriminant.
    let k = h + 4 + 2 * 10;
    let mut mob = bytes.clone();
    mob[k + 4..k + 8].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(
        PersonalRecord::decode(&mob, &ctx()),
        Err(Error::Invalid("unknown mob kind"))
    );
    // Unknown flags byte.
    let f = k + 4 + 2 * 8 + 4 + 2 * 8;
    let mut flags = bytes.clone();
    flags[f] = 2;
    assert_eq!(
        PersonalRecord::decode(&flags, &ctx()),
        Err(Error::Invalid("participant flags"))
    );
    // Non-finite pose, hotbar 9 and lock bit 36 are refused at encode.
    for bad in [
        LocalRestore { pos: [f32::NAN, 0.0, 0.0], yaw: 0.0, pitch: 0.0, hotbar: 0, locks: 0 },
        LocalRestore { pos: [0.0; 3], yaw: f32::INFINITY, pitch: 0.0, hotbar: 0, locks: 0 },
        LocalRestore { pos: [0.0; 3], yaw: 0.0, pitch: 0.0, hotbar: 9, locks: 0 },
        LocalRestore { pos: [0.0; 3], yaw: 0.0, pitch: 0.0, hotbar: 0, locks: 1 << 36 },
    ] {
        let rec = PersonalRecord { local: Some(bad), ..sample() };
        assert_eq!(rec.encode(), Err(Error::Invalid("participant local restore")));
    }
}

#[test]
fn handover_personal_caps_refuse_before_allocation_and_never_truncate() {
    // A count beyond the cap is refused straight after it is read.
    let mut v = sample().encode().unwrap();
    let h = header_len();
    v[h..h + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(
        PersonalRecord::decode(&v, &ctx()),
        Err(Error::Limit("participant reputation entries"))
    );
    let rep: Vec<((i32, i32), i16)> = (0..=REPUTATION_LIMIT as i32).map(|i| ((i, 0), 1)).collect();
    assert_eq!(
        PersonalRecord::from_maps(ctx(), rep.iter().map(|(k, v)| (k, v)), [], [], None),
        Err(Error::Limit("participant reputation entries"))
    );
    let claims: Vec<(u32, u32)> = (0..=CLAIMS_LIMIT as u32).map(|i| (i, 1)).collect();
    assert_eq!(
        PersonalRecord::from_maps(ctx(), [], [], claims.iter().map(|(k, v)| (k, v)), None),
        Err(Error::Limit("participant bounty claims"))
    );
    // The largest legal record still fits the member cap.
    let rep: Vec<((i32, i32), i16)> = (0..REPUTATION_LIMIT as i32).map(|i| ((i, 0), -100)).collect();
    let claims: Vec<(u32, u32)> = (0..CLAIMS_LIMIT as u32).map(|i| (i, u32::MAX)).collect();
    let max = PersonalRecord::from_maps(
        ctx(),
        rep.iter().map(|(k, v)| (k, v)),
        [],
        claims.iter().map(|(k, v)| (k, v)),
        sample().local,
    )
    .unwrap();
    let bytes = max.encode().unwrap();
    assert!(bytes.len() <= RECORD_LIMIT, "{}", bytes.len());
    assert_eq!(PersonalRecord::decode(&bytes, &ctx()).unwrap(), max);
}

#[test]
fn handover_personal_install_replaces_only_progression_and_runs_no_reward() {
    let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
    slot.kill_counter.insert(MobType::Pig, 99);
    slot.bounties_claimed.insert(500, 3);
    slot.reputation.per_village.insert((9, 9), 50);
    let before_inventory = slot.inventory.locked_indices();
    let r = sample();
    r.install_progression(&mut slot);
    assert_eq!(slot.kill_counter.len(), 2);
    assert_eq!(slot.kill_counter[&MobType::Wolf], 12);
    assert!(!slot.kill_counter.contains_key(&MobType::Pig));
    assert_eq!(slot.bounties_claimed.len(), 2);
    assert_eq!(slot.reputation.score((3, -4)), -60);
    assert_eq!(slot.reputation.score((9, 9)), 0);
    assert_eq!(slot.inventory.locked_indices(), before_inventory);
}
