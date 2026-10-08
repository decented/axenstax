//! C3b-1 (2026-10-08, protocol v77; C3b-fix-a, v78) — a joined client's shared
//! container screens: asking the server to open its real chest, dispenser,
//! dropper or furnace, opening on its answer, drawing from the mirror and
//! predicting clicks on it, applying the server's corrections and pushes,
//! and closing by the server's rule (Spec 04 §4.2g). The rules themselves
//! are `container_window`'s; this is the `GameState` glue, kept out of
//! `game_loop.rs`.

use web_time::{Duration, Instant};

use crate::block;

impl crate::GameState {
    /// C3b-1 — the context a container click is judged in for player
    /// `pidx`: `shared` for a joiner's mirror of the server's container.
    pub(crate) fn container_ctx(&self, pidx: usize, shared: bool) -> crate::window::ClickCtx {
        let eye = self.players[pidx].player.eye_pos();
        crate::window::ClickCtx::new(self.is_creative, crate::window::Station::Player, eye, |_| block::AIR)
            .with_shared(shared)
    }

    /// C3b-1 — player `pidx` (a joiner) right-clicked the container at
    /// `cell`: ask the server to open its real one (`OpenContainer`, a window
    /// op). Nothing opens until it answers (`apply_container_opened`); no
    /// private copy is created. Only a container within the server's reach
    /// rule asks (`container_window::container_in_reach`: a Reach Claw's
    /// longer ray would ask for one the server holds out of reach).
    pub(crate) fn request_shared_open(&mut self, pidx: usize, cell: [i32; 3]) {
        let p = &mut self.players[pidx];
        p.place_cooldown = 8;
        if !crate::container_window::container_in_reach(p.player.eye_pos(), cell) {
            self.toast = Some(("You can't open that here.".to_string(), Instant::now() + Duration::from_secs(3)));
            return;
        }
        p.crafting_ui.log_open_container(cell, &p.inventory, &p.armour_slots);
    }

    /// C3b-1 — the server answered our `OpenContainer`. Opened: player 0's
    /// screen opens on a mirror of the server's container
    /// (`PlayerSlot::shared_container`), with the matching `open_*` field
    /// naming its cell. Refused: a toast, and nothing opens.
    pub(crate) fn apply_container_opened(&mut self, pkt: &crate::protocol::ContainerOpenedPacket) {
        if self.players.is_empty() {
            return;
        }
        let Some(mirror) = crate::container_window::SharedContainer::from_opened(pkt, &self.registry) else {
            self.toast = Some(("You can't open that here.".to_string(), Instant::now() + Duration::from_secs(3)));
            return;
        };
        let key = (mirror.cell[0], mirror.cell[1], mirror.cell[2]);
        let p = &mut self.players[0];
        p.open_chest = None;
        p.open_dispenser = None;
        p.open_furnace = None;
        match mirror.kind {
            crate::container_window::ContainerKind::Chest { .. } => p.open_chest = Some(key),
            crate::container_window::ContainerKind::Dispenser | crate::container_window::ContainerKind::Dropper => {
                p.open_dispenser = Some(key)
            }
            crate::container_window::ContainerKind::Furnace => p.open_furnace = Some(key),
        }
        p.shared_container = Some(mirror);
        self.release_cursor();
    }

    /// C3b-1 / C3b-fix-a — a `WindowSlotSet` (a push or a correction, a
    /// window event in its turn): the named container slots of the open
    /// mirror are overwritten, and a correction's item delta is resolved on
    /// player 0's window as it is now, with this session's correction debt
    /// (`container_window::apply_slot_set`); nothing is replayed. Returns,
    /// per give of the delta, the units that didn't fit (the caller reports
    /// them, `ItemAction::GrantUnfit`).
    pub(crate) fn apply_window_slot_set(&mut self, pkt: &crate::protocol::WindowSlotSetPacket) -> Vec<u8> {
        let Some(p) = self.players.first_mut() else { return Vec::new() };
        let mut view = crate::window::WindowMut {
            inv: &mut p.inventory,
            armour: &mut p.armour_slots,
            cursor: &mut p.crafting_ui.cursor_item,
            grid: &mut p.crafting_ui.grid,
            container: p.shared_container.as_mut().map(|m| m.as_mut()),
        };
        let applied = crate::container_window::apply_slot_set(&mut view, pkt, &self.registry, &mut self.window_inbox.debt);
        p.crafting_ui.update_result();
        applied.unfit
    }

