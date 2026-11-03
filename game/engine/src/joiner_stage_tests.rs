//! A save cut short at each step, then a load: a matching pair of `world.dat`
//! and sidecars, with no item duplicated or lost.

use super::*;
use crate::chest::ChestData;
use crate::item::ItemStack;
use crate::joiner_save::JoinerSave;
use crate::player_slot::PlayerSlot;
use crate::save::{world_dir, WorldsRootGuard};
use crate::server::GameServer;
use crate::world::World;
use crate::world_open::{open_world, AutosavePolicy, OpenedWorld};
use sha2::{Digest, Sha256};
use std::fs;

const NAME: &str = "stage";
const CHEST: (i32, i32, i32) = (1, 70, 1);
/// The stone in the world, whoever holds it.
const STONE: u8 = 60;
const KEYS: [[u8; 32]; 2] = [[1; 32], [2; 32]];

fn stack(n: u8) -> Option<ItemStack> {
    (n > 0).then(|| ItemStack::new_block(crate::block::STONE, n))
}

/// Two joiners and a chest holding all the stone.
fn rig() -> GameServer {
    let mut s = GameServer::new(2, NAME.into(), 42);
    for (sp, key) in s.players.iter_mut().zip(KEYS) {
        sp.server_simulated = true;
        sp.verified_pubkey = Some(key);
        sp.enter_world();
    }
    s.world.insert_chest(CHEST, ChestData::default());
    hold(&mut s, 0);
    s
}

/// Each joiner holds `each`; the chest holds the rest.
fn hold(s: &mut GameServer, each: u8) {
    s.world.chest_at_mut(CHEST).unwrap().slots[0] = stack(STONE - 2 * each);
    for sp in &mut s.players {
        sp.inventory.set_slot(0, stack(each));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    /// `GameServer::try_save`: a dedicated server, `--keep-world`, a role cut.
    Server,
    /// A host's own save (`GameState::save_live_world`).
    HostSave,
    /// A host's autosave.
    HostAutosave,
}

fn save_by(route: Route, s: &GameServer) -> Result<(), String> {
    let slot = PlayerSlot::new(0, glam::Vec3::new(0.0, 70.0, 0.0), 1.0);
    let seats = std::slice::from_ref(&slot);
    match route {
        Route::Server => s.try_save(),
        Route::HostSave => s.save_with_joiners(|hook: &mut crate::save::BeforeCommit<'_>| {
            crate::save::save_world_superseding_autosave_with(NAME, &s.world, seats, 42, &[], &[], None, None, hook)
        }),
        Route::HostAutosave => s.save_with_joiners(|hook: &mut crate::save::BeforeCommit<'_>| {
            crate::save::autosave_world_with(NAME, &s.world, seats, 42, &[], &[], None, None, hook)
        }),
    }
}

fn saved_stone(key: [u8; 32]) -> u8 {
    let path = crate::handover::live::personal_adapter::path(&world_dir(NAME), key);
    match fs::read(&path) {
        Ok(bytes) => {
            let saved = JoinerSave::decode(&bytes).expect("a whole sidecar");
            saved.window().unwrap().inv.slot(0).map_or(0, |s| s.count)
        }
        Err(_) => 0,
    }
}

/// What a load finds: the chest, and each joiner's sidecar.
fn load(policy: AutosavePolicy) -> (u8, [u8; 2]) {
    let mut world = World::new();
    let opened = open_world(NAME, &mut world, policy).expect("the world opens");
    let chest = match opened {
        OpenedWorld::New => STONE,
        OpenedWorld::Loaded { .. } => world.chest_at(CHEST).expect("a chest").slots[0].as_ref().map_or(0, |s| s.count),
    };
    (chest, KEYS.map(saved_stone))
}

/// `(chest, each joiner)` is the state of one save, whole.
fn assert_pair(found: (u8, [u8; 2]), each: u8, why: &str) {
    assert_eq!(found, (STONE - 2 * each, [each, each]), "{why}");
}

fn cut_short(at: Cut, save: impl FnOnce() -> Result<(), String>) {
    CUT.with(|c| c.set(Some(at)));
    let saved = save();
    CUT.with(|c| c.set(None));
    assert!(saved.as_ref().is_err_and(|e| e.contains("cut short")), "{at:?}: the save stopped there: {saved:?}");
}

fn staging() -> std::path::PathBuf {
    world_dir(NAME).join(STAGED_JOINERS)
}

