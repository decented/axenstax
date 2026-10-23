//! Joined dialogue rendering and unpredicted requests. Payment/reward/progression
//! reach the player only through the ordered authoritative carriers.
use crate::{
    quest_wire::{QuestOperation, QuestRequest, QuestResult, QuestStatus, QuestViewPush},
    window::WindowMut,
};

impl crate::GameState {
    pub(crate) fn joined_quest_open(&self) -> bool {
        self.remote_client
            .as_ref()
            .is_some_and(|remote| remote.quest_view.is_some())
    }
    pub(crate) fn request_quest(
        &mut self,
        target: u32,
        incarnation: u64,
        operation: QuestOperation,
    ) {
        if self.players.is_empty()
            || self.players[0].is_dead()
            || self.pack_window_frozen(0)
            || self.handover_blocks_world_action()
        {
            return;
        }
        let Some(remote) = self
            .remote_client
            .as_ref()
            .filter(|remote| remote.is_connected())
        else {
            return;
        };
        let actor_respawns = remote.applied_actor_respawns();
        let external = remote.pack_retained_usage();
        if self
            .window_inbox
            .add_bounded(
                Vec::new(),
                Vec::new(),
                Vec::new(),
                (
                    external.0,
                    external
                        .1
                        .saturating_add(self.chunk_intake.carried_metadata_bytes()),
                ),
            )
            .is_err()
        {
            return;
        }
        let slot = &self.players[0];
        for stack in slot
            .inventory
            .slots_iter()
            .flatten()
            .chain(slot.crafting_ui.cursor_item.iter())
            .chain(slot.crafting_ui.grid.iter().flatten().flatten())
        {
            if let crate::item::Item::Plan(body) = &stack.item
                && let Err(error) = crate::plan_store::remember_checked(body, 0)
            {
                log::warn!("Quest request could not retain its private Plan: {error}");
                self.toast = Some((
                    "A local Plan could not be saved; try again.".into(),
                    web_time::Instant::now() + web_time::Duration::from_secs(3),
                ));
                return;
            }
        }
        for body in self.sent_uses.private_plans() {
            if crate::plan_store::remember_checked(body, 0).is_err() {
                return;
            }
        }
        let Some(raw) = crate::staged_window::MarkerWindow::try_capture_parts(
            &slot.inventory,
            &slot.armour_slots,
            &slot.crafting_ui.cursor_item,
            &slot.crafting_ui.grid,
            &self.registry,
        ) else {
            return;
        };
        let request = QuestRequest {
            actor_respawns,
            target,
            before: raw.digest_with(slot.crafting_ui.station(), None),
            incarnation,
            operation,
        };
        match self
            .remote_client
            .as_mut()
            .expect("connected client checked")
            .reserve_quest_request(request)
        {
            Ok(action) => self.send_request(crate::remote_client::Request::Action(action)),
            Err(error) => log::debug!("Quest request waits: {error}"),
        }
    }
    pub(crate) fn apply_quest_result(&mut self, packet: &QuestResult) -> Result<(), String> {
        let Some(remote) = self.remote_client.as_ref() else {
            return Ok(());
        };
        if packet.actor_respawns != remote.applied_actor_respawns() {
            return Ok(());
        }
        if let Some(view) = &packet.view {
            view.validate()?;
            if let Some(terms) = &view.terms {
                terms.native(&self.registry)?;
            }
        }
        let mode = remote.inventory_mode;
        let slot = &mut self.players[0];
        crate::pack_client::apply_canonical_window(
            &mut WindowMut {
                inv: &mut slot.inventory,
                armour: &mut slot.armour_slots,
                cursor: &mut slot.crafting_ui.cursor_item,
                grid: &mut slot.crafting_ui.grid,
                container: None,
            },
            &packet.window,
            &self.registry,
            &self.sent_uses,
            &mut self.window_inbox.debt,
            mode,
        )?;
        slot.crafting_ui.update_result();
        let open = packet.view.is_some();
        let remote = self.remote_client.as_mut().expect("connected client");
        remote.quest_view = packet.view.clone();
        remote.quest_note = Some(packet.note);
        if open {
            self.release_cursor();
        } else {
            self.capture_cursor();
        }
        let message = packet.note.message();
        if !message.is_empty() {
            self.toast = Some((
                message.into(),
                web_time::Instant::now() + web_time::Duration::from_secs(2),
            ));
        }
        Ok(())
    }
    pub(crate) fn apply_quest_view(&mut self, packet: &QuestViewPush) -> Result<(), String> {
        packet.validate()?;
        let Some(remote) = self.remote_client.as_mut() else {
            return Ok(());
        };
        if packet.actor_respawns != remote.applied_actor_respawns() {
            return Ok(());
        }
        let Some(current) = &remote.quest_view else {
            return Ok(());
        };
        if current.target != packet.target || current.incarnation != packet.incarnation {
            return Ok(());
        }
        if let Some(view) = &packet.view {
            if view.npc != current.npc || view.revision < current.revision {
                return Ok(());
            }
            if let Some(terms) = &view.terms {
                terms.native(&self.registry)?;
            }
        }
        remote.quest_view = packet.view.clone();
        if remote.quest_view.is_none() {
            self.capture_cursor();
        }
        Ok(())
    }
    pub(crate) fn draw_joined_quest(&mut self, viewport: &crate::screen::ViewportRect) {
        let Some(view) = self
            .remote_client
            .as_ref()
            .and_then(|remote| remote.quest_view.clone())
        else {
            return;
        };
        let pending = self
            .remote_client
            .as_ref()
            .is_some_and(|remote| remote.pack_request_pending());
        let (summary, mode) = match view.status {
            QuestStatus::Offer => (
                view.terms
                    .as_ref()
                    .and_then(|terms| terms.native(&self.registry).ok())
                    .map(|quest| crate::quest::summarise(&quest, false))
                    .unwrap_or_else(|| "This offer is unavailable.".into()),
                crate::villager_ui::DialogueMode::Offer,
            ),
            QuestStatus::Accepted { progress, target } => {
                let summary = view
                    .terms
                    .as_ref()
                    .and_then(|terms| terms.native(&self.registry).ok())
                    .map(|quest| crate::quest::summarise(&quest, false))
                    .unwrap_or_else(|| "The original quest is unavailable.".into());
                (
                    format!("{summary}  Progress: {progress} / {target}."),
                    if progress >= target {
                        crate::villager_ui::DialogueMode::ReadyToTurnIn
                    } else {
                        crate::villager_ui::DialogueMode::InProgress
                    },
                )
            }
            QuestStatus::Cooldown { remaining } => (
                format!("Come back in {} seconds.", remaining.div_ceil(20)),
                crate::villager_ui::DialogueMode::InProgress,
            ),
            QuestStatus::NoQuest => (
                "No quest at the moment.".into(),
                crate::villager_ui::DialogueMode::InProgress,
            ),
            QuestStatus::Unavailable => (
                "The original giver is unavailable.".into(),
                crate::villager_ui::DialogueMode::InProgress,
            ),
        };
        let summary = if pending {
            format!("{summary}  Waiting for the world…")
        } else {
            summary
        };
        // No monetary or raid/defender copy enters this joined adapter.
        let action = crate::villager_ui::draw_villager_dialogue(
            &self.renderer.egui.ctx,
            viewport,
            0,
            view.label.as_str().unwrap_or("Villager"),
            view.profession,
            Some(&summary),
            view.gossip.as_ref().and_then(|gossip| gossip.as_str().ok()),
            if pending {
                crate::villager_ui::DialogueMode::InProgress
            } else {
                mode
            },
            None,
            None,
        );
        use crate::villager_ui::DialogueAction;
        let operation = match (action, view.terms.as_ref()) {
            (DialogueAction::Accept, Some(terms)) if !pending => {
                Some(QuestOperation::Accept { id: terms.id })
            }
            (DialogueAction::Decline, Some(terms)) if !pending => {
                Some(QuestOperation::Decline { id: terms.id })
            }
            (DialogueAction::TurnIn, Some(terms)) if !pending => {
                Some(QuestOperation::TurnIn { id: terms.id })
            }
            (DialogueAction::Close, _) if !pending => Some(QuestOperation::Close),
            _ => None,
        };
        if let Some(operation) = operation {
            self.request_quest(view.target, view.incarnation, operation);
        }
    }
}
