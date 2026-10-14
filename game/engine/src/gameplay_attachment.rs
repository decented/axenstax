//! Common authority admission, before any external player receives gameplay.
use crate::progression::{GameplayBinding, WorldPlayer};

#[cfg(not(target_arch = "wasm32"))]
impl crate::GameState {
    /// The local world and seats are the save authority in a lending session.
    /// Failure leaves their gameplay state available and admits no new scope.
    pub(crate) fn prepare_local_gameplay(&mut self) -> Result<(), String> {
        if !self.persists_locally() {
            return Err("A joined session cannot create local world authority".into());
        }
        if self
            .players
            .first()
            .is_some_and(|slot| slot.personal_binding.is_some())
        {
            return self.preflight_personal_local();
        }
        if self
            .players
            .iter()
            .any(|slot| !slot.active_quests.is_empty() || !slot.villager_decline_until.is_empty())
        {
            return Err(
                "An existing quest cannot be converted without its exact original giver identity"
                    .into(),
            );
        }
        if self.players.is_empty() {
            return Err("missing local gameplay seat".into());
        }
        let old_scope = self.world.quest_scope;
        let scope = match old_scope {
            Some(scope) => scope,
            None => crate::quest_npcs::random_id()?,
        };
        let player = self
            .handover_signed_in()
            .map_or(WorldPlayer::LegacyLocal(0), WorldPlayer::Npub);
        let binding = GameplayBinding::WorldLocal { scope, player };
        binding.validate()?;
        let old_local = self.local_personal.clone();
        self.world.quest_scope = Some(scope);
        let slot = self
            .players
            .first_mut()
            .ok_or("missing local gameplay seat")?;
        let old_binding = slot.personal_binding.replace(binding.clone());
        self.local_personal = Some(binding);
        if let Err(error) = self.save_live_world() {
            self.world.quest_scope = old_scope;
            self.players[0].personal_binding = old_binding;
            self.local_personal = old_local;
            return Err(error);
        }
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl crate::server::GameServer {
    pub(crate) fn prepare_signed_scope(&mut self) -> Result<(), String> {
        if self.world.quest_scope.is_some() {
            return Ok(());
        }
        if self.lent {
            return Err("The local host's gameplay scope has not been saved".into());
        }
        let scope = crate::quest_npcs::random_id()?;
        self.world.quest_scope = Some(scope);
        if let Err(error) = self.try_save() {
            self.world.quest_scope = None;
            return Err(error);
        }
        Ok(())
    }
    pub(crate) fn attach_gameplay(
        &mut self,
        index: usize,
        session: [u8; 16],
        generation: u32,
        legacy: bool,
    ) -> Result<(), String> {
        if self.players[index].personal.is_some() {
            return Ok(());
        }
        let binding = match self.players[index].verified_pubkey {
            Some(key) => GameplayBinding::WorldLocal {
                scope: self
                    .world
                    .quest_scope
                    .ok_or("missing durable signed gameplay scope")?,
                player: WorldPlayer::Npub(key),
            },
            None => GameplayBinding::Guest {
                session,
                generation,
            },
        };
        let mut personal = crate::joiner_save::BoundPersonal::fresh(binding)?;
        if legacy {
            let mut record = personal.record()?;
            record.tracking = crate::progression::KillTracking::LegacyUntracked;
            personal.bytes = record.encode()?;
        }
        self.players[index].personal = Some(personal);
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn selected_world_local_present(directory: &std::path::Path) -> Result<bool, String> {
    let path = directory.join("world.dat");
    let bytes = match std::fs::File::open(path) {
        Ok(file) => {
            crate::save::read_bounded(file, crate::handover::checkpoint::WORLD_DATA_BYTES as u64)?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    let save = crate::save::read_world_save(&bytes).map_err(|e| e.to_string())?;
    let Some(local) = save.local_personal else {
        return Ok(false);
    };
    let record = crate::joiner_save::JoinerSave::decode(&local.0)?;
    Ok(record
        .personal
        .is_some_and(|bound| matches!(bound.binding, GameplayBinding::WorldLocal { .. })))
}

/// The selected snapshot, never a fresher remote mirror, owns a local restore.
pub(crate) fn selected_world_local(
    snapshot: &crate::personal_value::LocalPersonalSave,
    scope: Option<crate::progression::GameplayId>,
    signer: Option<[u8; 32]>,
    seat: Option<u8>,
) -> Result<crate::joiner_save::JoinerSave, String> {
    let saved = crate::joiner_save::JoinerSave::decode(&snapshot.0)?;
    let binding = &saved
        .personal
        .as_ref()
        .ok_or("selected personal snapshot has no authority")?
        .binding;
    match binding {
        GameplayBinding::WorldLocal {
            scope: expected,
            player,
        } if Some(*expected) == scope => {
            if match player {
                WorldPlayer::Npub(key) => Some(*key) != signer,
                WorldPlayer::LegacyLocal(local) => Some(*local) != seat,
            } {
                return Err("Sign in as this world's saved player before opening it.".into());
            }
            Ok(saved)
        }
        _ => Err("Selected local gameplay authority differs from its world scope".into()),
    }
}