/// The cut, then the load. Before the commit the world is the old save whole;
/// from the commit on, the new one.
fn crash_at(route: Route, at: Cut, policy: AutosavePolicy, before: u8, after: u8) {
    let _root = WorldsRootGuard::new(&format!("stage-{route:?}-{at:?}"));
    let mut s = rig();
    hold(&mut s, before);
    if route == Route::HostAutosave {
        save_by(Route::HostSave, &s).expect("the last save");
    } else {
        save_by(route, &s).expect("the last save");
    }
    assert_pair(load(policy), before, "the fixture");
    hold(&mut s, after);
    cut_short(at, || save_by(route, &s));
    drop(s);
    let expect = if at == Cut::BeforeCommit { before } else { after };
    assert_pair(load(policy), expect, &format!("{route:?} cut at {at:?}"));
    assert!(!staging().exists(), "{route:?} {at:?}: recovery leaves no stage");
}

const CUTS: [Cut; 4] = [Cut::BeforeCommit, Cut::AfterCommit, Cut::DuringPublish(1), Cut::BeforeRemoval];

#[test]
fn a_server_save_cut_short_anywhere_loads_as_a_matching_pair() {
    for at in CUTS {
        crash_at(Route::Server, at, AutosavePolicy::Ignore, 0, 20); // taken out of the chest
        crash_at(Route::Server, at, AutosavePolicy::Ignore, 20, 5); // put back
    }
}

#[test]
fn a_hosts_save_cut_short_anywhere_loads_as_a_matching_pair() {
    for at in CUTS {
        crash_at(Route::HostSave, at, AutosavePolicy::Prefer, 0, 20);
        crash_at(Route::HostSave, at, AutosavePolicy::Prefer, 20, 5);
    }
}

#[test]
fn a_hosts_autosave_cut_short_anywhere_loads_as_a_matching_pair() {
    for at in CUTS {
        crash_at(Route::HostAutosave, at, AutosavePolicy::Prefer, 0, 20);
        crash_at(Route::HostAutosave, at, AutosavePolicy::Prefer, 20, 5);
    }
}

/// A loader that skips the autosave reads the older save, so the autosave's
/// stage is for a commit it isn't reading.
#[test]
fn a_stage_for_the_autosave_is_dropped_by_a_load_of_world_dat() {
    let _root = WorldsRootGuard::new("stage-autosave-ignored");
    let mut s = rig();
    hold(&mut s, 3);
    save_by(Route::HostSave, &s).unwrap();
    hold(&mut s, 20);
    cut_short(Cut::AfterCommit, || save_by(Route::HostAutosave, &s));
    drop(s);
    assert_pair(load(AutosavePolicy::Ignore), 3, "the older save, whole");
    assert!(!staging().exists());
}

/// An autosave that fails to load falls back to the older save, so its stage
/// must not have published on the way past.
#[test]
fn a_stage_for_an_autosave_that_fails_to_load_is_dropped_with_it() {
    let _root = WorldsRootGuard::new("stage-autosave-torn");
    let dir = world_dir(NAME);
    let mut s = rig();
    s.world.set_block(1, 70, 1, crate::block::STONE);
    hold(&mut s, 3);
    save_by(Route::HostSave, &s).unwrap();
    hold(&mut s, 20);
    cut_short(Cut::AfterCommit, || save_by(Route::HostAutosave, &s));
    drop(s);
    // Its chunks can't be read, so the load fails after its world.dat decodes.
    let chunks = dir.join("autosave/chunks");
    fs::create_dir_all(chunks.join("9_9_9.chunk")).unwrap();
    assert_pair(load(AutosavePolicy::Prefer), 3, "the older save, whole");
    assert!(!staging().exists());
}

#[test]
fn a_first_save_cut_short_anywhere_loads_as_a_matching_pair() {
    for at in CUTS {
        let _root = WorldsRootGuard::new(&format!("stage-first-{at:?}"));
        let mut s = rig();
        hold(&mut s, 20);
        cut_short(at, || s.try_save());
        drop(s);
        // Before the commit there is no saved world: no chest, no sidecars.
        let expect = if at == Cut::BeforeCommit { 0 } else { 20 };
        assert_pair(load(AutosavePolicy::Ignore), expect, &format!("first save cut at {at:?}"));
        if at == Cut::BeforeCommit {
            // Still a new world, so no load ran: the next save drops the stage.
            assert!(staging().exists());
            settle(&world_dir(NAME), Commit::Either, &mut |_, _| {}).unwrap();
        }
        assert!(!staging().exists());
    }
}

