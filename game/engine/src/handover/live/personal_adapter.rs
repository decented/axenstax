//! Identity-selected personal bootstrap and mutable saves. Frozen members are
//! used once; a complete current v3 always wins and corruption never replays
//! the seed. Gameplay starts only after file and directory durability succeeds.
use super::eligibility::Refusal;
use crate::handover::{
    certificate::Core,
    durable::{MarkerKind, MarkerStore},
    manifest::{Manifest, MemberKind},
};
use crate::joiner_save::{BoundPersonal, JoinerSave, PersonalBinding};
use std::{fs, io::Write, path::Path};

pub(crate) struct Proof {
    pub core: Core,
    manifest: Manifest,
    retained: bool,
    retained_copy: Option<crate::handover::Id>,
}

pub(crate) fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let before = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !before.is_file() || before.len() > limit {
        return Err("unsafe or oversized personal file".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    let after = file.metadata().map_err(|e| e.to_string())?;
    if !after.is_file() {
        return Err("personal file type changed".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return Err("personal file changed".into());
        }
    }
    crate::save::read_bounded(file, limit)
}

pub(crate) fn proof(world: &Path) -> Result<Option<Proof>, String> {
    super::authority::open_guard(world).map_err(|r| r.0.to_string())?;
    if let Some(info) = super::retained::read(world).map_err(|e| e.to_string())? {
        let copy = info.retirement.retained_copy();
        return Ok(Some(Proof { core: info.core().clone(), manifest: info.manifest, retained: true, retained_copy: Some(copy) }));
    }
    let directory = world.join("handover");
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut transfer = None;
    for (seen, entry) in entries.enumerate() {
        if seen >= 256 {
            return Err("too many handover records".into());
        }
        let name = entry.map_err(|e| e.to_string())?.file_name();
        let Some(name) = name.to_str() else {
            return Err("unrecognised handover record".into());
        };
        if let Some(id) = name
            .strip_prefix("activated-")
            .and_then(|s| s.strip_suffix(".dat"))
        {
            let bytes = hex::decode(id).map_err(|_| "invalid activation name")?;
            let id: [u8; 16] = bytes.try_into().map_err(|_| "invalid activation name")?;
            if transfer.replace(id).is_some() {
                return Err("ambiguous personal activation".into());
            }
        }
    }
    let Some(transfer) = transfer else {
        return Ok(None);
    };
    let store = MarkerStore::open_existing(directory.clone()).map_err(|e| e.to_string())?;
    let activated = store
        .read(MarkerKind::Activated, transfer)
        .map_err(|e| e.to_string())?
        .ok_or("missing activation")?;
    let receipt = store
        .read(MarkerKind::Prepared, transfer)
        .map_err(|e| e.to_string())?;
    let core = crate::handover::phase::activated_core(
        &activated,
        transfer,
        receipt.as_deref().map(Vec::as_slice),
    )
    .map_err(|e| e.to_string())?;
    let bytes = read_regular(
        &directory.join("personal-manifest.dat"),
        crate::handover::manifest::MANIFEST_LIMIT as u64,
    )?;
    let manifest = Manifest::decode(&bytes).map_err(|e| e.to_string())?;
    if manifest.digest().map_err(|e| e.to_string())? != core.manifest
        || manifest.lineage != core.lineage
        || manifest.generation != core.generation
        || manifest.transfer != core.transfer
        || manifest.world != core.snapshot_world
        || manifest.source != core.source_persona
        || manifest.receiver != core.successor_persona
    {
        return Err("personal manifest differs from activated authority".into());
    }
    Ok(Some(Proof { core, manifest, retained: false, retained_copy: None }))
}

impl Proof {
    /// Only the archive's checked offer/manifest can prepare this inactive
    /// copy. Gameplay instead reads its final signed retained provenance.
    pub(super) fn retained_checkpoint(admitted: &crate::handover::checkpoint::AdmittedManifest, copy: crate::handover::Id) -> Self {
        Self { core: admitted.core().clone(), manifest: admitted.manifest().clone(), retained: true, retained_copy: Some(copy) }
    }
    pub(crate) fn is_retained(&self) -> bool { self.retained }
    pub(crate) fn local_owner(&self) -> [u8; 32] {
        if self.retained { self.core.source_persona } else { self.core.successor_persona }
    }
    pub(super) fn manifest(&self) -> &Manifest { &self.manifest }
    /// A visible activation/copy release is not sufficient after an uncertain
    /// directory sync. Retry the selected proof and its directory before play.
    pub(crate) fn confirm_authority_durable(&self, world: &Path) -> Result<(), String> {
        let directory = super::authority::dir(world);
        let name = if self.retained {
            format!("retained-copy-{}.dat", hex::encode(self.retained_copy.ok_or("missing retained copy identity")?))
        } else { format!("activated-{}.dat", hex::encode(self.core.transfer)) };
        crate::fsync::sync_regular(&directory.join(name)).map_err(|e| e.to_string())?;
        crate::fsync::sync_regular(&directory.join("personal-manifest.dat")).map_err(|e| e.to_string())?;
        crate::fsync::sync_dir(&directory).map_err(|e| e.to_string())?;
        crate::fsync::sync_dir(world).map_err(|e| e.to_string())?;
        if let Some(parent) = world.parent() { crate::fsync::sync_dir(parent).map_err(|e| e.to_string())?; }
        Ok(())
    }
    pub(crate) fn binding(&self, participant: [u8; 32]) -> PersonalBinding {
        PersonalBinding {
            context: crate::handover::personal::RecordContext {
                participant,
                lineage: self.core.lineage,
                generation: self.core.generation,
                transfer: self.core.transfer,
            },
            manifest: self.core.manifest,
            core: self.core.digest(),
        }
    }
    pub(crate) fn has_participant(&self, key: [u8; 32]) -> bool {
        self.manifest
            .agreements
            .iter()
            .any(|a| a.participant == key)
    }
}

pub(crate) fn path(world: &Path, key: [u8; 32]) -> std::path::PathBuf {
    world.join("joiners").join(format!(
        "{}.dat",
        crate::hosted_server::pubkey_to_npub(&key)
    ))
}

/// No manifested identity means no imported donation. A manifested identity
/// with a missing or malformed record refuses, rather than receiving a kit.
pub(crate) fn load_or_bootstrap(
    world: &Path,
    key: [u8; 32],
    proof: &Proof,
) -> Result<Option<JoinerSave>, String> {
    if !proof.has_participant(key) {
        return Ok(None);
    }
    let target = path(world, key);
    let bytes = read_regular(&target, crate::joiner_save::MAX_JOINER_SAVE_BYTES)?;
    let mut save = JoinerSave::decode(&bytes)?;
    let binding = proof.binding(key);
    if matches!(bytes.first(), Some(3 | 4)) {
        let current = save
            .personal
            .as_ref()
            .ok_or("current personal record has no binding")?;
        if current.binding.imported() != Some(&binding) {
            return Err("current personal record belongs to another transfer".into());
        }
        current.record()?;
        // A previous rename can have landed before directory fsync failed.
        // Presence alone never admits gameplay; retry full durability here.
        confirm_durable(&target)?;
        return Ok(Some(save));
    }
    if bytes.first() != Some(&2) {
        return Err("unsupported personal checkpoint seed".into());
    }
    let selected = crate::hosted_server::pubkey_to_npub(&key);
    let member = |kind, prefix| {
        proof
            .manifest
            .members
            .iter()
            .find(|m| m.kind == kind && m.path == format!("{prefix}/{selected}.dat"))
    };
    let window =
        member(MemberKind::Sidecar, "joiners").ok_or("missing manifested personal window")?;
    if window.schema != 1
        || window.length != bytes.len() as u64
        || window.digest != crate::handover::hash(&bytes)
    {
        return Err("personal window differs from frozen seed".into());
    }
    let maps = member(MemberKind::Progression, "participants")
        .ok_or("missing manifested personal progress")?;
    let progress = read_regular(
        &world.join(&maps.path),
        crate::handover::personal::RECORD_LIMIT as u64,
    )?;
    if ![1, 2].contains(&maps.schema)
        || crate::handover::personal::PersonalRecord::declared_schema(&progress).map_err(|e| e.to_string())? != maps.schema
        || maps.length != progress.len() as u64
        || maps.digest != crate::handover::hash(&progress)
    {
        return Err("personal progression differs from frozen seed".into());
    }
    let record = crate::handover::personal::PersonalRecord::decode(&progress, &binding.context)
        .map_err(|e| e.to_string())?;
    save.personal = Some(BoundPersonal::new(binding, &record)?);
    write(world, key, &save)?;
    Ok(Some(save))
}

pub(crate) fn confirm_durable(target: &Path) -> Result<(), String> {
    crate::fsync::sync_regular(target).map_err(|e| e.to_string())?;
    let directory = target.parent().ok_or("missing personal directory")?;
    crate::fsync::sync_dir(directory).map_err(|e| e.to_string())?;
    crate::fsync::sync_dir(directory.parent().ok_or("missing world directory")?)
        .map_err(|e| e.to_string())
}

/// Detect a remaining live personal authority even after the dedicated role
/// has cleared the local WorldSave field. This is read-only and bounded; a
/// proofless bound record must never become an ordinary bag or lossy export.
pub(crate) fn has_bound_records(world: &Path) -> Result<bool, String> { scan_bound_records(world, true) }
pub(crate) fn has_any_bound_records(world: &Path) -> Result<bool, String> { scan_bound_records(world, false) }
fn scan_bound_records(world: &Path, imported_only: bool) -> Result<bool, String> {
    let entries = match fs::read_dir(world.join("joiners")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    for (seen, entry) in entries.enumerate() {
        if seen >= 4096 {
            return Err("too many personal records to verify safely".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let Some(npub) = name.to_str().and_then(|name| name.strip_suffix(".dat")) else {
            continue;
        };
        if !npub.starts_with("npub1") {
            continue;
        }
        let bytes = read_regular(&entry.path(), crate::joiner_save::MAX_JOINER_SAVE_BYTES)?;
        if matches!(bytes.first(), Some(3 | 4)) && JoinerSave::decode(&bytes)?.personal.is_some_and(|p| !imported_only || p.binding.imported().is_some()) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Generic archives/forks omit proofs and participant files. Keep the complete
/// world directory for backup until a proof-preserving copy format exists.
pub(crate) fn generic_copy_guard(world: &Path) -> Result<(), String> {
    if proof(world)?.is_some() || scan_bound_records(world, false)? {
        return Err("This transferred world cannot use archive/profile export or Fork. Copy the complete world directory to keep its authority and saved players.".into());
    }
    Ok(())
}

pub(crate) fn write(world: &Path, key: [u8; 32], save: &JoinerSave) -> Result<(), String> {
    write_observed(world, key, save, &mut crate::handover::durable::NoFault)
}

pub(crate) fn write_observed(
    world: &Path,
    key: [u8; 32],
    save: &JoinerSave,
    observer: &mut impl crate::handover::durable::BoundaryObserver,
) -> Result<(), String> {
    use crate::handover::durable::WriteBoundary;
    super::authority::open_guard(world).map_err(|r| r.0.to_string())?;
    let target = path(world, key);
    crate::save_fence::check(&target)?;
    match fs::symlink_metadata(world) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => (),
        Ok(_) => return Err("unsafe world directory for personal save".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(world).map_err(|e| e.to_string())?;
            if let Some(parent) = world.parent() {
                crate::fsync::sync_dir(parent).map_err(|e| e.to_string())?;
            }
        }
        Err(e) => return Err(e.to_string()),
    }
    let bytes = save.encode()?;
    if save
        .personal
        .as_ref()
        .is_some_and(|p| p.binding.context().participant() != Some(key))
    {
        return Err("personal save identity mismatch".into());
    }
    let directory = target.parent().ok_or("missing personal directory")?;
    match fs::symlink_metadata(directory) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => (),
        Ok(_) => return Err("unsafe personal directory".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(directory).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
                    .map_err(|e| e.to_string())?;
            }
        }
        Err(e) => return Err(e.to_string()),
    }
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|e| e.to_string())?;
    let temp = directory.join(format!(".personal-{}.tmp", hex::encode(random)));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file =
            crate::fsync::open_written(&mut options, &temp).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        crate::fsync::sync_written(&file).map_err(|e| e.to_string())?;
        observer
            .reached(WriteBoundary::TempSynced)
            .map_err(|e| e.to_string())?;
        crate::save_fence::commit(&target, || {
            fs::rename(&temp, &target).map_err(|e| e.to_string())
        })?;
        observer
            .reached(WriteBoundary::Renamed)
            .map_err(|e| e.to_string())?;
        crate::fsync::sync_dir(directory).map_err(|e| e.to_string())?;
        crate::fsync::sync_dir(world).map_err(|e| e.to_string())?;
        observer
            .reached(WriteBoundary::DirectorySynced)
            .map_err(|e| e.to_string())
    })();
    if result.is_err() && fs::remove_file(&temp).is_ok() {
        let _ = crate::fsync::sync_dir(directory);
    }
    result
}

