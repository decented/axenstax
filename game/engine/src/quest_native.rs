//! Native continuation of the same portable holder and its exact original giver.
//! Legacy local offers and monetary rewards remain in their existing route.
use crate::progression::{GameplayId, GameplayRecord};

impl crate::GameState {
    fn native_gameplay_record(&self, pidx: usize) -> Result<GameplayRecord, String> {
        let slot = self.players.get(pidx).ok_or("missing local gameplay seat")?;
        let binding = slot.personal_binding.as_ref().ok_or("missing local gameplay authority")?;
        if self.joined() {
            return Err("joined gameplay is owned by its host".into());
        }
        if let crate::progression::GameplayBinding::WorldLocal { scope, .. } = binding
            && self.world.quest_scope != Some(*scope)
        {
            return Err("local gameplay belongs to another world scope".into());
        }
        GameplayRecord::from_slot(binding.context(), slot)
    }

    fn commit_native_gameplay(
        &mut self,
        pidx: usize,
        install: crate::progression::install::PreparedProgressionInstall,
    ) {
        install.commit(&mut self.players[pidx]);
        self.players[pidx].gameplay_clock.committed(self.tick_counter);
    }

    fn native_quest_notice(&mut self, error: &str) {
        self.toast = Some((error.into(), web_time::Instant::now() + web_time::Duration::from_secs(3)));
    }

    /// Read the same remaining timers; only an actual expiry writes a revision.
    pub(crate) fn advance_native_gameplay(&mut self) {
        if self.joined() {
            return;
        }
        for pidx in 0..self.players.len() {
            if self.players[pidx].personal_binding.is_none() {
                continue;
            }
            let old_clock = self.players[pidx].gameplay_clock;
            self.players[pidx].gameplay_clock.observe(self.tick_counter);
            let result = (|| -> Result<(), String> {
                let mut record = self.native_gameplay_record(pidx)?;
                let stored = &self.players[pidx].quest_progress;
                let expired = stored.declines.len() != record.quests.declines.len()
                    || (stored.penalty_left > 0 && record.quests.penalty_left == 0);
                if !expired {
                    return Ok(());
                }
                record.revision = record.revision.checked_add(1).ok_or("gameplay revision exhausted")?;
                record.encode()?;
                self.commit_native_gameplay(
                    pidx,
                    crate::progression::install::PreparedProgressionInstall::prepare(record)?,
                );
                Ok(())
            })();
            if let Err(error) = result {
                self.players[pidx].gameplay_clock = old_clock;
                self.players[pidx].mark_gameplay_incomplete();
                self.native_quest_notice(&error);
            }
        }
    }

    /// True means this bound holder owns the operation, including a refusal.
    pub(crate) fn credit_native_personal_kill(
        &mut self,
        pidx: usize,
        kind: crate::mob::MobType,
        pos: glam::Vec3,
    ) -> bool {
        if self.players[pidx].personal_binding.is_none() || self.joined() {
            return false;
        }
        let result = (|| -> Result<(), String> {
            let mut record = self.native_gameplay_record(pidx)?;
            let mut kills: ahash::AHashMap<_, _> = record.kills.iter().copied().collect();
            let count =
                kills.get(&kind).copied().unwrap_or(0).checked_add(1).ok_or("gameplay kill counter exhausted")?;
            kills.insert(kind, count);
            record.kills = kills.into_iter().collect();
            record.kills.sort_by_key(|(kind, _)| {
                u32::from_le_bytes(
                    bincode::serialize(kind).expect("scalar mob kind").try_into().expect("mob kind width"),
                )
            });
            if matches!(kind, crate::mob::MobType::Villager | crate::mob::MobType::Peddler)
                && record.quests.penalty_left == 0
                && let Some(village) = crate::reputation::village_at_position(&self.world, pos)
            {
                let mut reputation = crate::reputation::Reputation::default();
                reputation.per_village.extend(record.reputation.iter().copied());
                reputation.adjust(village, crate::reputation::VILLAGER_KILL_PENALTY);
                record.reputation = reputation.per_village.into_iter().collect();
                record.reputation.sort_by_key(|(village, _)| *village);
                record.quests.penalty_left = crate::reputation::VILLAGER_KILL_REP_COOLDOWN_TICKS as u32;
            }
            record.revision = record.revision.checked_add(1).ok_or("gameplay revision exhausted")?;
            record.encode()?;
            self.commit_native_gameplay(
                pidx,
                crate::progression::install::PreparedProgressionInstall::prepare(record)?,
            );
            Ok(())
        })();
        if let Err(error) = result {
            self.players[pidx].mark_gameplay_incomplete();
            self.native_quest_notice(&error);
        }
        true
    }