#[test]
fn a_stale_stage_is_discarded() {
    let _root = WorldsRootGuard::new("stage-stale");
    let dir = world_dir(NAME);
    let mut s = rig();
    hold(&mut s, 3);
    s.try_save().unwrap();
    let on_disk = fs::read(crate::handover::live::personal_adapter::path(&dir, KEYS[0])).unwrap();

    // For a world.dat that never committed.
    let later = Row { name: format!("{}.dat", crate::hosted_server::pubkey_to_npub(&KEYS[0])), bytes: b"later".to_vec() };
    stage(&dir, &[later], b"a world.dat that never committed").unwrap();
    assert!(staging().join(MARKER).is_file(), "the fixture staged");
    assert_pair(load(AutosavePolicy::Ignore), 3, "the old sidecars stand");
    assert!(!staging().exists(), "and the stage is gone");

    // Cut short before its marker: nothing says what it belongs to.
    fs::create_dir(staging()).unwrap();
    fs::write(staging().join("npub1half.dat"), b"half").unwrap();
    assert_pair(load(AutosavePolicy::Ignore), 3, "the old sidecars stand");
    assert!(!staging().exists());
    assert!(!dir.join("joiners/npub1half.dat").exists());
    assert_eq!(fs::read(crate::handover::live::personal_adapter::path(&dir, KEYS[0])).unwrap(), on_disk);

    // A marker that isn't one.
    fs::create_dir(staging()).unwrap();
    fs::write(staging().join(MARKER), b"not a digest").unwrap();
    fs::write(staging().join("npub1half.dat"), b"half").unwrap();
    assert_pair(load(AutosavePolicy::Ignore), 3, "the old sidecars stand");
    assert!(!dir.join("joiners/npub1half.dat").exists());
}

/// Every file under the world, with its bytes.
fn tree() -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    let mut stack = vec![world_dir(NAME)];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                found.push((path.clone(), Vec::new()));
                stack.push(path);
            } else {
                let bytes = fs::read(&path).unwrap();
                found.push((path, bytes));
            }
        }
    }
    found.sort();
    found
}

#[test]
fn recovery_run_twice_is_the_same() {
    for commit in [Commit::WorldDat, Commit::Either] {
        for at in [Cut::AfterCommit, Cut::DuringPublish(1), Cut::BeforeRemoval] {
            let _root = WorldsRootGuard::new(&format!("stage-twice-{commit:?}-{at:?}"));
            let dir = world_dir(NAME);
            let mut s = rig();
            s.try_save().unwrap();
            hold(&mut s, 20);
            cut_short(at, || s.try_save());
            drop(s);
            settle(&dir, commit, &mut |_, _| {}).unwrap();
            let once = tree();
            settle(&dir, commit, &mut |_, _| {}).unwrap();
            assert_eq!(tree(), once, "{at:?}");
            assert_pair(load(AutosavePolicy::Ignore), 20, "and it holds the new save");
        }
    }
}

/// A recovery cut short is recovered in its turn.
#[test]
fn a_recovery_cut_short_finishes_on_the_next_one() {
    for first in [Cut::AfterCommit, Cut::DuringPublish(1), Cut::BeforeRemoval] {
        for second in [Cut::AfterCommit, Cut::DuringPublish(1), Cut::BeforeRemoval] {
            let _root = WorldsRootGuard::new(&format!("stage-again-{first:?}-{second:?}"));
            let mut s = rig();
            s.try_save().unwrap();
            hold(&mut s, 20);
            cut_short(first, || s.try_save());
            drop(s);
            // What is left may hold too few sidecars to reach the second cut.
            CUT.with(|c| c.set(Some(second)));
            let again = settle(&world_dir(NAME), Commit::WorldDat, &mut |_, _| {});
            CUT.with(|c| c.set(None));
            drop(again);
            assert_pair(load(AutosavePolicy::Ignore), 20, &format!("{first:?} then {second:?}"));
            assert!(!staging().exists());
        }
    }
}

/// The load of a save from before staging, or one with nothing to stage.
#[test]
fn a_legacy_save_with_no_stage_loads_unchanged() {
    let _root = WorldsRootGuard::new("stage-legacy");
    let dir = world_dir(NAME);
    let mut s = rig();
    hold(&mut s, 3);
    s.try_save().unwrap();
    drop(s);
    assert!(!staging().exists());
    let before = tree();
    assert_pair(load(AutosavePolicy::Ignore), 3, "the sidecars and the world");
    settle(&dir, Commit::Either, &mut |_, _| {}).unwrap();
    assert_eq!(tree(), before, "the load wrote nothing");
    assert!(!staging().exists());
}