/// Role/baseline changes keep the exact selected world snapshot intact and
/// make its new personal selection durable before exposing gameplay.
pub(crate) fn write_world_snapshot(
    world: &Path,
    selected: &Path,
    snapshot: &crate::save::WorldSave,
) -> Result<(), String> {
    super::authority::open_guard(world).map_err(|r| r.0.to_string())?;
    let bytes = crate::save_format::encode_world_save(snapshot)?;
    crate::save::write_atomic(&selected.join("world.dat"), &bytes)?;
    crate::fsync::sync_dir(selected).map_err(|e| e.to_string())?;
    crate::fsync::sync_dir(world).map_err(|e| e.to_string())
}

/// Park an autosave after the real server world has durably become primary.
/// Even a future-dated recovery copy must never regain the old local role.
fn park_local_autosave(world: &Path) -> Result<(), String> {
    let from = world.join("autosave");
    match fs::symlink_metadata(&from) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => (),
        _ => return Err("unsafe local recovery snapshot".into()),
    }
    let mut random = [0; 16];
    getrandom::fill(&mut random).map_err(|e| e.to_string())?;
    let to = world.join(format!("autosave-kept-remote-{}", hex::encode(random)));
    crate::save_fence::commit(&from, || fs::rename(&from, &to).map_err(|e| e.to_string()))?;
    crate::fsync::sync_dir(world).map_err(|e| e.to_string())
}

