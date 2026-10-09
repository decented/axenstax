//! Eligibility: concrete refusals, before the source freezes and again at the
//! frozen capture, for state this build cannot carry faithfully. Nothing is
//! omitted, reset, settled or re-attributed to make a transfer pass; the
//! original world stays as it is and the reason names what blocked it.
use crate::save::{SavedTamedPetData, WorldSave};
use crate::server::GameServer;
use crate::tameable::OwnershipData;

/// A user-facing reason (UK English, npub-free).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal(pub &'static str);

/// The recipient's exact authenticated seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seat {
    pub slot: usize,
    pub generation: u32,
    pub key: [u8; 32],
}

/// Split-screen seats share one sign-in: none of them but the first has a
/// proven identity, so their records cannot be attributed.
pub fn local_seats(count: usize) -> Result<(), Refusal> {
    if count != 1 {
        return Err(Refusal("Split-screen players can't be handed over yet. Play with one local player."));
    }
    Ok(())
}

/// The signed-in identity now must be the one the host action captured.
pub fn source_identity(captured: [u8; 32], now: Option<[u8; 32]>) -> Result<(), Refusal> {
    match now {
        Some(k) if k == captured => Ok(()),
        Some(_) => Err(Refusal("You signed in as someone else since you started. Start the handover again.")),
        None => Err(Refusal("Sign in before handing this world over.")),
    }
}

/// Live seats and server-wide deliveries.
pub fn seats(server: &GameServer, recipient: Seat) -> Result<(), Refusal> {
    if !server.pending_item_grants.is_empty()
        || !server.pending_kill_events.is_empty()
        || !server.pending_bred_events.is_empty()
    {
        return Err(Refusal("Items or events are still on their way to players. Try again in a moment."));
    }
    for (i, sp) in server.players.iter().enumerate() {
        if !sp.server_simulated || !sp.is_in_world() {
            continue;
        }
        let Some(key) = sp.verified_pubkey else {
            continue; // guests persist nothing and carry no record
        };
        if server.failed_joiner_reads.contains(&key) {
            return Err(Refusal("A player's saved record here couldn't be read. It is kept untouched; the world can't be handed over until it is repaired."));
        }
        if i != recipient.slot {
            return Err(Refusal("Another signed-in player is here. Moving their saved record needs their agreement, which this version can't ask for yet."));
        }
    }
    let sp = server
        .players
        .get(recipient.slot)
        .filter(|sp| sp.server_simulated && sp.is_in_world())
        .ok_or(Refusal("That player is no longer in this world."))?;
    if sp.attach_gen != recipient.generation || sp.verified_pubkey != Some(recipient.key) {
        return Err(Refusal("That player reconnected. Choose them again."));
    }
    if !sp.progression_tracked || sp.progression_credits != 0 {
        return Err(Refusal("That player has village or hunting progress this session, which this version can't move yet."));
    }
    if sp.combat.dead {
        return Err(Refusal("That player is dead. Wait until they respawn."));
    }
    if !sp.window_events.is_quiescent() {
        return Err(Refusal("That player's inventory is still settling with yours. Try again in a moment."));
    }
    Ok(())
}

/// Saved participant records on disk (`joiners/<npub>.dat`) other than the
/// source's and the recipient's own: each needs that player's agreement.
pub fn saved_records(saved: &[[u8; 32]], source: [u8; 32], recipient: [u8; 32]) -> Result<(), Refusal> {
    if saved.iter().any(|k| *k != source && *k != recipient) {
        return Err(Refusal("Other players have saved records in this world. Moving them needs their agreement, which this version can't ask for yet."));
    }
    Ok(())
}

fn local_owner_index(s: &str) -> Option<&str> {
    s.strip_prefix("local-player-")
}

fn map_pet_owner(o: &mut OwnershipData, source_npub: &str) -> Result<(), Refusal> {
    match local_owner_index(&o.owner_pubkey) {
        None => Ok(()),
        Some("0") => {
            o.owner_pubkey = source_npub.to_string();
            Ok(())
        }
        Some(_) => Err(Refusal("A pet belongs to a split-screen player, who has no proven identity to carry it.")),
    }
}

