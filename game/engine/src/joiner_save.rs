//! Operator-local gameplay saves, keyed only by verified npub.
//!
//! OC (v2, 2026-10-09) — a save carries the window's correction debt
//! (`container_window::CorrectionDebt`) taken from the same fold as the
//! window (`window_events::effective_window_and_debt`), and an
//! [`ObligationHistory`] saying whether that debt is the whole story. A v1
//! save (which held no debt) still loads, with empty debt and
//! `Incomplete(LegacyV1)`; the writer always writes v2. The record is read by
//! dispatching on its leading version byte, never by trying one layout and
//! falling back. Spec: `docs/architecture/2026-10-09-sidecar-obligation-carrier.md`.
use crate::{
    container_window::{CorrectionDebt, MAX_CARRIED_DEBT},
    item::{Item, ItemStack},
    save::SavedSlot,
    window_events::EffectiveWindow,
};
use serde::{Deserialize, Serialize};

/// The layout the writer writes (OC).
const VERSION: u8 = 2;
/// The Sidecar layout (protocol v87), still read ([`JoinerSaveV1`]).
const VERSION_V1: u8 = 1;
/// The largest joiner save read (the bounds keep every write well under it).
pub(crate) const MAX_JOINER_SAVE_BYTES: u64 = 64 * 1024;

/// OC — is a joiner's carried correction debt the whole story? `Complete`:
/// every unit its window has owed since its saves began is carried (or was
/// written off by a death, accepted Mirror behaviour — so "proven zero"
/// means "no carried obligation", not "never had debt cleared").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObligationHistory {
    Complete,
    Incomplete(IncompleteCause),
}

/// OC — why some correction debt may be missing. Wire/disk-stable, append
/// only: LegacyV1 = 0, Overflow = 1, Unreadable = 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IncompleteCause {
    /// The session began from a v1 save, which held no debt.
    LegacyV1,
    /// Some owed debt was left out, so the carried debt may be short: the
    /// live debt grew past what a carrier holds ([`MAX_CARRIED_DEBT`]
    /// entries), its units saturated at `u32::MAX`, or an owed item that
    /// cannot be carried was dropped (the join's sync leaves out an entry the
    /// wire can't carry: a tool or armour piece at durability 0 or above its
    /// maximum, or an unregistered block; see `CorrectionDebt::wire_safe`).
    Overflow,
    /// The joiner's save didn't load this run (`failed_joiner_reads`):
    /// whatever it carried is unknown. Saves stay refused for it.
    Unreadable,
}

/// OC — a session's [`ObligationHistory`] (`ServerPlayer::obligations`).
/// Fresh per attach (`Complete`); it only ever becomes `Incomplete`, and
/// keeps the first cause: a v1 save, an unreadable save, a live debt past the
/// carried bound, an owed item the wire couldn't carry. Nothing turns it back
/// to `Complete`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionObligations(ObligationHistory);

impl Default for SessionObligations {
    fn default() -> Self {
        SessionObligations(ObligationHistory::Complete)
    }
}

impl SessionObligations {
    pub fn get(&self) -> ObligationHistory {
        self.0
    }

    /// From here on the session can't prove its debt (`cause`), unless it
    /// already couldn't (the first cause stays).
    pub fn mark(&mut self, cause: IncompleteCause) {
        if self.0 == ObligationHistory::Complete {
            self.0 = ObligationHistory::Incomplete(cause);
        }
    }

    fn merge(&mut self, history: ObligationHistory) {
        if let ObligationHistory::Incomplete(cause) = history {
            self.mark(cause);
        }
    }
}

#[derive(Serialize, Deserialize)]
enum Slot {
    Saved(SavedSlot),
    Plan { marker: [u8; 32], developed: bool },
}
impl Slot {
    fn capture(stack: Option<&ItemStack>) -> Self {
        if let Some(ItemStack {
            item: Item::Plan(p),
            ..
        }) = stack
        {
            return Self::Plan {
                marker: crate::plan::marker(p),
                developed: matches!(p.develop_state, crate::plan::DevelopState::Developed),
            };
        }
        let mut inv = crate::inventory::Inventory::new();
        inv.set_slot(0, stack.cloned());
        Self::Saved(crate::save::serialize_inventory_raw(&inv).remove(0))
    }
    fn restore(&self) -> Result<Option<ItemStack>, String> {
        match self {
            Self::Plan { marker, developed } => Ok(Some(ItemStack {
                item: Item::Plan(crate::plan::PlanData::marker_placeholder(
                    *marker, *developed,
                )),
                count: 1,
            })),
            Self::Saved(SavedSlot::Plan { .. }) => Err("sidecar contains a Plan body".into()),
            Self::Saved(saved) => {
                let mut inv = crate::inventory::Inventory::new();
                crate::save::restore_inventory(&mut inv, std::slice::from_ref(saved));
                Ok(inv.slot(0).cloned())
            }
        }
    }
}

/// OC — one owed entry in a v2 save: the item in the save's own item
/// encoding ([`SavedSlot`], exact for every non-Plan item, durability
/// included), always as ONE unit, and the units owed beside it (never in the
/// slot's u8 count, which a restore clamps to the stack size).
#[derive(Serialize, Deserialize)]
struct OwedEntry {
    item: SavedSlot,
    units: u32,
}

impl OwedEntry {
    fn capture(item: &Item, units: u32) -> Self {
        let mut inv = crate::inventory::Inventory::new();
        inv.set_slot(0, Some(ItemStack { item: item.clone(), count: 1 }));
        OwedEntry { item: crate::save::serialize_inventory_raw(&inv).remove(0), units }
    }

    /// The entry back, or why the whole save is invalid: an empty slot, a
    /// Plan, or an item that isn't exactly one unit.
    fn restore(&self) -> Result<(Item, u32), String> {
        match &self.item {
            SavedSlot::Empty => return Err("an owed entry with no item".into()),
            SavedSlot::Plan { .. } => return Err("an owed Plan".into()),
            SavedSlot::Block { count, .. } | SavedSlot::Material { count, .. } if *count != 1 => {
                return Err("an owed item that isn't one unit".into());
            }
            _ => {}
        }
        let mut inv = crate::inventory::Inventory::new();
        crate::save::restore_inventory(&mut inv, std::slice::from_ref(&self.item));
        match inv.slot(0) {
            Some(s) if s.count == 1 => Ok((s.item.clone(), self.units)),
            _ => Err("an owed item that doesn't restore".into()),
        }
    }
}