    /// C3b-1 — draw player `pidx`'s shared container screen from its mirror
    /// (the chest dialog for a chest, dispenser or dropper; the furnace UI),
    /// and apply what was clicked to the mirror and the window by the shared
    /// rule — the prediction — each logged as a window op. A Plan refused
    /// either way toasts. A close clears the `open_*` field; the close is
    /// sent by [`Self::tick_shared_container`].
    pub(crate) fn draw_shared_container(&mut self, pidx: usize, viewport: &crate::screen::ViewportRect) {
        use crate::container_window::{ContainerClick, ContainerData};
        let Some(mut mirror) = self.players[pidx].shared_container.take() else { return };
        let key = (mirror.cell[0], mirror.cell[1], mirror.cell[2]);
        let mut clicks: Vec<ContainerClick> = Vec::new();
        let mut closed = false;
        match &mirror.contents {
            ContainerData::Furnace(data) => {
                match crate::furnace_ui::draw_furnace_ui(
                    &self.renderer.egui.ctx,
                    viewport,
                    pidx,
                    data,
                    &self.registry,
                    &self.renderer.egui,
                ) {
                    crate::furnace_ui::FurnaceUiOutcome::Closed => closed = true,
                    crate::furnace_ui::FurnaceUiOutcome::SlotClick { kind, mode } => {
                        if matches!(kind, crate::furnace::SlotKind::Output) && data.output.is_some() {
                            self.fire_challenge(crate::scenario::ChallengeEvent::SmeltItem);
                        }
                        let hotbar = self.players[pidx].hotbar_slot;
                        clicks.push(ContainerClick::Furnace { kind, mode, hotbar });
                        self.players[pidx].place_cooldown = crate::player_slot::PLACE_COOLDOWN_TICKS;
                    }
                    crate::furnace_ui::FurnaceUiOutcome::InProgress => {}
                }
            }
            ContainerData::Chest(data) => {
                let result = crate::chest_ui::show_container_dialog(
                    &self.renderer.egui.ctx,
                    mirror.kind.title(),
                    data,
                    &self.players[pidx].inventory,
                    key,
                );
                clicks = result.clicks;
                closed = result.close_requested;
            }
        }
        let ctx = self.container_ctx(pidx, true);
        for click in &clicks {
            let p = &mut self.players[pidx];
            let applied =
                p.crafting_ui.apply_container_click(&mut p.inventory, &mut p.armour_slots, mirror.as_mut(), click, &ctx);
            if applied.result == crate::window::ClickResult::PlanStays {
                let text = match click {
                    ContainerClick::Deposit { .. } => "Plans can't go in shared containers yet",
                    _ => "Plans can't leave shared containers yet",
                };
                self.toast = Some((text.to_string(), Instant::now() + Duration::from_secs(3)));
            }
        }
        let p = &mut self.players[pidx];
        p.shared_container = Some(mirror);
        if closed {
            p.open_chest = None;
            p.open_dispenser = None;
            p.open_furnace = None;
            if pidx == 0 {
                self.capture_cursor();
            }
        }
    }

    /// C3b-1 — keep each shared container screen honest, every frame: it
    /// stays open only while its `open_*` field still names it, the player
    /// is alive and joined, and its cell still holds a container of the same
    /// kind within reach (`SharedContainer::still_open`, the server's rule).
    /// Otherwise the mirror goes, the field is cleared, and the close is
    /// logged as a window op (`CraftingUi::close_container`), so the server
    /// closes the container too and stops pushing it.
    pub(crate) fn tick_shared_container(&mut self) {
        let joined = self.joined();
        for pidx in 0..self.players.len() {
            let p = &self.players[pidx];
            let Some(mirror) = p.shared_container.as_ref() else { continue };
            let key = (mirror.cell[0], mirror.cell[1], mirror.cell[2]);
            let field = match mirror.kind {
                crate::container_window::ContainerKind::Chest { .. } => p.open_chest,
                crate::container_window::ContainerKind::Dispenser | crate::container_window::ContainerKind::Dropper => {
                    p.open_dispenser
                }
                crate::container_window::ContainerKind::Furnace => p.open_furnace,
            };
            let named = field == Some(key);
            let block_at = self.world.get_block(mirror.cell[0], mirror.cell[1], mirror.cell[2]);
            if named && joined && !p.is_dead() && mirror.still_open(block_at, p.player.eye_pos()) {
                continue;
            }
            let p = &mut self.players[pidx];
            p.shared_container = None;
            if named {
                p.open_chest = None;
                p.open_dispenser = None;
                p.open_furnace = None;
            }
            p.crafting_ui.close_container(&mut p.inventory, &mut p.armour_slots);
            if named && pidx == 0 {
                self.capture_cursor();
            }
        }
    }
}

/// C3b-1 (v77) — the break path's cleanup of the chest, furnace and
/// dispenser/dropper block entities at `pos` in a client's own `world`: each
/// is cleared (`chest::cleanup_chest`, `furnace::cleanup_furnace`,
/// `dispenser::cleanup_dispenser`), and its contents spilled as ground items
/// in `ecs` at the block centre only when `spill`. A JOINED client passes
/// `false`: its copy of a container is never the real one (a worldgen loot
/// chest's generated copy, or nothing), and the server's spill of the real
/// one (`HostedServer::spill_container_on_change`) is the only spill, so
/// exactly one set of real items lands. Single-player and a host spill, as
/// they always have.
pub(crate) fn clear_broken_containers(world: &mut crate::world::World, ecs: &mut hecs::World, pos: [i32; 3], spill: bool) {
    let at = glam::Vec3::new(pos[0] as f32 + 0.5, pos[1] as f32 + 0.5, pos[2] as f32 + 0.5);
    let [x, y, z] = pos;
    for (contents, salt) in [
        (crate::chest::cleanup_chest(world, x, y, z), 7919),
        (crate::furnace::cleanup_furnace(world, x, y, z), 6959),
        (crate::dispenser::cleanup_dispenser(world, x, y, z), 6151),
    ] {
        if !spill {
            continue;
        }
        for (k, stack) in contents.into_iter().enumerate() {
            crate::entity::spawn_item(ecs, at, stack, k as u32 * salt);
        }
    }
}