#[test]
fn a_save_with_no_joiner_stages_nothing() {
    let _root = WorldsRootGuard::new("stage-none");
    let s = GameServer::new(0, NAME.into(), 42);
    // No stage means no cut at the stage: it runs to the end.
    CUT.with(|c| c.set(Some(Cut::BeforeCommit)));
    let saved = s.try_save();
    CUT.with(|c| c.set(None));
    saved.unwrap();
    assert!(!staging().exists());
    assert!(!world_dir(NAME).join("joiners").exists());
}

/// One joiner's record can't be written: a server's save refuses, as it always
/// has, and now publishes nobody else's either.
#[test]
fn a_refused_joiner_stops_a_server_save_with_nothing_published() {
    let _root = WorldsRootGuard::new("stage-refused-server");
    let mut s = rig();
    hold(&mut s, 3);
    s.try_save().unwrap();
    hold(&mut s, 20);
    s.players[1].personal_read_failed = true;
    assert!(s.try_save().is_err());
    assert!(!staging().exists());
    drop(s);
    assert_pair(load(AutosavePolicy::Ignore), 3, "the old save, whole");
}

/// A host's own world still saves past one joiner's refused record, as it
/// always has, and reports it: that joiner keeps the record it has.
#[test]
fn a_refused_joiner_lets_a_hosts_save_commit_and_reports_it() {
    let _root = WorldsRootGuard::new("stage-refused-host");
    let mut s = rig();
    hold(&mut s, 3);
    save_by(Route::HostSave, &s).unwrap();
    hold(&mut s, 20);
    s.players[1].personal_read_failed = true;
    assert!(save_by(Route::HostSave, &s).is_err(), "the refusal is reported");
    drop(s);
    assert_eq!(load(AutosavePolicy::Prefer), (STONE - 40, [20, 3]));
    assert!(!staging().exists());
}