/// v1 (Sidecar, protocol v87) — the retained reader of the v1 layout,
/// field for field what v87 wrote, with its own version check. Never
/// written (tests make golden v1 bytes from it).
#[derive(Deserialize)]
#[cfg_attr(test, derive(Serialize))]
struct JoinerSaveV1 {
    version: u8,
    slots: Vec<Slot>,
    armour: [Option<crate::save::SavedArmourPiece>; 4],
    health: f32,
    hunger: u8,
    spawn: [f32; 3],
    run_id: [u8; 16],
    slept_night: Option<u32>,
}

/// A joiner save as v2 lays it out (and as a v1 save loads, upgraded): the
/// v1 fields in v1's order, then `owed` and `obligation_history`.
#[derive(Serialize, Deserialize)]
pub(crate) struct JoinerSave {
    version: u8,
    slots: Vec<Slot>,
    armour: [Option<crate::save::SavedArmourPiece>; 4],
    pub health: f32,
    pub hunger: u8,
    pub spawn: [f32; 3],
    run_id: [u8; 16],
    slept_night: Option<u32>,
    /// v2 — the effective correction debt, in live order, at most
    /// [`MAX_CARRIED_DEBT`] entries.
    owed: Vec<OwedEntry>,
    /// v2 — whether `owed` is the whole story.
    obligation_history: ObligationHistory,
}

impl JoinerSave {
    /// OC (A7) — a save of `window` and these vitals with NO correction
    /// debt and a `Complete` history: for a window that never had a
    /// correction debt (a host's own local slot, which is trusted). Every
    /// save is laid out by this one constructor.
    pub fn from_parts(
        window: &EffectiveWindow,
        health: f32,
        hunger: u8,
        spawn: [f32; 3],
        run_id: [u8; 16],
        slept_night: Option<u32>,
    ) -> Self {
        let slots = (0..36)
            .map(|i| Slot::capture(window.inv.slot(i)))
            .chain(std::iter::once(Slot::capture(window.cursor.as_ref())))
            .chain(window.grid.iter().flatten().map(|s| Slot::capture(s.as_ref())))
            .collect();
        Self {
            version: VERSION,
            slots,
            armour: crate::save::serialize_armour_slots(&window.armour),
            health,
            hunger,
            spawn,
            run_id,
            slept_night,
            owed: Vec::new(),
            obligation_history: ObligationHistory::Complete,
        }
    }

    /// Joiner `sp`'s save: its effective window and its effective correction
    /// debt from ONE fold (`window_events::effective_window_and_debt`), and
    /// its session's obligation history. A debt past [`MAX_CARRIED_DEBT`]
    /// entries keeps the first ones in live order and is written
    /// `Incomplete(Overflow)` (the session itself is marked where its live
    /// debt passes the bound, `window_events`); the save never fails over it.
    pub fn capture(sp: &crate::server::ServerPlayer, run_id: [u8; 16]) -> Self {
        let fold = crate::window_events::effective_window_and_debt(sp);
        let mut save = Self::from_parts(
            &fold.window,
            sp.combat.health,
            sp.combat.hunger,
            sp.spawn_pos.to_array(),
            run_id,
            sp.slept_night,
        );
        let (carried, dropped) = fold.debt.carried();
        let mut history = sp.obligations;
        if dropped > 0 || fold.saturated {
            history.mark(IncompleteCause::Overflow);
            log::warn!(
                "Joiner save for {}: {dropped} correction-debt entries past the {MAX_CARRIED_DEBT} a save \
                 carries left out{}; its obligations are saved incomplete",
                crate::hosted_server::verified_npub(sp.verified_pubkey).unwrap_or_else(|| sp.display_name.clone()),
                if fold.saturated { " (and owed units saturated)" } else { "" },
            );
        }
        save.owed = carried.entries().iter().map(|(item, n)| OwedEntry::capture(item, *n)).collect();
        save.obligation_history = history.get();
        save
    }

