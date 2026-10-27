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
        let existing = self
            .players
            .first()
            .and_then(|slot| slot.personal_binding.clone());
        if existing.is_some() && self.gameplay_preparation.is_none() {
            if let Some(GameplayBinding::WorldLocal { scope, .. }) = &existing
                && self.world.quest_scope != Some(*scope)
            {
                return Err("Saved local gameplay authority differs from its world scope".into());
            }
            if let Some(scope) = self.world.quest_scope {
                let directory = crate::save::world_dir(&self.world_name);
                let session = crate::gameplay_preparation::Session::Native(
                    self.gameplay_preparation_generation,
                );
                let preparation = crate::gameplay_preparation::ScopePreparation::new(
                    directory.clone(),
                    session,
                    scope,
                    existing.clone(),
                    Some(scope),
                    existing.as_ref(),
                );
                if let Err(error) = preparation.validate(&directory, session, existing.as_ref()) {
                    self.gameplay_preparation = Some(preparation);
                    self.session_saves.note_save(false);
                    return Err(error);
                }
                self.preflight_personal_local()?;
                crate::handover::live::personal_adapter::confirm_durable(
                    &directory.join("world.dat"),
                )?;
                self.gameplay_preparation_succeeded();
                return Ok(());
            }
        }
        if existing.is_none()
            && self.players.iter().any(|slot| {
                !slot.active_quests.is_empty()
                    || !slot.kill_quest_baseline.is_empty()
                    || slot
                        .villager_decline_until
                        .values()
                        .any(|until| *until > self.tick_counter)
            })
        {
            return Err(
                "An existing quest cannot be converted without its exact original giver identity"
                    .into(),
            );
        }
        if self.players.is_empty() {
            return Err("missing local gameplay seat".into());
        }
        let penalty = if existing.is_none() {
            match self.players[0].last_villager_kill_rep_tick {
                Some(last) => {
                    let elapsed = self
                        .tick_counter
                        .checked_sub(last)
                        .ok_or("The legacy villager penalty timestamp is ahead of gameplay time")?;
                    crate::reputation::VILLAGER_KILL_REP_COOLDOWN_TICKS.saturating_sub(elapsed)
                        as u32
                }
                None => 0,
            }
        } else {
            self.players[0].quest_progress.penalty_left
        };
        let old_scope = self.world.quest_scope;
        let scope = match self
            .gameplay_preparation
            .as_ref()
            .map(|pending| pending.scope)
            .or(old_scope)
        {
            Some(scope) => scope,
            None => crate::quest_npcs::random_id()?,
        };
        let player = self
            .handover_signed_in()
            .map_or(WorldPlayer::LegacyLocal(0), WorldPlayer::Npub);
        // A transferred player's proof remains its authority; the independent
        // world scope identifies givers and fresh attachments, not that proof.
        let first_binding = existing.is_none();
        let old_selected_binding = existing.clone();
        let binding = existing.unwrap_or(GameplayBinding::WorldLocal { scope, player });
        binding.validate()?;
        if binding
            .context()
            .participant()
            .is_some_and(|key| self.handover_signed_in() != Some(key))
        {
            return Err("Sign in as this world's saved player before preparing gameplay.".into());
        }
        let directory = crate::save::world_dir(&self.world_name);
        let session =
            crate::gameplay_preparation::Session::Native(self.gameplay_preparation_generation);
        let mut preparation = match &self.gameplay_preparation {
            Some(pending) => pending.clone(),
            None => crate::gameplay_preparation::ScopePreparation::new(
                directory.clone(),
                session,
                scope,
                Some(binding.clone()),
                old_scope,
                old_selected_binding.as_ref(),
            ),
        };
        if let Err(error) = preparation.validate(&directory, session, Some(&binding)) {
            self.gameplay_preparation = Some(preparation);
            self.session_saves.note_save(false);
            return Err(error);
        }
        let old_local = self.local_personal.clone();
        let old_saves = self.session_saves;
        let old_progress = self.players[0].quest_progress.clone();
        let old_clock = self.players[0].gameplay_clock;
        self.world.quest_scope = Some(scope);
        let slot = self
            .players
            .first_mut()
            .ok_or("missing local gameplay seat")?;
        let old_binding = slot.personal_binding.replace(binding.clone());
        if first_binding {
            slot.quest_progress.penalty_left = penalty;
            // The same holder must be anchored BEFORE the initial snapshot,
            // so its just-migrated remaining interval is captured unchanged.
            slot.gameplay_clock.restore(self.tick_counter);
        }
        self.local_personal = Some(binding);
        preparation.begin_write();
        self.gameplay_preparation = Some(preparation);
        let prepared = self.save_live_world().and_then(|()| {
            self.gameplay_preparation
                .as_ref()
                .expect("active preparation")
                .confirm()
        });
        if let Err(error) = prepared {
            self.world.quest_scope = old_scope;
            self.players[0].personal_binding = old_binding;
            self.players[0].quest_progress = old_progress;
            self.players[0].gameplay_clock = old_clock;
            self.local_personal = old_local;
            self.session_saves = old_saves;
            self.session_saves.note_save(false);
            if !self
                .gameplay_preparation
                .as_mut()
                .expect("active preparation")
                .failed()
            {
                self.gameplay_preparation = None;
            }
            return Err(error);
        }
        self.gameplay_preparation = None;
        self.session_saves.note_save(true);
        self.gameplay_preparation_succeeded();
        Ok(())
    }

    fn gameplay_preparation_succeeded(&mut self) {
        if let Some(host) = self.hosted_server.as_mut()
            && host.lends_host_world()
        {
            host.local_gameplay_preparation = Some(Ok(()));
            host.local_gameplay_waiting_deaths = false;
        }
    }
}

