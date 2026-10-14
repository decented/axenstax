//! Prepare ordinary dialogue against the actual actor, giver and common holder.
//! Candidates own scratch state only; reservations precede every live mutation.
use super::*;
use crate::{
    action_identity::ActionHeader,
    progression::GameplayRecord,
    quest_state::QuestSession,
    quest_wire::{
        QuestNote, QuestOperation, QuestRequest, QuestStatus, QuestTerms, QuestView, Text,
    },
    window_events::{PackWindowStage, RewardOrigin},
};

pub(super) struct QuestAttempt {
    pub window: PackWindowStage,
    pub personal: crate::joiner_save::BoundPersonal,
    pub session: Option<QuestSession>,
    pub view: Option<QuestView>,
    pub rewards: Vec<crate::item::ItemStack>,
    pub origin: Option<RewardOrigin>,
    pub engagement: Option<(hecs::Entity, crate::progression::GameplayId)>,
    pub note: QuestNote,
}

fn domain_note(refusal: crate::quest_play::Refusal) -> QuestNote {
    use crate::quest_play::Refusal;
    match refusal {
        Refusal::Changed => QuestNote::NoOffer,
        Refusal::Cooldown => QuestNote::Cooldown,
        Refusal::Accepted => QuestNote::AlreadyAccepted,
        Refusal::MissingItems => QuestNote::MissingItems,
        Refusal::IncompleteKills => QuestNote::IncompleteKills,
        Refusal::Untracked => QuestNote::Untracked,
        Refusal::Capacity => QuestNote::Waiting,
    }
}

impl HostedServer {
    pub(super) fn quest_view(
        &self,
        i: usize,
        session: &QuestSession,
        record: &GameplayRecord,
    ) -> Result<QuestView, QuestNote> {
        let ecs = &self.server.ecs;
        let component = ecs
            .get::<&crate::villager::VillagerComponent>(session.entity)
            .map_err(|_| QuestNote::NoGiver)?;
        let pos = ecs
            .get::<&crate::entity::Position>(session.entity)
            .map_err(|_| QuestNote::NoGiver)?
            .0;
        let village = crate::reputation::village_at_position(&self.server.world, pos);
        let reputation = village
            .and_then(|v| record.reputation.iter().find(|(village, _)| *village == v))
            .map_or(0, |(_, score)| *score);
        let accepted = record
            .quests
            .accepted
            .iter()
            .find(|quest| quest.npc == session.npc);
        let cooldown = record
            .quests
            .declines
            .iter()
            .find(|(npc, _)| *npc == session.npc)
            .map(|(_, ticks)| *ticks);
        let (terms, status) = if let Some(quest) = accepted {
            let native = crate::quest_play::as_native(quest).map_err(domain_note)?;
            let raw = crate::window_events::pack_canonical_window(
                &self.server.players[i],
                &self.server.registry,
            )
            .map_err(|_| QuestNote::Updated)?;
            let kills = crate::quest_play::kill_progress(record, quest).map_err(domain_note)?;
            let (progress, target) =
                crate::quest::progress_against(&native, &raw.window.inv, kills);
            (
                Some(QuestTerms::capture(quest).map_err(|_| QuestNote::Waiting)?),
                QuestStatus::Accepted {
                    progress: progress.min(target),
                    target,
                },
            )
        } else if let Some(remaining) = cooldown {
            (None, QuestStatus::Cooldown { remaining })
        } else if let Some(offer) = &session.offer {
            (
                Some(QuestTerms::capture(&offer.quest).map_err(|_| QuestNote::Waiting)?),
                QuestStatus::Offer,
            )
        } else {
            (None, QuestStatus::NoQuest)
        };
        let view = QuestView {
            target: session.target,
            npc: session.npc,
            incarnation: session.incarnation,
            revision: record.revision,
            label: Text::new(component.profession.name()).map_err(|_| QuestNote::Waiting)?,
            profession: component.profession,
            gossip: component
                .gossip_line
                .as_deref()
                .map(Text::new)
                .transpose()
                .map_err(|_| QuestNote::Waiting)?,
            terms,
            status,
            reputation,
        };
        view.validate().map_err(|_| QuestNote::Updated)?;
        Ok(view)
    }

    pub(super) fn current_quest_giver(
        &self,
        i: usize,
        request: &QuestRequest,
    ) -> Result<hecs::Entity, QuestNote> {
        let (entity, kind) = self
            .joiner_target(i, request.target)
            .ok_or(QuestNote::OutOfReach)?;
        if !matches!(
            kind,
            crate::mob::MobType::Villager | crate::mob::MobType::Peddler
        ) {
            return Err(QuestNote::NoGiver);
        }
        let ecs = &self.server.ecs;
        if ecs.get::<&crate::satoshi::SatoshiMarker>(entity).is_ok() {
            return Err(QuestNote::Denied);
        }
        let component = ecs
            .get::<&crate::villager::VillagerComponent>(entity)
            .map_err(|_| QuestNote::NoGiver)?;
        if component.profession == crate::villager::Profession::Builder
            || component.commission.is_some()
        {
            return Err(QuestNote::Denied);
        }
        let position = ecs
            .get::<&crate::entity::Position>(entity)
            .map_err(|_| QuestNote::NoGiver)?
            .0;
        if position.distance(self.server.players[i].player.pos) > crate::villager_ui::INTERACT_RANGE
        {
            return Err(QuestNote::OutOfReach);
        }
        self.remote_plot_allows(
            &self.server.players[i],
            position.x.floor() as i32,
            position.z.floor() as i32,
        )
        .map_err(|_| QuestNote::Denied)?;
        if self.server.play_mode.is_spectator() {
            return Err(QuestNote::Denied);
        }
        Ok(entity)
    }

