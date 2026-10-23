//! Non-admitted scope preparation provenance, never a second gameplay holder.
use crate::progression::{GameplayBinding, GameplayId};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub(crate) const PENDING: &str =
    "Gameplay authority needs a successful preparation retry before saving or handing over.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Session {
    Native(u64),
    Server([u8; 16]),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Selected {
    hash: [u8; 32],
    scope: Option<GameplayId>,
    binding: Option<GameplayBinding>,
}

fn selected(directory: &Path) -> Result<Option<Selected>, String> {
    use crate::handover::live::personal_adapter as adapter;
    let path = directory.join("world.dat");
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
        Ok(_) => (),
    }
    let bytes = adapter::read_regular(&path, crate::handover::checkpoint::WORLD_DATA_BYTES as u64)?;
    let save = crate::save::read_world_save(&bytes).map_err(|error| error.to_string())?;
    if let Some(world) = &save.quest_world {
        world.validate()?;
    }
    let scope = save.quest_world.as_ref().map(|world| world.scope);
    let binding = match save.local_personal {
        Some(local) => {
            let saved = crate::joiner_save::JoinerSave::decode(&local.0)?;
            match saved.personal {
                Some(personal) => {
                    personal.record()?;
                    match &personal.binding {
                        GameplayBinding::WorldLocal { scope: bound, .. }
                            if scope != Some(*bound) =>
                        {
                            return Err(
                                "Selected local holder differs from its quest world scope".into()
                            );
                        }
                        GameplayBinding::Guest { .. } => {
                            return Err(
                                "Guest authority cannot own a selected local snapshot".into()
                            );
                        }
                        _ => (),
                    }
                    Some(personal.binding)
                }
                None => return Err("Selected personal snapshot has no gameplay authority".into()),
            }
        }
        None => None,
    };
    Ok(Some(Selected {
        hash: Sha256::digest(&bytes).into(),
        scope,
        binding,
    }))
}

/// A lent server borrows the native authority, whose holder stays in
/// GameState. Its dedicated-server selected-holder cache is not that owner.
pub(crate) fn confirm_lent_scope(
    directory: &Path,
    scope: GameplayId,
    native_prepared: bool,
) -> Result<(), String> {
    let actual = selected(directory)?.ok_or("Missing prepared native world authority")?;
    let holder_matches = match (native_prepared, actual.binding.as_ref()) {
        (true, Some(binding)) => binding.persists(),
        (false, None) => true,
        _ => false,
    };
    if actual.scope != Some(scope) || !holder_matches {
        return Err("The saved native authority differs from the lent world scope".into());
    }
    crate::handover::live::personal_adapter::confirm_durable(&directory.join("world.dat"))
}

#[derive(Clone, Debug)]
pub(crate) struct ScopePreparation {
    directory: PathBuf,
    session: Session,
    pub(crate) scope: GameplayId,
    /// Exact expected selected-holder identity after this write. A dedicated
    /// attempt preserves its existing selection, independently of the joining
    /// player's npub; None is an exact absence, never a wildcard.
    binding: Option<GameplayBinding>,
    before: Option<Selected>,
    observed: Option<Selected>,
    unresolved: bool,
    writing: bool,
}

impl ScopePreparation {
    pub(crate) fn new(
        directory: PathBuf,
        session: Session,
        scope: GameplayId,
        binding: Option<GameplayBinding>,
        previous_scope: Option<GameplayId>,
        previous_binding: Option<&GameplayBinding>,
    ) -> Self {
        let (before, unreadable) = match selected(&directory) {
            Ok(before) => (before, false),
            Err(_) => (None, true),
        };
        let actual_scope = before.as_ref().and_then(|selected| selected.scope);
        let actual_binding = before
            .as_ref()
            .and_then(|selected| selected.binding.as_ref());
        let unresolved =
            unreadable || actual_scope != previous_scope || actual_binding != previous_binding;
        Self {
            directory,
            session,
            scope,
            binding: binding.or_else(|| previous_binding.cloned()),
            observed: before.clone(),
            before,
            unresolved,
            writing: false,
        }
    }