/// A dedicated role cut leaves canonical authority only when the selected
/// world snapshot has no local holder. Never use this to override a snapshot.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn returned_world_local(
    directory: &std::path::Path,
    scope: Option<crate::progression::GameplayId>,
    signer: Option<[u8; 32]>,
) -> Result<Option<crate::joiner_save::JoinerSave>, String> {
    use crate::handover::live::personal_adapter as adapter;
    let Some(key) = signer else {
        if adapter::has_any_bound_records(directory)? {
            return Err("Sign in as this world's saved player before opening it.".into());
        }
        return Ok(None);
    };
    let path = adapter::path(directory, key);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if adapter::has_any_bound_records(directory)? {
                return Err(
                    "This world's saved player authority is missing. Nothing was reset.".into(),
                );
            }
            return Ok(None);
        }
        Err(error) => return Err(error.to_string()),
        Ok(_) => (),
    }
    let bytes = adapter::read_regular(&path, crate::joiner_save::MAX_JOINER_SAVE_BYTES)?;
    let saved = crate::joiner_save::JoinerSave::decode(&bytes)?;
    let Some(personal) = &saved.personal else {
        if adapter::has_any_bound_records(directory)? {
            return Err(
                "This world's saved player authority is missing. Nothing was reset.".into(),
            );
        }
        return Ok(None);
    };
    if bytes.first() != Some(&4)
        || !matches!(personal.binding, GameplayBinding::WorldLocal { scope: expected, player: WorldPlayer::Npub(owner) } if Some(expected) == scope && owner == key)
    {
        return Err(
            "Current saved player authority differs from this world or sign-in. Nothing was reset."
                .into(),
        );
    }
    if !saved.obligations_proven_zero() {
        return Err("This player's inventory may still owe items; it cannot become a local seat. Nothing was changed.".into());
    }
    personal.record()?;
    adapter::confirm_durable(&path)?;
    Ok(Some(saved))
}

#[cfg(test)]
#[path = "gameplay_attachment_tests.rs"]
mod tests;

#[cfg(not(target_arch = "wasm32"))]
impl crate::server::GameServer {
    pub(crate) fn prepare_signed_scope(&mut self) -> Result<(), String> {
        self.prepare_signed_scope_with_native(false)
    }