    /// OC (A9) — the bytes to write, after running the reader's own
    /// validation ([`Self::decode`]) on them: what can't be read back is
    /// never written (an older, readable save is kept instead).
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let bytes = bincode::serialize(self).map_err(|e| e.to_string())?;
        Self::decode(&bytes).map_err(|e| format!("a joiner save that wouldn't read back: {e}"))?;
        Ok(bytes)
    }

    /// OC — read a save: dispatch on the leading version byte, then decode
    /// that layout alone (fixint, at most [`MAX_JOINER_SAVE_BYTES`], no
    /// trailing bytes) — v1 into the retained [`JoinerSaveV1`], upgraded with
    /// empty debt and `Incomplete(LegacyV1)` — and validate the window and
    /// the owed list. Any failure invalidates the whole save.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        use bincode::Options;
        let options = || {
            bincode::DefaultOptions::new()
                .with_fixint_encoding()
                .with_limit(MAX_JOINER_SAVE_BYTES)
                .reject_trailing_bytes()
        };
        let saved = match bytes.first() {
            Some(&VERSION) => options().deserialize::<JoinerSave>(bytes).map_err(|e| e.to_string())?,
            Some(&VERSION_V1) => {
                let v1: JoinerSaveV1 = options().deserialize(bytes).map_err(|e| e.to_string())?;
                Self::from_v1(v1)?
            }
            Some(v) => return Err(format!("unsupported joiner save version {v}")),
            None => return Err("empty joiner save".into()),
        };
        saved.window()?;
        saved.debt()?;
        Ok(saved)
    }

    fn from_v1(v1: JoinerSaveV1) -> Result<Self, String> {
        if v1.version != VERSION_V1 {
            return Err("invalid v1 joiner save".into());
        }
        Ok(Self {
            version: VERSION,
            slots: v1.slots,
            armour: v1.armour,
            health: v1.health,
            hunger: v1.hunger,
            spawn: v1.spawn,
            run_id: v1.run_id,
            slept_night: v1.slept_night,
            owed: Vec::new(),
            obligation_history: ObligationHistory::Incomplete(IncompleteCause::LegacyV1),
        })
    }

    pub fn window(&self) -> Result<EffectiveWindow, String> {
        if self.version != VERSION
            || self.slots.len() != 46
            || !self.health.is_finite()
            || !(0.0..=20.0).contains(&self.health)
            || self.hunger > 20
            || self.spawn.iter().any(|v| !v.is_finite())
        {
            return Err("invalid or unsupported joiner save".into());
        }
        for (at, piece) in self.armour.iter().enumerate() {
            if piece.is_some_and(|p| p.slot as usize != at) {
                return Err("invalid equipped armour slot".into());
            }
        }
        let mut w = EffectiveWindow {
            inv: crate::inventory::Inventory::new(),
            armour: crate::save::restore_armour_slots(&self.armour),
            cursor: None,
            grid: Default::default(),
        };
        for at in 0..36 {
            w.inv.set_slot(at, self.slots[at].restore()?);
        }
        w.cursor = self.slots[36].restore()?;
        for r in 0..3 {
            for c in 0..3 {
                w.grid[r][c] = self.slots[37 + r * 3 + c].restore()?;
            }
        }
        Ok(w)
    }

    /// OC — the carried correction debt, in live order; refused whole unless
    /// every entry restores and the list keeps the carried bounds
    /// (`CorrectionDebt::from_entries`: at most [`MAX_CARRIED_DEBT`]
    /// entries, at least one unit each, no Plan, no two equal items).
    pub fn debt(&self) -> Result<CorrectionDebt, String> {
        let entries = self.owed.iter().map(OwedEntry::restore).collect::<Result<Vec<_>, _>>()?;
        CorrectionDebt::from_entries(entries)
    }

    #[cfg(test)]
    pub fn history(&self) -> ObligationHistory {
        self.obligation_history
    }

    /// OC — does this save prove its joiner owes nothing? Its history is
    /// `Complete` and it carries no debt. (An unreadable save has no
    /// `JoinerSave`: [`crate::server::ServerPlayer::obligations_proven_zero`]
    /// covers it.) For C3d's Strict admission and the handover's eligibility.
    #[allow(dead_code)] // consumed by C3d (Strict admission) and the handover (eligibility)
    pub fn obligations_proven_zero(&self) -> bool {
        self.obligation_history == ObligationHistory::Complete && self.owed.is_empty()
    }

    /// Restore the vitals onto joiner `sp`'s body, and the save's obligation
    /// history onto its session (`ServerPlayer::obligations`: an incomplete
    /// save makes the session incomplete, keeping a cause already there).
    pub fn restore_vitals(&self, sp: &mut crate::server::ServerPlayer, run_id: [u8; 16]) {
        sp.combat.health = self.health;
        sp.combat.hunger = self.hunger;
        sp.combat.dead = self.health <= 0.0;
        sp.death_handled = sp.combat.dead;
        sp.slept_night = (self.run_id == run_id)
            .then_some(self.slept_night)
            .flatten();
        sp.obligations.merge(self.obligation_history);
    }
}