    pub(crate) fn validate(
        &self,
        directory: &Path,
        session: Session,
        binding: Option<&GameplayBinding>,
    ) -> Result<(), String> {
        if self.directory != directory
            || self.session != session
            || matches!(self.session, Session::Native(_)) && self.binding.as_ref() != binding
            || self.unresolved
        {
            return Err(
                "The pending gameplay preparation identity differs. Nothing was changed.".into(),
            );
        }
        if selected(directory)? != self.observed {
            return Err(
                "The selected gameplay preparation changed. Nothing was overwritten.".into(),
            );
        }
        Ok(())
    }

    pub(crate) fn begin_write(&mut self) {
        self.writing = true;
    }

    pub(crate) fn confirm(&self) -> Result<(), String> {
        let actual = selected(&self.directory)?.ok_or("Missing proposed gameplay primary")?;
        if actual.scope != Some(self.scope) || actual.binding != self.binding {
            return Err("The written gameplay authority differs from its preparation".into());
        }
        crate::handover::live::personal_adapter::confirm_durable(&self.directory.join("world.dat"))
    }

    /// Ordinary writers cannot substitute rolled-back live authority. Only
    /// this synchronous preparation's matching installed candidate may write.
    pub(crate) fn guard_write(
        &self,
        directory: &Path,
        session: Session,
        scope: Option<GameplayId>,
        binding: Option<&GameplayBinding>,
    ) -> Result<(), String> {
        if !self.writing
            || self.directory != directory
            || self.session != session
            || scope != Some(self.scope)
            || self.binding.as_ref() != binding
        {
            return Err(PENDING.into());
        }
        Ok(())
    }

    /// Inspect ANY failed write, including ordinary errors after rename. An
    /// exact unchanged predecessor clears the proposal; a visible candidate
    /// is retained by its actual hash. Unknown/foreign state remains refused.
    pub(crate) fn failed(&mut self) -> bool {
        self.writing = false;
        match selected(&self.directory) {
            Ok(actual) => {
                let candidate = actual.as_ref().is_some_and(|actual| {
                    actual.scope == Some(self.scope) && actual.binding == self.binding
                });
                if !candidate && actual == self.before {
                    return false;
                }
                self.observed = actual;
                self.unresolved = !candidate;
            }
            Err(_) => self.unresolved = true,
        }
        true
    }
}

impl crate::GameState {
    pub(crate) fn guard_gameplay_preparation_write(&self) -> Result<(), String> {
        if let Some(preparation) = &self.gameplay_preparation {
            preparation.guard_write(
                &crate::save::world_dir(&self.world_name),
                Session::Native(self.gameplay_preparation_generation),
                self.world.quest_scope,
                self.local_personal.as_ref(),
            )?;
        }
        Ok(())
    }

    pub(crate) fn refuse_pending_gameplay_cut(&self) -> Result<(), String> {
        if self.gameplay_preparation.is_some() {
            return Err(PENDING.into());
        }
        Ok(())
    }
}

impl crate::server::GameServer {
    pub(crate) fn guard_gameplay_preparation_write(
        &self,
        local: Option<&crate::personal_value::LocalPersonalSave>,
    ) -> Result<(), String> {
        if let Some(preparation) = &self.gameplay_preparation {
            let selected = local
                .map(|local| crate::joiner_save::JoinerSave::decode(&local.0))
                .transpose()?;
            let binding = selected
                .as_ref()
                .and_then(|selected| selected.personal.as_ref())
                .map(|personal| &personal.binding);
            preparation.guard_write(
                &crate::save::world_dir(&self.world_name),
                Session::Server(self.run_id),
                self.world.quest_scope,
                binding,
            )?;
        }
        Ok(())
    }

    pub(crate) fn refuse_pending_gameplay_cut(&self) -> Result<(), String> {
        if self.gameplay_preparation.is_some() {
            return Err(PENDING.into());
        }
        Ok(())
    }
}