    pub(crate) fn prepare_signed_scope_with_native(
        &mut self,
        native_prepared: bool,
    ) -> Result<(), String> {
        if self.lent {
            self.refuse_pending_gameplay_cut()?;
            let scope = self
                .world
                .quest_scope
                .ok_or("The local host's gameplay scope has not been saved")?;
            return crate::gameplay_preparation::confirm_lent_scope(
                &crate::save::world_dir(&self.world_name),
                scope,
                native_prepared,
            );
        }
        if let Some(scope) = self.world.quest_scope
            && self.gameplay_preparation.is_none()
        {
            let directory = crate::save::world_dir(&self.world_name);
            let session = crate::gameplay_preparation::Session::Server(self.run_id);
            let selected = self
                .selected_local_personal
                .as_ref()
                .map(|local| crate::joiner_save::JoinerSave::decode(&local.0))
                .transpose()?;
            let binding = selected
                .as_ref()
                .and_then(|saved| saved.personal.as_ref())
                .map(|personal| &personal.binding);
            let preparation = crate::gameplay_preparation::ScopePreparation::new(
                directory.clone(),
                session,
                scope,
                None,
                Some(scope),
                binding,
            );
            if let Err(error) = preparation.validate(&directory, session, None) {
                self.gameplay_preparation = Some(preparation);
                return Err(error);
            }
            crate::handover::live::personal_adapter::confirm_durable(&directory.join("world.dat"))?;
            return Ok(());
        }
        let directory = crate::save::world_dir(&self.world_name);
        let session = crate::gameplay_preparation::Session::Server(self.run_id);
        let mut preparation = match &self.gameplay_preparation {
            Some(pending) => pending.clone(),
            None => {
                let selected = self
                    .selected_local_personal
                    .as_ref()
                    .map(|local| crate::joiner_save::JoinerSave::decode(&local.0))
                    .transpose()?;
                let binding = selected
                    .as_ref()
                    .and_then(|saved| saved.personal.as_ref())
                    .map(|personal| &personal.binding);
                crate::gameplay_preparation::ScopePreparation::new(
                    directory.clone(),
                    session,
                    crate::quest_npcs::random_id()?,
                    None,
                    self.world.quest_scope,
                    binding,
                )
            }
        };
        if let Err(error) = preparation.validate(&directory, session, None) {
            self.gameplay_preparation = Some(preparation);
            return Err(error);
        }
        let old_scope = self.world.quest_scope;
        let scope = preparation.scope;
        self.world.quest_scope = Some(scope);
        preparation.begin_write();
        self.gameplay_preparation = Some(preparation);
        let prepared = self.try_save().and_then(|()| {
            self.gameplay_preparation
                .as_ref()
                .expect("active preparation")
                .confirm()
        });
        if let Err(error) = prepared {
            self.world.quest_scope = old_scope;
            if !self
                .gameplay_preparation
                .as_mut()
                .expect("active preparation")
                .failed()
            {
                self.gameplay_preparation = None;
            }
            return Err(error);
        }
        self.gameplay_preparation = None;
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
        self.players[index]
            .gameplay_clock
            .restore(self.tick_counter);
        Ok(())
    }
}

/// Does the saved `world.dat` carry a world-local personal snapshot? Read-only.
/// An `Err` uses `open_world`'s wording for the same failure: a read failure is
/// "world.dat can't be read (…)", a decode failure "world.dat is damaged (…)".
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn selected_world_local_present(directory: &std::path::Path) -> Result<bool, String> {
    let unreadable = |e: String| format!("world.dat can't be read ({e})");
    let damaged = |e: String| format!("world.dat is damaged ({e})");
    let path = directory.join("world.dat");
    let bytes = match std::fs::File::open(path) {
        Ok(file) => {
            crate::save::read_bounded(file, crate::handover::checkpoint::WORLD_DATA_BYTES as u64)
                .map_err(unreadable)?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(unreadable(error.to_string())),
    };
    let save = crate::save::read_world_save(&bytes).map_err(|e| damaged(e.to_string()))?;
    let Some(local) = save.local_personal else {
        return Ok(false);
    };
    let record = crate::joiner_save::JoinerSave::decode(&local.0).map_err(damaged)?;
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
