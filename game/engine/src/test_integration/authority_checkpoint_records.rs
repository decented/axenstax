//! Keep slice 2b — a restore and the world's stored records: a joiner's sidecar
//! stays the truth until it re-attaches, the world's meta is round-tripped and
//! bound, a refusal after the load leaves the folder as it was, and the load
//! generates nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::authority_checkpoint::{fixture, persist_and_checkpoint, restore, SEED};
use super::authority_checkpoint_refusals::assert_invalid_checkpoint;
use super::twin_run::run_to;
use crate::authority_checkpoint::{seal, section_offsets, RestoreError};
use crate::container_window::ItemDelta;
use crate::item::ItemStack;
use crate::joiner_save::{BoundPersonal, JoinerSave};
use crate::progression::{GameplayBinding, WorldPlayer};
use crate::save::WorldsRootGuard;
use crate::test_harness::{TestConfig, TestHost};

/// Every file under `dir`, with its bytes.
fn folder_bytes(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(dir).expect("a folder").flatten() {
        if entry.metadata().expect("metadata").is_dir() {
            out.extend(folder_bytes(&entry.path()));
        } else {
            out.insert(entry.path(), std::fs::read(entry.path()).expect("a file"));
        }
    }
    out
}

/// Joiner `slot` is bound to this world and owes 12 cobblestone it doesn't hold.
fn bind_and_owe(h: &mut TestHost, slot: usize) {
    let scope = [19; 16];
    let key = h.server.players[slot].verified_pubkey.expect("signed in");
    h.server.world.quest_scope = Some(scope);
    let sp = &mut h.server.players[slot];
    sp.personal = Some(BoundPersonal::fresh(GameplayBinding::WorldLocal { scope, player: WorldPlayer::Npub(key) }).unwrap());
    let stack = ItemStack::new_block(crate::block::COBBLESTONE, 12);
    let take = ItemDelta { take: vec![(0, stack)], give: vec![] };
    sp.window_events.debt.apply(&mut sp.inventory, &mut sp.craft_grid, &mut sp.cursor, &mut sp.armour, &take);
}

/// The record is the truth until the joiner comes back: an autosave, a shutdown
/// save and a save of that one slot all leave it byte for byte, debt and binding
/// included, and nothing reads the slot as owing nothing meanwhile.
#[test]
fn a_restored_joiners_debt_stays_in_its_record() {
    let _root = WorldsRootGuard::new("keep-s2b-debt");
    let mut a = fixture("ckpt-debt", 1);
    bind_and_owe(&mut a, 1);
    run_to(&mut a, 20);
    let bytes = persist_and_checkpoint(&mut a);
    let path = a.server.joiner_path(a.server.players[1].verified_pubkey.unwrap());
    let record = std::fs::read(&path).expect("the save wrote a sidecar");
    let saved = JoinerSave::decode(&record).expect("a record");
    assert!(!saved.debt().unwrap().is_empty(), "the fixture owes something");
    assert!(saved.personal.is_some(), "and is bound");

    let mut b = TestHost::adopt(restore(&a, &bytes).expect("restores"));
    assert!(b.server.players[1].personal.is_some(), "the binding came back from the sidecar");
    assert!(!b.server.players[1].obligations_proven_zero(), "a restored debtor doesn't read as clean");
    b.server.try_save().expect("the restored world saves");
    assert_eq!(std::fs::read(&path).unwrap(), record, "a save rewrote the record");
    b.server.save_joiner(1).expect("a slot's own save does nothing");
    assert_eq!(std::fs::read(&path).unwrap(), record, "a slot's save rewrote the record");
    let end = b.server.tick_counter + 20;
    run_to(&mut b, end);
    b.server.save_joiners().expect("the autosave's pass does nothing");
    assert_eq!(std::fs::read(&path).unwrap(), record, "ticks and an autosave rewrote the record");

    b.server.players[1].enter_world();
    b.server.save_joiner(1).unwrap();
    assert_ne!(std::fs::read(&path).unwrap(), record, "back in the world, its saves are its own again");
}