    pub(crate) fn decay_native_personal(&mut self, pidx: usize, days: u32) -> bool {
        if self.players[pidx].personal_binding.is_none() || self.joined() {
            return false;
        }
        if days == 0 {
            return true;
        }
        let result = (|| -> Result<(), String> {
            let mut record = self.native_gameplay_record(pidx)?;
            if record.reputation.iter().all(|(_, score)| *score == 0) {
                return Ok(());
            }
            for (_, score) in &mut record.reputation {
                *score = crate::reputation_day::decay_score(*score, days);
            }
            record.revision = record.revision.checked_add(1).ok_or("gameplay revision exhausted")?;
            record.encode()?;
            self.commit_native_gameplay(
                pidx,
                crate::progression::install::PreparedProgressionInstall::prepare(record)?,
            );
            Ok(())
        })();
        if let Err(error) = result {
            self.players[pidx].mark_gameplay_incomplete();
            self.native_quest_notice(&error);
        }
        true
    }

    fn native_quest_giver(&self, pidx: usize, target: hecs::Entity, npc: GameplayId) -> Result<glam::Vec3, String> {
        if !self.ecs.get::<&crate::quest_npcs::QuestNpcId>(target).is_ok_and(|id| id.0 == npc)
            || !self
                .ecs
                .get::<&crate::entity::MobKind>(target)
                .is_ok_and(|kind| matches!(kind.0, crate::mob::MobType::Villager | crate::mob::MobType::Peddler))
            || !self.ecs.get::<&crate::combat::Health>(target).is_ok_and(|health| health.current > 0.0)
            || !self.ecs.get::<&crate::villager::VillagerComponent>(target).is_ok_and(|component| {
                component.profession != crate::villager::Profession::Builder && component.commission.is_none()
            })
        {
            return Err("The original giver is unavailable.".into());
        }
        let pos = self.ecs.get::<&crate::entity::Position>(target).map_err(|_| "The original giver is unavailable.")?.0;
        if !pos.is_finite()
            || self.players[pidx].is_dead()
            || (pos - self.players[pidx].player.eye_pos()).length() >= crate::villager_ui::INTERACT_RANGE
        {
            return Err("Move closer to the original giver.".into());
        }
        Ok(pos)
    }

    /// Validate every candidate before consuming the actual bag. Private bodies
    /// in the rest of the window are never reconstructed from marker copies.
    pub(crate) fn turn_in_native_quest(
        &mut self,
        pidx: usize,
        target: hecs::Entity,
        npc: GameplayId,
        id: GameplayId,
    ) -> Result<(), String> {
        if self.handover_blocks_world_action() {
            return Err("Wait for this world's handover.".into());
        }
        let pos = self.native_quest_giver(pidx, target, npc)?;
        let record = self.native_gameplay_record(pidx)?;
        let slot = &self.players[pidx];
        let raw = crate::staged_window::MarkerWindow::try_capture_parts(
            &slot.inventory,
            &slot.armour_slots,
            &slot.crafting_ui.cursor_item,
            &slot.crafting_ui.grid,
            &self.registry,
        )
        .ok_or("This inventory cannot pay the quest.")?;
        let complete = crate::quest_play::turn_in(&record, npc, id, &raw)
            .map_err(|_| "You're missing something the original quest needs.")?;
        complete.record.encode()?;
        let accepted = record
            .quests
            .accepted
            .iter()
            .find(|quest| quest.npc == npc && quest.id == id)
            .ok_or("The original quest has changed.")?;
        let quest = crate::quest_play::as_native(accepted).map_err(|_| "The original quest is unavailable.")?;
        let install = crate::progression::install::PreparedProgressionInstall::prepare(complete.record)?;
        if !crate::quest::fetch_make_consume(&quest, &mut self.players[pidx].inventory) {
            return Err("You're missing something the original quest needs.".into());
        }
        self.commit_native_gameplay(pidx, install);
        for stack in complete.rewards {
            if let Some(remainder) = self.players[pidx].inventory.add_item(stack) {
                crate::entity::spawn_item(&mut self.ecs, pos, remainder, self.tick_counter as u32);
            }
        }
        self.players[pidx].dialogue_villager = None;
        if pidx == 0 {
            self.capture_cursor();
        }
        Ok(())
    }

