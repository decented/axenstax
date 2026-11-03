//! `GameServer::restore_from_checkpoint`, in the contract's order: the schema
//! field, the seal, the remaining versions, the config binding, then the digest of
//! every bound record as it lies on disk. Only then does the load run, so a record
//! that doesn't match is refused before anything has parsed it. The load writes
//! nothing: the recoveries of a normal open don't run (`world_open::open_bound_world`).
//! Then the layout, the volatile sections and a last check that it all encodes back
//! to the same bytes.

use super::codec::{Malformed, Reader};
use super::manifest::Entry;
use super::volatile::Volatile;
use super::{volatile, AuthorityConfig, Header, RestoreError, AFTER_VERSIONS, HEADER_BYTES, MAX_CHECKPOINT_BYTES, SEAL_BYTES};
use crate::server::GameServer;

impl From<Malformed> for RestoreError {
    fn from(why: Malformed) -> Self {
        RestoreError::InvalidCheckpoint(why.0.to_string())
    }
}

/// The world folder's name, as the load path addresses a world by name under
/// the worlds root. A folder elsewhere is the caller's mistake, not a bad
/// checkpoint.
fn world_name(world_dir: &std::path::Path) -> Result<String, RestoreError> {
    let name = world_dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| RestoreError::Fault(format!("{} names no world", world_dir.display())))?;
    if crate::save::world_dir(name) != world_dir {
        return Err(RestoreError::Fault(format!("{} is outside the worlds root", world_dir.display())));
    }
    Ok(name.to_string())
}

/// Everything between the header and the seal, read and range-checked. The one
/// reader of these bytes, used by `restore` and by `checkpoint` on its own output.
pub(super) fn decode_body(body: &[u8], tick: u64) -> Result<(Volatile, Vec<Entry>), Malformed> {
    let mut body = Reader::new(body);
    let restored = volatile::read(&mut body)?;
    restored.validate()?;
    if restored.tick_counter() != tick {
        return Err(Malformed("the header and the clock name different ticks"));
    }
    let manifest = super::manifest::read(&mut body)?;
    if !body.is_empty() {
        return Err(Malformed("bytes left over after the manifest"));
    }
    Ok((restored, manifest))
}

impl GameServer {
    /// Restore an authority from `checkpoint` and the records in `world_dir`,
    /// which must be the ones written by the save just before it. The errors
    /// map onto the contract's codes (`RestoreError`). Not wired into
    /// `--keep-world` yet: this is the API.
    ///
    /// What comes back holds the restored state and every other digest section
    /// as a fresh load left it (Spec 01 §4.1.5 has the ledger). A joiner in it
    /// has a body and no connection state: it must re-attach before it acts.
    pub(crate) fn restore_from_checkpoint(
        config: &AuthorityConfig,
        checkpoint: &[u8],
        world_dir: &std::path::Path,
    ) -> Result<Self, RestoreError> {
        super::check_schema(checkpoint)?;
        if checkpoint.len() > MAX_CHECKPOINT_BYTES {
            return Err(RestoreError::InvalidCheckpoint(format!("{} bytes is over the cap", checkpoint.len())));
        }
        let header = Header::read(checkpoint)?;
        // The seal before the versions, so that damage to a version field reads
        // as damage.
        super::check_seal(checkpoint)?;
        header.check_versions()?;
        header.check_config(config)?;

        let (mut restored, manifest) = decode_body(&checkpoint[HEADER_BYTES..checkpoint.len() - SEAL_BYTES], header.tick)?;

        let name = world_name(world_dir)?;
        if crate::world_open::is_new_world(&name).map_err(RestoreError::Fault)? {
            return Err(RestoreError::InvalidCheckpoint(format!("world '{name}' holds no saved records")));
        }
        if let Some(why) = super::manifest::stored_mismatch(&manifest, world_dir).map_err(RestoreError::Fault)? {
            return Err(RestoreError::InvalidCheckpoint(format!("the stored records and the manifest disagree: {why}")));
        }
        refuse_a_bound_world_dat_that_isnt_the_state(world_dir, header.tick)?;
        refuse_a_pending_role_cut(world_dir)?;
        let seed = u32::try_from(config.seed).map_err(|_| RestoreError::InvalidConfig("a seed past u32".into()))?;
        let mut server = GameServer::new(0, name, seed);
        // The world holds what its records and layout say, so the load generates
        // nothing: a column generated here could add to a table the manifest binds,
        // and a good checkpoint would read as a mismatch.
        server.render_distance = 0;
        server.initial_load_restoring().map_err(RestoreError::Fault)?;
        if server.world.generator.version() != config.worldgen_version {
            return Err(RestoreError::InvalidConfig(format!(
                "the world's generator is v{}, the config's v{}",
                server.world.generator.version(),
                config.worldgen_version
            )));
        }
        // The shell's secret, which the header's key id just vouched for. The
        // play mode comes back with the volatile sections.
        server.pop_secret = config.pop_secret;
        server.difficulty = config.difficulty;

        restored.apply_layout(&mut server);
        let found = super::manifest::entries(&server).map_err(RestoreError::Fault)?;
        if let Some(why) = super::manifest::first_mismatch(&manifest, &found) {
            return Err(RestoreError::InvalidCheckpoint(format!("the stored records and the manifest disagree: {why}")));
        }
        let deaths = restored.take_deaths();
        restored.apply(&mut server);
        super::deaths::apply(&deaths, &mut server).map_err(RestoreError::InvalidCheckpoint)?;
        reload_sidecars(&mut server);

        // Every field this module carries is back, and the server is the one the
        // header binds, if all of it after the version numbers (which may be an
        // older build's) comes out the same. A difference means the input wasn't
        // in canonical form: a `-0.0`, say, which decodes and passes the seal, or
        // a play mode the binding doesn't name.
        let again = server.checkpoint().map_err(|e| RestoreError::Fault(format!("{e:?}")))?;
        let bound_part = |bytes: &[u8]| bytes[AFTER_VERSIONS..bytes.len() - SEAL_BYTES].to_vec();
        if bound_part(&again) != bound_part(checkpoint) {
            return Err(RestoreError::InvalidCheckpoint("the bytes aren't this module's canonical form".into()));
        }
        Ok(server)
    }
}

