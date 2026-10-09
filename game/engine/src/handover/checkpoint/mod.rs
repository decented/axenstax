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