    /// Select only an existing exact-giver portable quest or timer. Loading
    /// never generates a new offer or converts a legacy monetary quest.
    pub(crate) fn draw_native_portable_quest(
        &mut self,
        pidx: usize,
        target: hecs::Entity,
        viewport: &crate::screen::ViewportRect,
    ) -> bool {
        if self.joined() || self.players[pidx].personal_binding.is_none() {
            return false;
        }
        let npc = match self.ecs.get::<&crate::quest_npcs::QuestNpcId>(target) {
            Ok(id) => id.0,
            Err(_) => return false,
        };
        let stored = &self.players[pidx].quest_progress;
        if !stored.accepted.iter().any(|quest| quest.npc == npc) && !stored.declines.iter().any(|(id, _)| *id == npc) {
            return false;
        }
        let result = (|| -> Result<(), String> {
            self.native_quest_giver(pidx, target, npc)?;
            let record = self.native_gameplay_record(pidx)?;
            let accepted = record.quests.accepted.iter().find(|quest| quest.npc == npc);
            let (summary, mode) = if let Some(accepted) = accepted {
                let quest = crate::quest_play::as_native(accepted).map_err(|_| "The original quest is unavailable.")?;
                let kills = crate::quest_play::kill_progress(&record, accepted)
                    .map_err(|_| "This quest's earlier kills are not known.")?;
                let (have, need) = crate::quest::progress_against(&quest, &self.players[pidx].inventory, kills);
                (
                    format!("{}  Progress: {have} / {need}.", crate::quest::summarise(&quest, false)),
                    if have >= need {
                        crate::villager_ui::DialogueMode::ReadyToTurnIn
                    } else {
                        crate::villager_ui::DialogueMode::InProgress
                    },
                )
            } else {
                let remaining = record.quests.declines.iter().find(|(id, _)| *id == npc).map_or(0, |(_, ticks)| *ticks);
                (
                    format!("Come back in {} seconds.", remaining.div_ceil(20)),
                    crate::villager_ui::DialogueMode::InProgress,
                )
            };
            let (label, profession, gossip) = {
                let component = self
                    .ecs
                    .get::<&crate::villager::VillagerComponent>(target)
                    .map_err(|_| "The original giver is unavailable.")?;
                (
                    crate::villager_ui::villager_label(target, &component),
                    component.profession,
                    component.gossip_line.clone(),
                )
            };
            match crate::villager_ui::draw_villager_dialogue(
                &self.renderer.egui.ctx,
                viewport,
                pidx,
                &label,
                profession,
                Some(&summary),
                gossip.as_deref(),
                mode,
                None,
                None,
            ) {
                crate::villager_ui::DialogueAction::Close => {
                    self.players[pidx].dialogue_villager = None;
                    if pidx == 0 {
                        self.capture_cursor();
                    }
                }
                crate::villager_ui::DialogueAction::TurnIn => {
                    if let Some(quest) = accepted {
                        self.turn_in_native_quest(pidx, target, npc, quest.id)?;
                    }
                }
                _ => {}
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.native_quest_notice(&error);
        }
        true
    }
}