/// The bound `world.dat` matches its digest. It still isn't this checkpoint's state if
/// this build only part-decodes it (the rest would be defaults), or if it holds another
/// tick's clock: a checkpoint taken after its persist has digests that can all match
/// while the tables are stale. A save with no clock is older than the field, and says
/// nothing either way.
fn refuse_a_bound_world_dat_that_isnt_the_state(world_dir: &std::path::Path, tick: u64) -> Result<(), RestoreError> {
    let stored = std::fs::read(world_dir.join("world.dat"))
        .map_err(|e| RestoreError::Fault(format!("world.dat can't be read ({e})")))?;
    let (save, partial) = crate::save::read_world_save_reporting(&stored)
        .map_err(|e| RestoreError::InvalidCheckpoint(format!("the bound world.dat doesn't read: {e}")))?;
    if partial {
        return Err(RestoreError::InvalidCheckpoint("the bound world.dat only partly decodes".into()));
    }
    match save.clock() {
        Some(clock) if clock.tick_counter != tick => Err(RestoreError::InvalidCheckpoint(format!(
            "the stored records are of tick {}, and the checkpoint of tick {tick}",
            clock.tick_counter
        ))),
        _ => Ok(()),
    }
}

/// A server with no local players makes a handover's role cut at its first load,
/// so a checkpoint's folder has already made it. One that hasn't (the wrong folder,
/// a copy from before the cut) would make it inside `initial_load`, an irreversible
/// change of authority, before the manifest is read.
fn refuse_a_pending_role_cut(world_dir: &std::path::Path) -> Result<(), RestoreError> {
    use crate::handover::live::personal_adapter::{proof, role_cut_pending};
    if proof(world_dir).map_err(RestoreError::Fault)?.is_some() && role_cut_pending(world_dir).map_err(RestoreError::Fault)? {
        return Err(RestoreError::Fault(format!("{} hasn't made its role cut", world_dir.display())));
    }
    Ok(())
}

/// What a joiner's sidecar holds and the checkpoint doesn't: its bound personal
/// record and its obligation history. A joiner with a sidecar isn't saved again
/// until it re-attaches, since that file also holds what it owes.
fn reload_sidecars(server: &mut GameServer) {
    for i in 0..server.players.len() {
        let sp = &server.players[i];
        if !sp.server_simulated || !sp.is_in_world() || sp.verified_pubkey.is_none() {
            continue;
        }
        if let Some(saved) = server.load_joiner_unsettled(i) {
            saved.restore_session(&mut server.players[i]);
        }
        let now = server.tick_counter;
        server.players[i].gameplay_clock.restore(now);
    }
}