impl crate::server::ServerPlayer {
    /// OC — does this joiner provably owe no correction debt? Its session's
    /// history is `Complete` (no v1 save, no unreadable save, no overflow)
    /// and its effective debt (the waiting events folded in) is empty. For
    /// C3d's Strict admission and the handover's eligibility.
    #[allow(dead_code)] // consumed by C3d (Strict admission) and the handover (eligibility)
    pub fn obligations_proven_zero(&self) -> bool {
        self.obligations.get() == ObligationHistory::Complete
            && crate::window_events::effective_window_and_debt(self).debt.is_empty()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl crate::server::GameServer {
    fn joiner_path(&self, pk: [u8; 32]) -> std::path::PathBuf {
        crate::save::world_dir(&self.world_name)
            .join("joiners")
            .join(format!("{}.dat", crate::hosted_server::pubkey_to_npub(&pk)))
    }
    pub(crate) fn load_joiner(&mut self, i: usize) -> Option<JoinerSave> {
        let pk = self.players.get(i)?.verified_pubkey?;
        let path = self.joiner_path(pk);
        let read = std::fs::File::open(&path);
        let saved = if matches!(&read, Err(e) if e.kind() == std::io::ErrorKind::NotFound) {
            None
        } else {
            let result = read
                .map_err(|e| e.to_string())
                .and_then(|file| crate::save::read_bounded(file, MAX_JOINER_SAVE_BYTES))
                .and_then(|bytes| JoinerSave::decode(&bytes));
            match result {
                Ok(saved) => Some(saved),
                Err(e) => {
                    log::warn!("Joiner save {} was not loaded: {e}", path.display());
                    self.failed_joiner_reads.insert(pk);
                    None
                }
            }
        };
        // OC (A1) — a joiner whose save didn't load this run can't prove
        // what it owed: its session starts `Incomplete(Unreadable)`.
        if self.failed_joiner_reads.contains(&pk) {
            self.players[i].obligations.mark(IncompleteCause::Unreadable);
        }
        saved
    }
    pub(crate) fn save_joiner(&self, i: usize) -> Result<(), String> {
        let Some(sp) = self
            .players
            .get(i)
            .filter(|sp| sp.server_simulated && sp.is_in_world())
        else {
            return Ok(());
        };
        let Some(pk) = sp.verified_pubkey else {
            return Ok(());
        };
        if self.failed_joiner_reads.contains(&pk) {
            return Err(format!(
                "Preserving unreadable joiner save {}",
                self.joiner_path(pk).display()
            ));
        }
        let path = self.joiner_path(pk);
        let bytes = JoinerSave::capture(sp, self.run_id).encode()?;
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        crate::save::write_atomic(&path, &bytes)
    }
    pub(crate) fn save_joiners(&self) -> Result<(), String> {
        let errors: Vec<_> = self
            .players
            .iter()
            .enumerate()
            .filter(|(_, sp)| sp.connected)
            .filter_map(|(i, _)| self.save_joiner(i).err())
            .collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}
#[cfg(target_arch = "wasm32")]
impl crate::server::GameServer {
    pub(crate) fn load_joiner(&mut self, _: usize) -> Option<JoinerSave> {
        None
    }
    pub(crate) fn save_joiner(&self, _: usize) -> Result<(), String> {
        Ok(())
    }
    pub(crate) fn save_joiners(&self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    fn server() -> crate::server::GameServer {
        let mut s = crate::server::GameServer::new(1, "sidecar".into(), 42);
        let sp = &mut s.players[0];
        sp.server_simulated = true;
        sp.verified_pubkey = Some([1; 32]);
        sp.enter_world();
        s
    }
    #[test]
    fn sidecar_round_trips_whole_effective_window_and_run_scoped_night() {
        let _g = crate::save::WorldsRootGuard::new("sidecar-full");
        let mut s = server();
        let sp = &mut s.players[0];
        sp.inventory
            .set_slot(7, Some(ItemStack::new_block(crate::block::STONE, 3)));
        sp.cursor = Some(ItemStack::new_block(crate::block::DIRT, 4));
        sp.craft_grid[2][2] = Some(ItemStack::new_material(crate::item::MaterialId::Diamond, 2));
        sp.armour[3] = Some(crate::armour::ArmourItem::new(
            crate::armour::ArmourSlot::Boots,
            crate::armour::ArmourMaterial::Iron,
        ));
        let mut plan = crate::plan::PlanData::debug_3x3_stone();
        plan.name = "Captured locally".into();
        let marker = crate::plan::marker(&plan);
        sp.inventory.set_slot(
            9,
            Some(ItemStack {
                item: Item::Plan(plan),
                count: 1,
            }),
        );
        sp.combat.health = 7.0;
        sp.combat.hunger = 9;
        sp.spawn_pos = glam::Vec3::new(4.5, 70.0, 9.5);
        sp.slept_night = Some(3);
        crate::window_events::queue(
            sp,
            crate::window_events::WindowEvent::Grant(ItemStack::new_block(crate::block::DIRT, 2)),
            0,
        );
        let expected =
            crate::inventory_sync::packet(&crate::window_events::effective_window(sp), &Default::default(), 1);
        s.save_joiner(0).unwrap();
        let path = s.joiner_path([1; 32]);
        assert!(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("npub1")
        );
        let saved = s.load_joiner(0).unwrap();
        assert_eq!(
            crate::inventory_sync::packet(&saved.window().unwrap(), &Default::default(), 1),
            expected
        );
        let restored_window = saved.window().unwrap();
        let Item::Plan(p) = &restored_window.inv.slot(9).unwrap().item else {
            panic!("Plan lost")
        };
        assert_eq!(p.marker, Some(marker));
        assert!(p.cells.is_empty());
        s.players[0].enter_world();
        saved.restore_vitals(&mut s.players[0], s.run_id);
        assert_eq!(
            (
                s.players[0].combat.health,
                s.players[0].combat.hunger,
                s.players[0].slept_night
            ),
            (7.0, 9, Some(3))
        );
        let mut restarted = server();
        let old_run = s.run_id;
        assert_ne!(restarted.run_id, old_run);
        let saved = restarted.load_joiner(0).unwrap();
        saved.restore_vitals(&mut restarted.players[0], restarted.run_id);
        assert_eq!(restarted.players[0].slept_night, None);
        assert_eq!(
            crate::inventory_sync::packet(&saved.window().unwrap(), &Default::default(), 1),
            expected
        );
    }
    #[test]
    fn sidecar_guests_never_write_and_bad_files_survive_later_saves() {
        let _g = crate::save::WorldsRootGuard::new("sidecar-bad");
        let mut s = server();
        s.players[0].verified_pubkey = None;
        s.save_joiner(0).unwrap();
        assert!(!crate::save::world_dir("sidecar").exists());
        s.players[0].verified_pubkey = Some([1; 32]);
        let path = s.joiner_path([1; 32]);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        for bytes in [
            vec![255],
            b"corrupt old data".to_vec(),
            vec![0; 64 * 1024 + 1],
        ] {
            std::fs::write(&path, &bytes).unwrap();
            assert!(s.load_joiner(0).is_none());
            assert!(s.save_joiners().is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
    #[test]
    fn sidecar_dead_window_stays_dead_and_dedicated_save_has_fresh_local_player() {
        let _g = crate::save::WorldsRootGuard::new("sidecar-no-p0");
        let mut s = server();
        s.players[0].combat.health = 0.0;
        s.players[0].combat.dead = true;
        s.players[0]
            .inventory
            .set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 9)));
        s.try_save().unwrap();
        let (world, _) =
            crate::save::load_world("sidecar", &mut crate::world::World::new()).unwrap();
        assert!(world.players.is_empty());
        assert!(world.inventory.is_empty());
        assert_eq!(world.player_health, 20.0);
        let saved = s.load_joiner(0).unwrap();
        s.players[0].enter_world();
        saved.restore_vitals(&mut s.players[0], s.run_id);
        assert!(s.players[0].combat.dead);
        assert!(s.players[0].death_handled);
    }
}

/// OC (2026-10-09) — the obligation carrier: a joiner save carries the
/// window's correction debt (v2) and its obligation history.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod oc_tests {
    use super::*;
    use crate::container_window::ItemDelta;
    use crate::server::ServerPlayer;
    use crate::window_events::{self, WindowEvent};

    const STONE: u16 = crate::block::STONE;

    fn server() -> crate::server::GameServer {
        let mut s = crate::server::GameServer::new(1, "oc".into(), 42);
        let sp = &mut s.players[0];
        sp.server_simulated = true;
        sp.verified_pubkey = Some([1; 32]);
        sp.enter_world();
        s
    }

    fn stone() -> Item {
        Item::Block(STONE)
    }

    fn take(item: &Item, n: u8) -> ItemDelta {
        ItemDelta { take: vec![(0, ItemStack { item: item.clone(), count: n })], give: vec![] }
    }

    fn give(item: &Item, n: u8) -> ItemDelta {
        ItemDelta { take: vec![], give: vec![ItemStack { item: item.clone(), count: n }] }
    }

    fn correction(delta: ItemDelta) -> WindowEvent {
        WindowEvent::Correction(Box::new(crate::window_ops::Correction {
            client: delta.clone(),
            own: Some(delta),
            view: crate::window_ops::ViewEvent::Sets { sets: Vec::new(), furnace: None },
        }))
    }

    /// A short take on the server copy's live ledger — NOT one of the apply
    /// sites that mark a session (`window_events::apply_own_now`, the
    /// numbered path): only a capture sees what it does.
    fn owe(sp: &mut ServerPlayer, item: &Item, n: u8) {
        sp.window_events.debt.apply(&mut sp.inventory, &mut sp.craft_grid, &mut sp.cursor, &mut sp.armour, &take(item, n));
    }

    /// Every tool and armour piece kind, by the wire's own decode.
    fn every_tool_and_armour() -> Vec<Item> {
        let mut out = Vec::new();
        for a in 0..=255u8 {
            for b in 0..=255u8 {
                let tool = crate::protocol::WireItem::Tool { tool_type: a, material: b, durability: 1 };
                out.extend(crate::inventory::item_from_wire_full(&tool));
                let piece = crate::protocol::WireItem::Armour { slot: a, material: b, durability: 1 };
                out.extend(crate::inventory::item_from_wire_full(&piece));
            }
        }
        out
    }

    /// `n` pairwise-unequal items of the largest non-Plan encodings (tools
    /// and armour at every durability).
    fn large_items(n: usize) -> Vec<Item> {
        let mut out = Vec::new();
        for kind in every_tool_and_armour() {
            let max = match &kind {
                Item::Tool(t) => crate::crafting::Tool::new(t.tool_type, t.material).durability,
                Item::Armour(a) => crate::armour::max_durability(a.slot, a.material),
                _ => unreachable!(),
            };
            for d in 1..=max {
                let mut item = kind.clone();
                match &mut item {
                    Item::Tool(t) => t.durability = d,
                    Item::Armour(a) => a.durability = d,
                    _ => unreachable!(),
                }
                out.push(item);
                if out.len() == n {
                    return out;
                }
            }
        }
        panic!("fewer than {n} distinct tools and armour pieces");
    }

    fn every_item() -> Vec<Item> {
        let reg = crate::block::BlockRegistry::new();
        let mut out: Vec<Item> = (1..reg.len()).map(|id| Item::Block(id as u16)).collect();
        out.extend((0..=u16::MAX).filter_map(|v| crate::item::MaterialId::try_from(v).ok()).map(Item::Material));
        for kind in every_tool_and_armour() {
            let max = match &kind {
                Item::Tool(t) => crate::crafting::Tool::new(t.tool_type, t.material).durability,
                Item::Armour(a) => crate::armour::max_durability(a.slot, a.material),
                _ => unreachable!(),
            };
            for d in [1, max / 2, max] {
                let mut item = kind.clone();
                match &mut item {
                    Item::Tool(t) => t.durability = d.max(1),
                    Item::Armour(a) => a.durability = d.max(1),
                    _ => unreachable!(),
                }
                out.push(item);
            }
        }
        out
    }

    fn v1_of(save: JoinerSave) -> JoinerSaveV1 {
        JoinerSaveV1 {
            version: 1,
            slots: save.slots,
            armour: save.armour,
            health: save.health,
            hunger: save.hunger,
            spawn: save.spawn,
            run_id: save.run_id,
            slept_night: save.slept_night,
        }
    }

    fn write_record(s: &crate::server::GameServer, bytes: &[u8]) -> std::path::PathBuf {
        let path = s.joiner_path([1; 32]);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// Brief: raw debt 1 plus a waiting correction give 1 → saved debt
    /// empty, slot empty, no double charge.
    #[test]
    fn oc_capture_takes_the_debt_from_the_windows_own_fold() {
        let mut s = server();
        let run_id = s.run_id;
        let sp = &mut s.players[0];
        owe(sp, &stone(), 1);
        assert_eq!(sp.window_events.debt.owed(&stone()), 1);
        let n = window_events::queue(sp, correction(give(&stone(), 1)), 0);
        let saved = JoinerSave::capture(sp, run_id);
        let w = saved.window().unwrap();
        assert!(w.inv.slots_iter().all(|s| s.is_none()), "the waiting give pays the debt: nothing lands");
        assert!(saved.debt().unwrap().is_empty(), "the give that paid the debt is in the fold: owed again would be a double charge");
        window_events::apply_through(sp, n, 0);
        assert!(sp.window_events.debt.is_empty() && sp.inventory.slots_iter().all(|s| s.is_none()));
    }

    /// v1 bytes (from the retained v1 type; the layout itself is pinned by
    /// literal bytes in `oc_v1_literal_bytes_pin_the_layout`) load as
    /// `Incomplete(LegacyV1)` with today's window and empty debt; a Mirror
    /// session begun from them still writes `Incomplete(LegacyV1)` (v2).
    #[test]
    fn oc_v1_golden_bytes_load_legacy_and_stay_legacy_after_a_session() {
        let _g = crate::save::WorldsRootGuard::new("oc-v1");
        let mut s = server();
        let run_id = s.run_id;
        let sp = &mut s.players[0];
        sp.inventory.set_slot(3, Some(ItemStack::new_block(STONE, 5)));
        sp.cursor = Some(ItemStack::new_block(crate::block::DIRT, 2));
        sp.combat.health = 11.0;
        sp.combat.hunger = 7;
        owe(sp, &stone(), 2);
        let expected = crate::inventory_sync::packet(&window_events::effective_window(sp), &Default::default(), 1);
        let golden = bincode::serialize(&v1_of(JoinerSave::capture(sp, run_id))).unwrap();
        assert_eq!(golden[0], 1, "a v1 record's first byte is its version");
        let path = write_record(&s, &golden);
        let saved = s.load_joiner(0);
        assert!(saved.is_some(), "v1 golden bytes must load (by the v1 layout)");
        let saved = saved.unwrap();
        assert_eq!(saved.history(), ObligationHistory::Incomplete(IncompleteCause::LegacyV1));
        assert!(saved.debt().unwrap().is_empty(), "a v1 record carries no debt (today's behaviour)");
        assert_eq!(crate::inventory_sync::packet(&saved.window().unwrap(), &Default::default(), 1), expected);
        assert!(!saved.obligations_proven_zero());
        // The session it begins, then a Mirror session's debt, paid and owed.
        let mut next = server();
        let saved = next.load_joiner(0).unwrap();
        next.players[0].enter_world();
        saved.restore_vitals(&mut next.players[0], next.run_id);
        assert_eq!(next.players[0].obligations.get(), ObligationHistory::Incomplete(IncompleteCause::LegacyV1));
        window_events::apply_own_now(&mut next.players[0], &take(&stone(), 1), 0);
        window_events::apply_own_now(&mut next.players[0], &give(&stone(), 1), 0);
        assert!(next.players[0].window_events.debt.is_empty());
        next.save_joiner(0).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes[0], 2, "the writer writes v2");
        let resaved = JoinerSave::decode(&bytes).unwrap();
        assert_eq!(resaved.history(), ObligationHistory::Incomplete(IncompleteCause::LegacyV1), "Incomplete is sticky");
        assert!(!next.players[0].obligations_proven_zero());
    }

    /// The v1 layout, pinned by literal bytes (not by the retained type
    /// agreeing with itself): a fixed all-empty fixture whose bincode fixint
    /// encoding was written out by hand from the v87 struct
    /// (`git show 1dc5725e:game/engine/src/joiner_save.rs`: `version: u8`,
    /// `slots: Vec<Slot>`, `armour: [Option<SavedArmourPiece>; 4]`,
    /// `health: f32`, `hunger: u8`, `spawn: [f32; 3]`, `run_id: [u8; 16]`,
    /// `slept_night: Option<u32>`). A later change to `Slot` / `SavedSlot` /
    /// the field order breaks one of the two assertions.
    #[test]
    fn oc_v1_literal_bytes_pin_the_layout() {
        const V1_GOLDEN: [u8; 415] = [
            // version
            0x01,
            // slots: Vec len = 46 (u64)
            0x2e, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // 46 slots, each Slot::Saved (u32 0) then SavedSlot::Empty (u32 0)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // armour: [None; 4] (no length prefix, one tag byte each)
            0x00, 0x00, 0x00, 0x00,
            // health 17.5f32 = 0x418c0000, hunger 13
            0x00, 0x00, 0x8c, 0x41, 0x0d,
            // spawn [10.5, 80.0, -20.5] = 0x41280000, 0x42a00000, 0xc1a40000
            0x00, 0x00, 0x28, 0x41, 0x00, 0x00, 0xa0, 0x42, 0x00, 0x00, 0xa4, 0xc1,
            // run_id, then slept_night None
            0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae, 0xaf,
            0x00,
        ];

        fn fixture() -> JoinerSaveV1 {
            JoinerSaveV1 {
                version: 1,
                slots: (0..46).map(|_| Slot::Saved(SavedSlot::Empty)).collect(),
                armour: [None; 4],
                health: 17.5,
                hunger: 13,
                spawn: [10.5, 80.0, -20.5],
                run_id: std::array::from_fn(|i| 0xa0 + i as u8),
                slept_night: None,
            }
        }
        assert_eq!(bincode::serialize(&fixture()).unwrap(), V1_GOLDEN, "the retained v1 type still writes v87's bytes");
        // The version-dispatching loader reads the literal bytes as v1.
        let loaded = JoinerSave::decode(&V1_GOLDEN).expect("v87's bytes load");
        assert_eq!(loaded.history(), ObligationHistory::Incomplete(IncompleteCause::LegacyV1));
        assert_eq!((loaded.health, loaded.hunger, loaded.spawn), (17.5, 13, [10.5, 80.0, -20.5]));
        assert_eq!(loaded.run_id, std::array::from_fn::<u8, 16, _>(|i| 0xa0 + i as u8));
        assert_eq!(loaded.slept_night, None);
        assert!(loaded.debt().unwrap().is_empty());
        assert!(loaded.window().unwrap().inv.slots_iter().all(|s| s.is_none()));
        assert!(!loaded.obligations_proven_zero());
    }

    /// Bounds: each violation makes the WHOLE record unreadable — never
    /// truncated — so it takes the unreadable path (file preserved, saves
    /// refused, session `Incomplete(Unreadable)`).
    #[test]
    fn oc_owed_bounds_make_the_whole_record_unreadable() {
        let _g = crate::save::WorldsRootGuard::new("oc-bounds");
        let cases: Vec<(&str, Vec<OwedEntry>)> = vec![
            ("257 entries", large_items(257).iter().map(|i| OwedEntry::capture(i, 1)).collect()),
            (
                "a Plan",
                vec![OwedEntry { item: SavedSlot::Plan { data: crate::plan::PlanData::debug_3x3_stone() }, units: 1 }],
            ),
            ("0 units", vec![OwedEntry::capture(&stone(), 0)]),
            ("two equal items", vec![OwedEntry::capture(&stone(), 1), OwedEntry::capture(&stone(), 2)]),
            ("an empty item", vec![OwedEntry { item: SavedSlot::Empty, units: 1 }]),
            ("units in the count", vec![OwedEntry { item: SavedSlot::Block { block_id: STONE, count: 2 }, units: 1 }]),
        ];
        for (why, owed) in cases {
            let mut s = server();
            let mut save = JoinerSave::capture(&s.players[0], s.run_id);
            save.owed = owed;
            let bytes = bincode::serialize(&save).unwrap();
            assert!(JoinerSave::decode(&bytes).is_err(), "{why}: refused by the reader");
            let path = write_record(&s, &bytes);
            assert!(s.load_joiner(0).is_none(), "{why}: never loaded");
            assert_eq!(
                s.players[0].obligations.get(),
                ObligationHistory::Incomplete(IncompleteCause::Unreadable),
                "{why}"
            );
            assert!(!s.players[0].obligations_proven_zero(), "{why}");
            assert!(s.save_joiners().is_err(), "{why}: saves refused");
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "{why}: file bytes unchanged");
        }
        // At the cap exactly, it loads.
        let mut s = server();
        let mut save = JoinerSave::capture(&s.players[0], s.run_id);
        save.owed = large_items(256).iter().map(|i| OwedEntry::capture(i, 3)).collect();
        write_record(&s, &bincode::serialize(&save).unwrap());
        assert_eq!(s.load_joiner(0).unwrap().debt().unwrap().entries().len(), 256);
    }

    /// A capture whose fold holds more than 256 entries writes the first 256
    /// in live order and `Incomplete(Overflow)`; the save never fails over it.
    #[test]
    fn oc_capture_past_the_cap_keeps_the_first_256_and_writes_overflow() {
        let _g = crate::save::WorldsRootGuard::new("oc-overflow");
        let mut s = server();
        for item in large_items(300) {
            owe(&mut s.players[0], &item, 1);
        }
        assert_eq!(s.players[0].window_events.debt.entries().len(), 300);
        assert_eq!(s.players[0].obligations.get(), ObligationHistory::Complete, "no apply site saw it");
        s.save_joiner(0).expect("an overflowing debt never fails the save");
        let saved = s.load_joiner(0).expect("and the save reads back");
        assert_eq!(
            saved.history(),
            ObligationHistory::Incomplete(IncompleteCause::Overflow),
            "a capture past the cap must write Overflow"
        );
        let debt = saved.debt().unwrap();
        assert_eq!(debt.entries(), &s.players[0].window_events.debt.entries()[..256], "the first 256, in live order");
    }

    /// A2 — the session is marked where its live ledger passes the cap (both
    /// real server-copy apply sites), and stays marked once paid down.
    #[test]
    fn oc_a_live_debt_past_the_cap_marks_the_session_for_good() {
        let _g = crate::save::WorldsRootGuard::new("oc-sticky");
        let items = large_items(257);
        let many = ItemDelta {
            take: items.iter().map(|i| (0, ItemStack { item: i.clone(), count: 1 })).collect(),
            give: vec![],
        };
        // `apply_own_now`.
        let mut s = server();
        window_events::apply_own_now(&mut s.players[0], &many, 0);
        assert_eq!(s.players[0].obligations.get(), ObligationHistory::Incomplete(IncompleteCause::Overflow));
        let back = ItemDelta { take: vec![], give: items.iter().map(|i| ItemStack { item: i.clone(), count: 1 }).collect() };
        window_events::apply_own_now(&mut s.players[0], &back, 0);
        assert!(s.players[0].window_events.debt.is_empty(), "paid back in full");
        s.save_joiner(0).unwrap();
        assert_eq!(
            s.load_joiner(0).unwrap().history(),
            ObligationHistory::Incomplete(IncompleteCause::Overflow),
            "sticky: a later capture under the cap still writes Overflow"
        );
        assert!(!s.players[0].obligations_proven_zero());
        // The numbered path (`apply_through`).
        let mut s = server();
        let n = window_events::queue(&mut s.players[0], correction(many), 0);
        assert_eq!(s.players[0].obligations.get(), ObligationHistory::Complete);
        window_events::apply_through(&mut s.players[0], n, 0);
        assert_eq!(s.players[0].obligations.get(), ObligationHistory::Incomplete(IncompleteCause::Overflow));
        // A3 — the first cause stays.
        let mut legacy = SessionObligations::default();
        legacy.mark(IncompleteCause::LegacyV1);
        legacy.mark(IncompleteCause::Overflow);
        assert_eq!(legacy.get(), ObligationHistory::Incomplete(IncompleteCause::LegacyV1));
    }

    /// A4 — owed units past `u32::MAX` saturate (the shared rule, so both
    /// copies alike) and the server marks the session `Overflow`.
    #[test]
    fn oc_owed_units_saturate_and_the_server_marks_overflow() {
        let mut s = server();
        let sp = &mut s.players[0];
        sp.window_events.debt = CorrectionDebt::from_entries(vec![(stone(), u32::MAX - 1)]).unwrap();
        window_events::apply_own_now(sp, &take(&stone(), 5), 0);
        assert_eq!(sp.window_events.debt.owed(&stone()), u32::MAX);
        assert_eq!(sp.obligations.get(), ObligationHistory::Incomplete(IncompleteCause::Overflow));
        let mut client = CorrectionDebt::from_entries(vec![(stone(), u32::MAX - 1)]).unwrap();
        let (mut inv, mut grid, mut cursor, mut armour) =
            (crate::inventory::Inventory::new(), Default::default(), None, [None; 4]);
        let applied = client.apply(&mut inv, &mut grid, &mut cursor, &mut armour, &take(&stone(), 5));
        assert!(applied.saturated);
        assert_eq!(client, sp.window_events.debt, "the same rule on both copies");
    }

    /// The predicate C3d and the handover consume.
    #[test]
    fn oc_obligations_proven_zero_truth_table() {
        let _g = crate::save::WorldsRootGuard::new("oc-truth");
        let join = |saved: &JoinerSave| {
            let mut s = server();
            s.players[0].enter_world();
            saved.restore_vitals(&mut s.players[0], s.run_id);
            let n = window_events::queue(
                &mut s.players[0],
                WindowEvent::Replace { window: Box::new(saved.window().unwrap()), debt: saved.debt().unwrap() },
                0,
            );
            window_events::apply_through(&mut s.players[0], n, 0);
            s
        };
        // No record.
        let mut s = server();
        assert!(s.load_joiner(0).is_none());
        assert!(s.players[0].obligations_proven_zero(), "no record");
        // Fresh v2.
        s.save_joiner(0).unwrap();
        let fresh = s.load_joiner(0).unwrap();
        assert!(fresh.obligations_proven_zero(), "fresh v2");
        assert!(join(&fresh).players[0].obligations_proven_zero(), "fresh v2 session");
        // v2 with owed.
        owe(&mut s.players[0], &stone(), 1);
        s.save_joiner(0).unwrap();
        let owing = s.load_joiner(0).unwrap();
        assert_eq!(owing.history(), ObligationHistory::Complete);
        assert!(!owing.obligations_proven_zero(), "v2 with owed");
        assert!(!join(&owing).players[0].obligations_proven_zero(), "v2 with owed: session");
        // v1.
        let v1 = bincode::serialize(&v1_of(JoinerSave::capture(&server().players[0], s.run_id))).unwrap();
        let path = write_record(&s, &v1);
        let legacy = s.load_joiner(0).unwrap();
        assert!(!legacy.obligations_proven_zero(), "v1");
        assert!(!join(&legacy).players[0].obligations_proven_zero(), "v1 session");
        // Overflow.
        let mut big = server();
        for item in large_items(257) {
            owe(&mut big.players[0], &item, 1);
        }
        big.save_joiner(0).unwrap();
        let over = big.load_joiner(0).unwrap();
        assert!(!over.obligations_proven_zero(), "overflow");
        assert!(!join(&over).players[0].obligations_proven_zero(), "overflow session");
        // Unreadable.
        std::fs::write(&path, b"not a joiner save").unwrap();
        let mut bad = server();
        assert!(bad.load_joiner(0).is_none());
        assert!(!bad.players[0].obligations_proven_zero(), "unreadable");
    }

    /// A7 — a save from parts carries no debt and a `Complete` history.
    #[test]
    fn oc_from_parts_writes_no_debt_and_a_complete_history() {
        let mut w = EffectiveWindow {
            inv: crate::inventory::Inventory::new(),
            armour: [None; 4],
            cursor: Some(ItemStack::new_block(crate::block::DIRT, 4)),
            grid: Default::default(),
        };
        w.inv.set_slot(0, Some(ItemStack::new_block(STONE, 3)));
        w.armour[3] = Some(crate::armour::ArmourItem::new(
            crate::armour::ArmourSlot::Boots,
            crate::armour::ArmourMaterial::Iron,
        ));
        let save = JoinerSave::from_parts(&w, 12.0, 5, [1.0, 70.0, 2.0], [9; 16], Some(4));
        let back = JoinerSave::decode(&save.encode().unwrap()).unwrap();
        assert_eq!(
            crate::inventory_sync::packet(&back.window().unwrap(), &Default::default(), 1),
            crate::inventory_sync::packet(&w, &Default::default(), 1)
        );
        assert!(back.debt().unwrap().is_empty());
        assert_eq!(back.history(), ObligationHistory::Complete);
        assert!(back.obligations_proven_zero());
        assert_eq!((back.health, back.hunger, back.spawn), (12.0, 5, [1.0, 70.0, 2.0]));
    }

    /// A9 — what the reader would refuse is never written; the older save
    /// stays.
    #[test]
    fn oc_a_save_that_would_not_read_back_is_never_written() {
        let _g = crate::save::WorldsRootGuard::new("oc-encode");
        let mut s = server();
        s.save_joiner(0).unwrap();
        let before = std::fs::read(s.joiner_path([1; 32])).unwrap();
        s.players[0].combat.health = f32::NAN;
        assert!(s.save_joiner(0).is_err());
        assert_eq!(std::fs::read(s.joiner_path([1; 32])).unwrap(), before);
    }

    /// Item 6 / A5 — every non-Plan item kind round-trips exactly through the
    /// save's owed encoding and through the wire's (to the client's decode on
    /// the same registry), units beside it, never in a count.
    #[test]
    fn oc_owed_items_round_trip_exactly_across_kinds() {
        let reg = crate::block::BlockRegistry::new();
        let items = every_item();
        assert!(items.iter().any(|i| matches!(i, Item::Tool(_))) && items.iter().any(|i| matches!(i, Item::Armour(_))));
        for item in &items {
            let units = 70_000;
            let entry = OwedEntry::capture(item, units);
            let entry: OwedEntry = bincode::deserialize(&bincode::serialize(&entry).unwrap()).unwrap();
            assert_eq!(entry.restore().unwrap(), (item.clone(), units), "{item:?} on disk");
            let debt = CorrectionDebt::from_entries(vec![(item.clone(), units)]).unwrap();
            assert_eq!(debt.wire_safe(&reg).1, 0, "{item:?} is sent");
            assert_eq!(CorrectionDebt::from_wire(&debt.to_wire(), &reg).unwrap(), debt, "{item:?} on the wire");
        }
        // A whole carried list keeps its order both ways.
        let debt = CorrectionDebt::from_entries(large_items(256).into_iter().rev().zip(1..).collect()).unwrap();
        assert_eq!(CorrectionDebt::from_wire(&debt.to_wire(), &reg).unwrap(), debt);
        let mut save = JoinerSave::from_parts(&window_events::effective_window(&server().players[0]), 20.0, 20, [0.0; 3], [0; 16], None);
        save.owed = debt.entries().iter().map(|(i, n)| OwedEntry::capture(i, *n)).collect();
        assert_eq!(JoinerSave::decode(&save.encode().unwrap()).unwrap().debt().unwrap(), debt);
        // What the client couldn't decode back is never sent.
        let mut broken = crate::crafting::Tool::new(crate::crafting::ToolType::Pickaxe, crate::crafting::ToolMaterial::Iron);
        broken.durability = 0;
        let odd = CorrectionDebt::from_entries(vec![(Item::Tool(broken), 1), (stone(), 2)]).unwrap();
        let (sent, left_out) = odd.wire_safe(&reg);
        assert_eq!((sent.entries(), left_out), (&[(stone(), 2)][..], 1));
    }

    /// Worst cases at the cap fit the 64 KiB file cap and the ordinary
    /// packet cap: every window slot a Plan (the largest slot encoding) and
    /// 256 owed tools and armour (the largest non-Plan encodings).
    #[test]
    fn oc_worst_case_save_and_sync_fit_their_caps() {
        let plan = |i: u8| ItemStack {
            item: Item::Plan(crate::plan::PlanData::marker_placeholder([i; 32], true)),
            count: 1,
        };
        let mut w = EffectiveWindow {
            inv: crate::inventory::Inventory::new(),
            armour: [None; 4],
            cursor: Some(plan(200)),
            grid: Default::default(),
        };
        for at in 0..36 {
            w.inv.set_slot(at, Some(plan(at as u8)));
        }
        for (k, cell) in w.grid.iter_mut().flatten().enumerate() {
            *cell = Some(plan(100 + k as u8));
        }
        for (at, slot) in [
            crate::armour::ArmourSlot::Helmet,
            crate::armour::ArmourSlot::Chestplate,
            crate::armour::ArmourSlot::Leggings,
            crate::armour::ArmourSlot::Boots,
        ]
        .into_iter()
        .enumerate()
        {
            w.armour[at] = Some(crate::armour::ArmourItem::new(slot, crate::armour::ArmourMaterial::Diamond));
        }
        let debt = CorrectionDebt::from_entries(large_items(256).into_iter().map(|i| (i, u32::MAX)).collect()).unwrap();
        let mut save = JoinerSave::from_parts(&w, 20.0, 20, [f32::MAX; 3], [255; 16], Some(u32::MAX));
        save.owed = debt.entries().iter().map(|(i, n)| OwedEntry::capture(i, *n)).collect();
        save.obligation_history = ObligationHistory::Incomplete(IncompleteCause::Unreadable);
        let bytes = save.encode().unwrap();
        eprintln!("OC worst-case joiner save at the cap: {} bytes (cap {MAX_JOINER_SAVE_BYTES})", bytes.len());
        assert!(bytes.len() as u64 <= MAX_JOINER_SAVE_BYTES);
        let pkt = crate::protocol::serialize_packet(
            crate::protocol::PacketType::InventorySync,
            &crate::inventory_sync::packet(&w, &debt, u32::MAX),
        );
        eprintln!(
            "OC worst-case InventorySync at the cap: {} bytes (cap {})",
            pkt.len(),
            crate::protocol::MAX_WIRE_PACKET_LEN
        );
        assert!(pkt.len() <= crate::protocol::MAX_WIRE_PACKET_LEN);
        let back: crate::protocol::InventorySyncPacket = crate::protocol::safe_deserialize(&pkt[1..]).unwrap();
        assert_eq!(back.owed.len(), 256);
    }
}