/// A publish that failed with the session still running is finished before a
/// joiner's own leave write: the older staged record can't land on the newer.
#[test]
fn a_leave_after_a_failed_publish_keeps_the_newer_record() {
    let _root = WorldsRootGuard::new("stage-leave");
    let mut s = rig();
    s.try_save().unwrap();
    hold(&mut s, 20);
    cut_short(Cut::AfterCommit, || s.try_save());
    s.players[0].inventory.set_slot(0, stack(7));
    s.save_joiner(0).unwrap();
    assert_eq!(saved_stone(KEYS[0]), 7, "the leave's record");
    assert!(!staging().exists(), "the stage went first");
    s.settle_joiner_stage().unwrap();
    assert_eq!(saved_stone(KEYS[0]), 7, "and nothing landed on it");
    assert_eq!(saved_stone(KEYS[1]), 20, "the other joiner's record was published");
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// What the checkpoint manifest says of a sidecar is what's on disk after every
/// step that can move it: never ahead of a publish that hasn't happened.
#[test]
fn the_manifests_digests_only_move_with_the_publish() {
    let _root = WorldsRootGuard::new("stage-manifest");
    let dir = world_dir(NAME);
    let path = crate::handover::live::personal_adapter::path(&dir, KEYS[0]);
    let name = path.file_name().unwrap().to_str().unwrap().to_string();
    let on_disk = || sha256(&fs::read(&path).unwrap());

    let mut s = rig();
    hold(&mut s, 3);
    s.try_save().unwrap();
    assert_eq!(s.stored_records.joiner_digest(&name), Some(on_disk()), "straight after a commit");

    hold(&mut s, 20);
    cut_short(Cut::BeforeCommit, || s.try_save());
    assert_eq!(s.stored_records.joiner_digest(&name), Some(on_disk()), "staged only");
    s.settle_joiner_stage().unwrap();

    cut_short(Cut::AfterCommit, || s.try_save());
    assert_eq!(s.stored_records.joiner_digest(&name), Some(on_disk()), "committed, not published");
    s.settle_joiner_stage().unwrap();
    assert_eq!(saved_stone(KEYS[0]), 20);
    assert_eq!(s.stored_records.joiner_digest(&name), Some(on_disk()), "published by a later settle");
    s.try_save().unwrap();
    assert_eq!(s.stored_records.joiner_digest(&name), Some(on_disk()), "straight after the next commit");

    // A server that restarts into a cut-short save reads its records after
    // the recovery, so its manifest is true of the disk too.
    hold(&mut s, 5);
    cut_short(Cut::DuringPublish(1), || s.try_save());
    drop(s);
    let mut back = GameServer::new(0, NAME.into(), 42);
    back.initial_load_generating(false).expect("loads");
    assert!(!staging().exists());
    assert_eq!(saved_stone(KEYS[0]), 5);
    assert_eq!(back.stored_records.joiner_digest(&name), Some(on_disk()), "after the recovery");
}

/// Only regular files with a sidecar's name are published.
#[cfg(unix)]
#[test]
fn a_stage_publishes_only_regular_sidecars() {
    let _root = WorldsRootGuard::new("stage-odd");
    let dir = world_dir(NAME);
    let s = rig();
    s.try_save().unwrap();
    drop(s);
    let world_dat = fs::read(dir.join("world.dat")).unwrap();
    stage(&dir, &[Row { name: "npub1good.dat".into(), bytes: b"good".to_vec() }], &world_dat).unwrap();
    let outside = dir.join("outside");
    fs::write(&outside, b"outside").unwrap();
    std::os::unix::fs::symlink(&outside, staging().join("npub1link.dat")).unwrap();
    fs::write(staging().join("other.txt"), b"other").unwrap();
    settle(&dir, Commit::WorldDat, &mut |_, _| {}).unwrap();
    assert_eq!(fs::read(dir.join("joiners/npub1good.dat")).unwrap(), b"good");
    assert!(fs::symlink_metadata(dir.join("joiners/npub1link.dat")).is_err());
    assert!(!dir.join("joiners/other.txt").exists());
    assert!(!staging().exists());
    assert_eq!(fs::read(&outside).unwrap(), b"outside");
}

/// A bound record only in the stage still stops a generic copy of the world.
#[test]
fn a_bound_record_in_the_stage_still_counts_as_bound() {
    use crate::handover::live::personal_adapter::generic_copy_guard;
    use crate::progression::{GameplayBinding, WorldPlayer};
    let _root = WorldsRootGuard::new("stage-bound");
    let dir = world_dir(NAME);
    let scope = [7; 16];
    let mut s = rig();
    let binding = GameplayBinding::WorldLocal { scope, player: WorldPlayer::Npub(KEYS[0]) };
    s.players[0].personal = Some(crate::joiner_save::BoundPersonal::fresh(binding).unwrap());
    let saved = JoinerSave::capture(&s.players[0], s.calendar_id);
    let row = Row { name: format!("{}.dat", crate::hosted_server::pubkey_to_npub(&KEYS[0])), bytes: saved.encode().unwrap() };
    fs::create_dir_all(&dir).unwrap();
    assert!(generic_copy_guard(&dir).is_ok(), "nothing bound yet");
    stage(&dir, &[row], b"a world.dat").unwrap();
    assert!(generic_copy_guard(&dir).is_err(), "the stage's record counts");
}

/// A fence up before the save: nothing staged, nothing published.
#[test]
fn a_fenced_world_stages_nothing() {
    let _root = WorldsRootGuard::new("stage-fenced");
    let dir = world_dir(NAME);
    let mut s = rig();
    hold(&mut s, 3);
    s.try_save().unwrap();
    hold(&mut s, 20);
    crate::save_fence::raise(&dir);
    assert!(s.try_save().is_err());
    assert!(!staging().exists());
    crate::save_fence::lift(&dir);
    drop(s);
    assert_pair(load(AutosavePolicy::Ignore), 3, "the old save, whole");
}

/// A fence that rises between the commit and the publish holds the records in
/// the stage, and a handover refuses until they are settled.
#[test]
fn a_fence_between_the_commit_and_the_publish_holds_the_stage_and_refuses_a_handover() {
    use crate::handover::live::eligibility::saved_record_keys;
    let _root = WorldsRootGuard::new("stage-fence-mid");
    let dir = world_dir(NAME);
    let mut s = rig();
    hold(&mut s, 3);
    s.try_save().unwrap();
    hold(&mut s, 20);
    cut_short(Cut::AfterCommit, || s.try_save());
    assert_eq!(saved_stone(KEYS[0]), 3, "committed, not published");
    assert_eq!(saved_record_keys(&dir).map(|keys| keys.len()).map_err(|r| r.0), Err("A saved player record here is unfinished or unrecognised. Nothing was changed."));

    crate::save_fence::raise(&dir);
    assert!(s.settle_joiner_stage().is_err(), "the fence refuses the publish");
    assert!(staging().join(MARKER).is_file(), "and the stage stands");
    assert_eq!(saved_stone(KEYS[0]), 3);

    crate::save_fence::lift(&dir);
    s.settle_joiner_stage().unwrap();
    assert_eq!(saved_stone(KEYS[0]), 20);
    assert_eq!(saved_record_keys(&dir).map(|keys| keys.len()).map_err(|r| r.0), Ok(2));
}