/// On the TRANSFER COPY only: proven source local seat 0 ownership becomes
/// the source's npub (already-npub owners stay); anything slot-numbered that
/// cannot be attributed refuses. Then the copy is stripped of positional
/// player rows (their state travels in the source's own records).
pub fn neutralise(save: &mut WorldSave, source_npub: &str) -> Result<(), Refusal> {
    macro_rules! owner {
        ($o:expr, $ty:path) => {{
            use $ty as O;
            match $o {
                O::LocalPlayer(0) => *$o = O::Npub(source_npub.to_string()),
                O::LocalPlayer(_) => {
                    return Err(Refusal("Something here belongs to a split-screen player, who has no proven identity to carry it."))
                }
                O::Npub(_) => {}
            }
        }};
    }
    for v in &mut save.vendors {
        if let Some(o) = &mut v.data.owner {
            owner!(o, crate::vendor::VendorOwner);
        }
    }
    for t in &mut save.tip_jars {
        if let Some(o) = &mut t.data.owner {
            owner!(o, crate::tip_jar::TipJarOwner);
        }
    }
    for a in &mut save.auctions {
        if a.data.high_bidder.is_some() {
            return Err(Refusal("An auction has a bid this version can't attribute to a person. Finish or cancel it first."));
        }
        owner!(&mut a.data.owner, crate::auction::AuctionOwner);
    }
    for p in &mut save.plots {
        owner!(&mut p.owner, crate::plot::PlotOwner);
    }
    for h in &mut save.market_hubs {
        owner!(&mut h.owner, crate::market_hub::HubOwner);
    }
    if !save.raid_kills.is_empty()
        || save
            .active_raids
            .iter()
            .any(|r| !r.contribution_table.is_empty() || r.killing_blow.is_some())
    {
        return Err(Refusal("Raid records here are kept by seat, not by person, so they can't move yet."));
    }
    for m in &mut save.saved_mobs {
        match &mut m.data {
            SavedTamedPetData::Wolf(d) => map_pet_owner(&mut d.ownership, source_npub)?,
            SavedTamedPetData::Nostrich(d) => map_pet_owner(&mut d.ownership, source_npub)?,
            SavedTamedPetData::Companion { data, .. } | SavedTamedPetData::Companion2 { data, .. } => {
                map_pet_owner(&mut data.ownership, source_npub)?
            }
            SavedTamedPetData::Steed { data, .. } | SavedTamedPetData::Steed2 { data, .. } => {
                if data.kept_by.is_some() {
                    return Err(Refusal("A kept horse is held by seat number, which this version can't move yet."));
                }
            }
            SavedTamedPetData::Authored { .. } => {}
        }
    }
    if save.players.iter().any(|p| !p.tamed_pets.is_empty()) {
        return Err(Refusal("This world has old-style pet records this version can't move yet."));
    }
    save.players.clear();
    save.inventory.clear();
    save.locked_slots.clear();
    save.player_x = 0.0;
    save.player_y = 0.0;
    save.player_z = 0.0;
    save.player_health = 20.0;
    save.hotbar_slot = 0;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auction::{AuctionData, AuctionOwner};
    use crate::save::{SavedAuction, SavedTipJar, SavedVendor};
    use crate::tip_jar::{TipJarData, TipJarOwner};
    use crate::vendor::{VendorData, VendorOwner};

    const SOURCE: &str = "npub1source";

    fn live_copy() -> WorldSave {
        let slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::new(4.0, 70.0, 4.0), 1.0);
        crate::save::build_world_save(&crate::world::World::new(), &[slot], 42, &[], &[])
    }
    fn vendor(owner: VendorOwner) -> SavedVendor {
        SavedVendor { x: 1, y: 2, z: 3, data: VendorData { owner: Some(owner), ..Default::default() } }
    }

    #[test]
    fn handover_neutralise_maps_proven_seat0_keeps_npub_and_strips_rows() {
        let mut s = live_copy();
        assert_eq!(s.players.len(), 1, "the builder wrote the seat row");
        s.vendors.push(vendor(VendorOwner::LocalPlayer(0)));
        s.vendors.push(vendor(VendorOwner::Npub("npub1other".into())));
        s.tip_jars.push(SavedTipJar {
            x: 0,
            y: 0,
            z: 0,
            data: TipJarData { owner: Some(TipJarOwner::LocalPlayer(0)), ..Default::default() },
        });
        s.auctions.push(SavedAuction { x: 0, y: 0, z: 0, data: AuctionData::new(AuctionOwner::LocalPlayer(0)) });
        neutralise(&mut s, SOURCE).unwrap();
        assert!(matches!(&s.vendors[0].data.owner, Some(VendorOwner::Npub(n)) if n == SOURCE));
        assert!(matches!(&s.vendors[1].data.owner, Some(VendorOwner::Npub(n)) if n == "npub1other"));
        assert!(matches!(&s.tip_jars[0].data.owner, Some(TipJarOwner::Npub(n)) if n == SOURCE));
        assert!(matches!(&s.auctions[0].data.owner, AuctionOwner::Npub(n) if n == SOURCE));
        assert!(s.players.is_empty() && s.inventory.is_empty() && s.locked_slots.is_empty());
        assert_eq!((s.player_x, s.player_health, s.hotbar_slot), (0.0, 20.0, 0));
    }

    #[test]
    fn handover_neutralise_refuses_unattributable_slot_state() {
        let cases: [fn(&mut WorldSave); 4] = [
            |s| s.vendors.push(vendor(VendorOwner::LocalPlayer(1))),
            |s| {
                let mut a = AuctionData::new(AuctionOwner::Npub("npub1x".into()));
                a.high_bidder = Some(0);
                s.auctions.push(SavedAuction { x: 0, y: 0, z: 0, data: a });
            },
            |s| s.raid_kills.push(((0, 0), 0, 0)),
            |s| {
                let pet = crate::save::SavedTamedPet {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    data: SavedTamedPetData::Authored { kind: crate::mob::MobType::Cow },
                };
                s.players[0].tamed_pets.push(pet);
            },
        ];
        for (i, case) in cases.iter().enumerate() {
            let mut s = live_copy();
            case(&mut s);
            assert!(neutralise(&mut s, SOURCE).is_err(), "case {i} must refuse");
            assert_eq!(s.players.len(), 1, "case {i}: a refusal leaves the rows to the original");
        }
    }

    #[test]
    fn handover_identity_seat_count_and_saved_record_rules() {
        assert!(local_seats(1).is_ok());
        assert!(local_seats(2).is_err());
        assert!(source_identity([1; 32], Some([1; 32])).is_ok());
        assert!(source_identity([1; 32], Some([2; 32])).is_err());
        assert!(source_identity([1; 32], None).is_err());
        assert!(saved_records(&[[1; 32], [2; 32]], [1; 32], [2; 32]).is_ok());
        assert!(saved_records(&[[3; 32]], [1; 32], [2; 32]).is_err());
    }
}
