//! Operator-local gameplay saves, keyed only by verified npub.
use crate::{
    item::{Item, ItemStack},
    save::SavedSlot,
    window_events::EffectiveWindow,
};
use serde::{Deserialize, Serialize};

const VERSION: u8 = 1;
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
}
impl JoinerSave {
    pub fn capture(sp: &crate::server::ServerPlayer, run_id: [u8; 16]) -> Self {
        let w = crate::window_events::effective_window(sp);
        let slots = (0..36)
            .map(|i| Slot::capture(w.inv.slot(i)))
            .chain(std::iter::once(Slot::capture(w.cursor.as_ref())))
            .chain(w.grid.iter().flatten().map(|s| Slot::capture(s.as_ref())))
            .collect();
        Self {
            version: VERSION,
            slots,
            armour: crate::save::serialize_armour_slots(&w.armour),
            health: sp.combat.health,
            hunger: sp.combat.hunger,
            spawn: sp.spawn_pos.to_array(),
            run_id,
            slept_night: sp.slept_night,
        }
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
    pub fn restore_vitals(&self, sp: &mut crate::server::ServerPlayer, run_id: [u8; 16]) {
        sp.combat.health = self.health;
        sp.combat.hunger = self.hunger;
        sp.combat.dead = self.health <= 0.0;
        sp.death_handled = sp.combat.dead;
        sp.slept_night = (self.run_id == run_id)
            .then_some(self.slept_night)
            .flatten();
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
        if matches!(&read, Err(e) if e.kind() == std::io::ErrorKind::NotFound) {
            return None;
        }
        let result = read
            .map_err(|e| e.to_string())
            .and_then(|file| crate::save::read_bounded(file, 64 * 1024))
            .and_then(|bytes| {
                use bincode::Options;
                let saved: JoinerSave = bincode::DefaultOptions::new()
                    .with_fixint_encoding()
                    .with_limit(64 * 1024)
                    .reject_trailing_bytes()
                    .deserialize(&bytes)
                    .map_err(|e| e.to_string())?;
                saved.window()?;
                Ok(saved)
            });
        match result {
            Ok(saved) => Some(saved),
            Err(e) => {
                log::warn!("Joiner save {} was not loaded: {e}", path.display());
                self.failed_joiner_reads.insert(pk);
                None
            }
        }
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
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        let bytes =
            bincode::serialize(&JoinerSave::capture(sp, self.run_id)).map_err(|e| e.to_string())?;
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
            crate::inventory_sync::packet(&crate::window_events::effective_window(sp), 1);
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
            crate::inventory_sync::packet(&saved.window().unwrap(), 1),
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
            crate::inventory_sync::packet(&saved.window().unwrap(), 1),
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