impl crate::server::GameServer {
    pub(crate) fn claim_remote_personal(
        &mut self,
        key: [u8; 32],
        proof: &Proof,
    ) -> Result<(), String> {
        if key != proof.local_owner() { return Ok(()); }
        self.claim_selected_personal(key, crate::progression::GameplayBinding::Imported(proof.binding(key)))
    }
    pub(crate) fn claim_remote_world_local(&mut self, key: [u8; 32], local_players: usize) -> Result<(), String> {
        let Some(snapshot) = &self.selected_local_personal else { return Ok(()); };
        let selected = JoinerSave::decode(&snapshot.0)?;
        let binding = &selected.personal.as_ref().ok_or("selected personal snapshot has no binding")?.binding;
        let crate::progression::GameplayBinding::WorldLocal { scope, player: crate::progression::WorldPlayer::Npub(owner) } = binding else { return Ok(()); };
        if *owner != key { return Ok(()); }
        if local_players > 0 { return Err("This player is already playing locally here.".into()); }
        if Some(*scope) != self.world.quest_scope { return Err("selected local gameplay scope differs".into()); }
        self.claim_selected_personal(key, binding.clone())
    }
    fn claim_selected_personal(&mut self, key: [u8; 32], expected: crate::progression::GameplayBinding) -> Result<(), String> {
        self.refuse_pending_gameplay_cut()?;
        let world = crate::save::world_dir(&self.world_name);
        if self.selected_local_personal.is_none()
            && matches!(fs::symlink_metadata(world.join("autosave")), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
        {
            // Routine reconnect, or retry after a visible role cut: confirm
            // the primary/park rename instead of saving the whole world.
            crate::fsync::sync_regular(&world.join("world.dat")).map_err(|e| e.to_string())?;
            crate::fsync::sync_dir(&world).map_err(|e| e.to_string())?;
            return Ok(());
        }
        if let Some(local) = &self.selected_local_personal {
            let saved = JoinerSave::decode(&local.0)?;
            if saved
                .personal
                .as_ref()
                .is_none_or(|p| p.binding != expected)
            {
                return Err("selected local personal authority differs".into());
            }
            write(&world, key, &saved)?;
        }
        // This commits the actual live server world and other players, never
        // an old decoded primary. Its explicit None selects remote personal
        // authority; the canonical record was made durable before this cut.
        self.try_save_selected(None)?;
        // A role cut is rarer than a save: prove the newly written chunk
        // contents as well as the primary record before remote gameplay.
        for ((cx, cy, cz), chunk) in self.world.persistable_chunks() {
            if !chunk.is_empty() {
                crate::fsync::sync_regular(
                    &world.join("chunks").join(format!("{cx}_{cy}_{cz}.chunk")),
                )
                .map_err(|e| e.to_string())?;
            }
        }
        if world.join("chunks").is_dir() {
            crate::fsync::sync_dir(&world.join("chunks")).map_err(|e| e.to_string())?;
        }
        crate::fsync::sync_regular(&world.join("world.dat")).map_err(|e| e.to_string())?;
        crate::fsync::sync_dir(&world).map_err(|e| e.to_string())?;
        park_local_autosave(&world)?;
        self.selected_local_personal = None;
        Ok(())
    }
}

pub(crate) fn install(
    save: &JoinerSave,
    slot: &mut crate::player_slot::PlayerSlot,
) -> Result<crate::progression::GameplayBinding, String> {
    let personal = save
        .personal
        .as_ref()
        .ok_or("missing current personal state")?;
    let record = personal.record()?;
    let window = save.window()?;
    slot.inventory = window.inv;
    slot.armour_slots = window.armour;
    slot.crafting_ui.cursor_item = window.cursor;
    slot.crafting_ui.grid = window.grid;
    slot.crafting_ui.open = false;
    fn resolve(stack: &mut crate::item::ItemStack) {
        if let crate::item::Item::Plan(body) = &mut stack.item
            && let Some(marker) = body.marker
            && let Some(resolved) = crate::plan_store::resolve(marker)
        {
            let developed = |p: &crate::plan::PlanData| {
                matches!(p.develop_state, crate::plan::DevelopState::Developed)
            };
            if developed(body) == developed(&resolved) {
                *body = resolved;
            }
        }
    }
    for i in 0..36 {
        if let Some(mut stack) = slot.inventory.slot(i).cloned() {
            resolve(&mut stack);
            slot.inventory.set_slot(i, Some(stack));
        }
    }
    for stack in slot
        .crafting_ui
        .cursor_item
        .iter_mut()
        .chain(slot.crafting_ui.grid.iter_mut().flatten().flatten())
    {
        resolve(stack);
    }
    slot.combat.health = save.health;
    slot.combat.hunger = save.hunger;
    slot.combat.dead = save.health <= 0.0;
    slot.spawn_pos = glam::Vec3::from_array(save.spawn);
    slot.player.velocity = glam::Vec3::ZERO;
    slot.player.reset_fall();
    slot.player.pos = record
        .local
        .map_or(slot.spawn_pos, |l| glam::Vec3::from_array(l.pos));
    if let Some(local) = record.local {
        slot.camera.yaw = local.yaw;
        slot.camera.pitch = local.pitch;
        slot.hotbar_slot = local.hotbar as usize;
        slot.inventory.set_locked_from(
            &(0..36)
                .filter(|i| local.locks & (1 << i) != 0)
                .collect::<Vec<u32>>(),
        );
    }
    record.install_progression(slot);
    slot.personal_binding = Some(personal.binding.clone());
    Ok(personal.binding.clone())
}

pub(crate) fn preflight(
    seed: &[u8],
    progress: &[u8],
    context: crate::handover::personal::RecordContext,
) -> Result<(), Refusal> {
    let result = (|| -> Result<(), String> {
        if seed.first() != Some(&2) { return Err("personal transfer needs an immutable v2 seed".into()); }
        let mut save = JoinerSave::decode(seed)?;
        let record = crate::handover::personal::PersonalRecord::decode(progress, &context)
            .map_err(|e| e.to_string())?;
        save.personal = Some(BoundPersonal::new(
            PersonalBinding {
                context,
                manifest: [1; 32],
                core: [1; 32],
            },
            &record,
        )?);
        save.encode()?;
        crate::personal_sync::snapshot(save.personal.as_ref().unwrap(), [1; 16], 0, 1, 1)?;
        Ok(())
    })();
    result.map_err(|_| Refusal("A player's complete inventory and progress is too large or malformed to move. Nothing was changed."))
}

impl crate::GameState {
    pub(crate) fn preflight_personal_local(&self) -> Result<(), String> {
        self.guard_gameplay_preparation_write()?;
        let Some(binding) = &self.local_personal else {
            return Ok(());
        };
        if binding.context().participant().is_some_and(|key| self.handover_signed_in() != Some(key)) {
            return Err(
                "Sign in as this world's saved player before saving. Nothing was changed.".into(),
            );
        }
        let slot = self.players.first().ok_or("missing personal player")?;
        for item in super::capture::local0_plan_items(slot) {
            if let crate::item::Item::Plan(body) = item {
                let resident = crate::body_residency::allocated_body_bytes(&body)
                    .map_err(|_| "Plan body allocation overflow")?;
                crate::plan_store::remember_checked(&body, resident)?;
            }
        }
        crate::joiner_save::capture_local(slot)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "personal_adapter_tests.rs"]
mod tests;
