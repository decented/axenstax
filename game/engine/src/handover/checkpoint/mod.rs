//! Native, inactive checkpoint I/O. Supplied snapshots must already be frozen,
//! personally attributed and sanitised by the eventual live controller.
//! Nothing here opens a world, listens for gameplay or changes source authority.
mod admission;
mod bounded_decode;
mod evidence;
mod files;
mod import;
mod pack;
mod receipt;
use super::{Error, Result};
use crate::save::{WorldMeta, WorldSave};
pub use admission::{AdmittedManifest, OfferContext, sign_offer};
pub use evidence::{OfflineRecovery, RecoveredCheckpoint};
pub use import::{DurableCheckpoint, Import, ImportBoundary, ImportObserver, NoFault};
pub use pack::{
    CheckpointSource, PackBinding, PackedCheckpoint, ParticipantMember, pack, pack_limited,
};

/// A transfer COPY, preserving world history but carrying no source operator
/// credentials, publication endpoint or Stash opt-in. Original is unchanged.
pub fn transfer_metadata(original: &WorldMeta) -> Result<WorldMeta> {
    let mut count = files::Count {
        inner: Vec::new(),
        bytes: 0,
        limit: 2 * 1024 * 1024,
    };
    serde_json::to_writer(&mut count, original).map_err(|_| Error::Limit("metadata copy"))?;
    if count.inner.capacity() > 2 * 1024 * 1024 {
        return Err(Error::Limit("metadata copy allocation"));
    }
    let mut copy = bounded_decode::metadata(&count.inner)?;
    copy.pop_secret = None;
    copy.owner_pubkey = None;
    copy.published_to = None;
    copy.cloud_save = false;
    Ok(copy)
}
fn validate_metadata(meta: &WorldMeta) -> Result<()> {
    if meta.pop_secret.is_some()
        || meta.owner_pubkey.is_some()
        || meta.published_to.is_some()
        || meta.cloud_save
    {
        return Err(Error::Invalid("source operator metadata"));
    }
    Ok(())
}
fn validate_neutral(save: &WorldSave) -> Result<()> {
    if !save.players.is_empty()
        || !save.inventory.is_empty()
        || !save.locked_slots.is_empty()
        || save.player_x.to_bits() != 0
        || save.player_y.to_bits() != 0
        || save.player_z.to_bits() != 0
        || save.player_health != 20.0
        || save.hotbar_slot != 0
    {
        return Err(Error::Invalid("unattributed personal save"));
    }
    if !save.waypoints.is_empty() || save.satoshi != Default::default() {
        return Err(Error::Invalid("source-local player state"));
    }
    // Anything still owned or kept by seat number would land on whoever
    // becomes seat 0 on the recipient: the session maps proven seat 0 to
    // its npub first, so a remaining seat owner means it was never mapped.
    use crate::save::SavedTamedPetData as Pet;
    let seat = |o: &crate::tameable::OwnershipData| o.owner_pubkey.starts_with("local-player-");
    let unmapped = save.vendors.iter().any(|v| matches!(v.data.owner, Some(crate::vendor::VendorOwner::LocalPlayer(_))))
        || save.tip_jars.iter().any(|t| matches!(t.data.owner, Some(crate::tip_jar::TipJarOwner::LocalPlayer(_))))
        || save.auctions.iter().any(|a| {
            a.data.high_bidder.is_some() || matches!(a.data.owner, crate::auction::AuctionOwner::LocalPlayer(_))
        })
        || save.plots.iter().any(|p| matches!(p.owner, crate::plot::PlotOwner::LocalPlayer(_)))
        || save.market_hubs.iter().any(|h| matches!(h.owner, crate::market_hub::HubOwner::LocalPlayer(_)))
        || !save.raid_kills.is_empty()
        || save.active_raids.iter().any(|r| !r.contribution_table.is_empty() || r.killing_blow.is_some())
        || save.saved_mobs.iter().any(|m| match &m.data {
            Pet::Wolf(d) => seat(&d.ownership),
            Pet::Nostrich(d) => seat(&d.ownership),
            Pet::Companion { data, .. } | Pet::Companion2 { data, .. } => seat(&data.ownership),
            Pet::Steed { data, .. } | Pet::Steed2 { data, .. } => data.kept_by.is_some(),
            Pet::Authored { .. } => false,
        });
    if unmapped {
        return Err(Error::Invalid("seat-numbered ownership"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;

/// Small caller limits exercise the same real paths; hard protocol caps cannot
/// be raised. Logical bytes exclude separately bounded tar headers/padding.
#[derive(Clone, Copy)]
pub struct Limits {
    pub compressed: u64,
    pub logical: u64,
    pub members: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            compressed: super::manifest::COMPRESSED_LIMIT,
            logical: super::manifest::DECOMPRESSED_LIMIT,
            members: 65536,
        }
    }
}
impl Limits {
    fn validate(self) -> Result<()> {
        let max = Self::default();
        if self.compressed == 0
            || self.compressed > max.compressed
            || self.logical == 0
            || self.logical > max.logical
            || self.members == 0
            || self.members > max.members
        {
            return Err(Error::Limit("checkpoint limits"));
        }
        Ok(())
    }
}