    pub(super) fn prepare_quest_attempt(
        &self,
        i: usize,
        action: ActionHeader,
        request: &QuestRequest,
    ) -> Result<QuestAttempt, QuestNote> {
        let sp = &self.server.players[i];
        if request.actor_respawns != sp.respawns
            || !sp.is_present_and_alive()
            || sp.window_events.predates_replace(action.events_applied)
        {
            return Err(QuestNote::Dead);
        }
        if !sp.window_events.accepts_boundary(action.events_applied) {
            return Err(QuestNote::Updated);
        }
        let raw = crate::window_events::pack_raw_marker(sp, &self.server.registry)
            .map_err(|_| QuestNote::Updated)?;
        if raw.digest_with(sp.station, None) != request.before {
            return Err(QuestNote::Updated);
        }
        let personal = sp.personal.as_ref().ok_or(QuestNote::Denied)?;
        let record = personal.record().map_err(|_| QuestNote::Denied)?;
        let (mut record, expired) = sp.gameplay_clock.project(&record, self.server.tick_counter);
        if expired {
            record.revision = record.revision.checked_add(1).ok_or(QuestNote::Waiting)?;
        }
        let entity = self.current_quest_giver(i, request)?;
        let position = self
            .server
            .ecs
            .get::<&crate::entity::Position>(entity)
            .map_err(|_| QuestNote::NoGiver)?
            .0;
        let scope = self.server.world.quest_scope.ok_or(QuestNote::Denied)?;
        let talk = matches!(request.operation, QuestOperation::Talk);
        let mut session = if talk {
            let npc = crate::quest_npcs::prepare_engagement(scope, &self.server.ecs, entity)
                .map_err(|_| QuestNote::Waiting)?;
            // Opening the same live encounter retains its frozen offer. A new
            // encounter gets a new incarnation; no profession/position fallback.
            match sp.quest_session.as_ref().filter(|session| {
                session.entity == entity
                    && session.npc == npc
                    && session.actor_respawns == sp.respawns
            }) {
                Some(session) => session.clone(),
                None => {
                    let component = self
                        .server
                        .ecs
                        .get::<&crate::villager::VillagerComponent>(entity)
                        .map_err(|_| QuestNote::NoGiver)?;
                    let village =
                        crate::reputation::village_at_position(&self.server.world, position);
                    let offer = village
                        .map(|village| {
                            crate::quest_play::Offer::generate(
                                npc,
                                village,
                                component.profession,
                                u32::from_le_bytes(
                                    npc[..4].try_into().expect("four identity bytes"),
                                ) ^ self.server.tick_counter as u32,
                            )
                        })
                        .transpose()
                        .map_err(domain_note)?
                        .flatten();
                    QuestSession {
                        entity,
                        target: request.target,
                        npc,
                        incarnation: sp
                            .quest_next_incarnation
                            .checked_add(1)
                            .ok_or(QuestNote::Waiting)?,
                        actor_respawns: sp.respawns,
                        offer,
                        revision: record.revision,
                        window_digest: raw.digest_with(sp.station, None),
                    }
                }
            }
        } else {
            sp.quest_session
                .as_ref()
                .filter(|session| {
                    session.entity == entity
                        && session.target == request.target
                        && session.incarnation == request.incarnation
                        && session.actor_respawns == sp.respawns
                        && self
                            .server
                            .ecs
                            .get::<&crate::quest_npcs::QuestNpcId>(entity)
                            .is_ok_and(|id| id.0 == session.npc)
                })
                .cloned()
                .ok_or(QuestNote::NoOffer)?
        };
        let mut cost = crate::staged_window::StagedClick {
            after: raw.clone(),
            result: crate::window::ClickResult::Done,
            sources: std::array::from_fn(|_| None),
        };
        let mut rewards = Vec::new();
        let mut note = QuestNote::Done;
        match request.operation {
            QuestOperation::Talk | QuestOperation::Close => {}
            QuestOperation::Accept { id } => {
                let offer = session
                    .offer
                    .as_ref()
                    .filter(|offer| offer.quest.id == id)
                    .ok_or(QuestNote::NoOffer)?;
                record = crate::quest_play::accept(&record, offer).map_err(domain_note)?;
            }
            QuestOperation::Decline { id } => {
                let offer = session
                    .offer
                    .as_ref()
                    .filter(|offer| offer.quest.id == id)
                    .ok_or(QuestNote::NoOffer)?;
                record = crate::quest_play::decline(&record, offer).map_err(domain_note)?;
            }
            QuestOperation::TurnIn { id } => {
                let completion = crate::quest_play::turn_in(&record, session.npc, id, &raw)
                    .map_err(domain_note)?;
                record = completion.record;
                cost = completion.cost;
                rewards = completion.rewards;
                session.offer = None;
                note = QuestNote::Completed;
            }
        }
        let window =
            crate::window_events::admit_pack_staged(sp, &self.server.registry, &raw, &cost)
                .map_err(|_| QuestNote::MissingItems)?;
        session.revision = record.revision;
        let view = if matches!(request.operation, QuestOperation::Close) {
            None
        } else {
            Some(self.quest_view(i, &session, &record)?)
        };
        let origin = (!rewards.is_empty()).then_some(RewardOrigin {
            npc: session.npc,
            position,
        });
        let engagement = talk.then_some((entity, session.npc));
        Ok(QuestAttempt {
            window,
            personal: crate::joiner_save::BoundPersonal::with_gameplay(
                personal.binding.clone(),
                &record,
            )
            .map_err(|_| QuestNote::Waiting)?,
            session: (!matches!(request.operation, QuestOperation::Close)).then_some(session),
            view,
            rewards,
            origin,
            engagement,
            note,
        })
    }
}
