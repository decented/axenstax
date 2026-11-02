//! `GameServer::restore_from_checkpoint`: verify, load the records through the
//! existing load path, verify them against the manifest, apply the volatile
//! sections. Nothing reaches the caller unless all of it holds, and a refusal
//! changes nothing on disk beyond what an ordinary load does.

use super::codec::{Malformed, Reader};
use super::{volatile, AuthorityConfig, Header, RestoreError, HEADER_BYTES, MAX_CHECKPOINT_BYTES, SEAL_BYTES};
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
        if checkpoint.len() > MAX_CHECKPOINT_BYTES {
            return Err(RestoreError::InvalidCheckpoint(format!("{} bytes is over the cap", checkpoint.len())));
        }
        // The schema first: a newer checkpoint's seal and layout are not ours to read.
        let header = Header::read(checkpoint)?;
        header.check_versions()?;
        super::check_seal(checkpoint)?;
        header.check_config(config)?;

        let mut body = Reader::new(&checkpoint[HEADER_BYTES..checkpoint.len() - SEAL_BYTES]);
        let mut restored = volatile::read(&mut body)?;
        if restored.tick_counter() != header.tick {
            return Err(RestoreError::InvalidCheckpoint("the header and the clock name different ticks".into()));
        }
        let manifest = super::manifest::read(&mut body)?;
        if !body.is_empty() {
            return Err(RestoreError::InvalidCheckpoint("bytes left over after the manifest".into()));
        }

        let name = world_name(world_dir)?;
        if crate::world_open::is_new_world(&name).map_err(RestoreError::Fault)? {
            return Err(RestoreError::InvalidCheckpoint(format!("world '{name}' holds no saved records")));
        }
        let seed = u32::try_from(config.seed).map_err(|_| RestoreError::InvalidConfig("a seed past u32".into()))?;
        let mut server = GameServer::new(0, name, seed);
        // The load generates what is missing round its centre; the layout below
        // discards it, so it may as well be one column.
        server.render_distance = 0;
        server.initial_load().map_err(RestoreError::Fault)?;
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
        restored.apply(&mut server);
        reload_sidecars(&mut server);

        // Every field this module carries is back if the body comes out the
        // same (the header's version numbers may be an older build's). A
        // difference means the input wasn't in canonical form: a non-zero
        // padding of a float's sign, say, that a seal can't see.
        let again = server.checkpoint().map_err(|e| RestoreError::Fault(format!("{e:?}")))?;
        let body_of = |bytes: &[u8]| bytes[HEADER_BYTES..bytes.len() - SEAL_BYTES].to_vec();
        if body_of(&again) != body_of(checkpoint) {
            return Err(RestoreError::InvalidCheckpoint("the bytes aren't this module's canonical form".into()));
        }
        Ok(server)
    }
}

/// What a joiner's sidecar holds and the checkpoint doesn't: its bound personal
/// record and its obligation history, or its next save would write them away.
fn reload_sidecars(server: &mut GameServer) {
    for i in 0..server.players.len() {
        let sp = &server.players[i];
        if !sp.server_simulated || !sp.is_in_world() || sp.verified_pubkey.is_none() {
            continue;
        }
        if let Some(saved) = server.load_joiner(i) {
            saved.restore_session(&mut server.players[i]);
        }
        let now = server.tick_counter;
        server.players[i].gameplay_clock.restore(now);
    }
}
