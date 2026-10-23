//! Sequential ordinary quest authority uses the existing pack flight and ledger.
use super::quest_attempt::QuestAttempt;
use super::*;
use crate::{
    action_identity::{ActionFate, ActionHeader},
    action_wire::ActionReceipt,
    quest_wire::{QuestRequest, QuestResult},
    window_events::WindowEvent,
};

impl HostedServer {
    /// Complete cost/reward/progression carriers and both retention budgets
    /// are prepared before engaging a giver or consuming an accepted quest.
    pub(super) fn serve_quest(
        &mut self,
        i: usize,
        action: ActionHeader,
        request: &QuestRequest,
    ) -> bool {
        let sp = &mut self.server.players[i];
        if sp.pack_flight.is_some() || !self.edit_queues[i].is_empty() {
            return false;
        }
        if sp.window_events.accepts_boundary(action.events_applied)
            && !sp.window_events.predates_replace(action.events_applied)
        {
            crate::window_events::apply_through(
                sp,
                action.events_applied,
                self.server.tick_counter,
            );
        }
        // NPC scope is world-owned even when the admitted actor is a guest.
        // Establish it durably only for a real current ordinary-giver request;
        // this does not create a guest's identity file or weaken lent ownership.
        if self.server.world.quest_scope.is_none()
            && self.current_quest_giver(i, request).is_ok()
            && let Err(error) = self.server.prepare_signed_scope()
        {
            log::warn!("Quest giver scope could not be prepared: {error}");
        }
        let (attempt, fate) = match self.prepare_quest_attempt(i, action, request) {
            Ok(attempt) => (attempt, ActionFate::Accepted),
            Err(note) => {
                let sp = &self.server.players[i];
                let Some(personal) = sp.personal.clone() else {
                    return false;
                };
                let Ok(window) =
                    crate::window_events::pack_canonical_window(sp, &self.server.registry)
                else {
                    return false;
                };
                let view = sp.quest_session.as_ref().and_then(|session| {
                    personal
                        .record()
                        .ok()
                        .and_then(|record| self.quest_view(i, session, &record).ok())
                });
                let fate = if request.actor_respawns != sp.respawns
                    || !sp.is_present_and_alive()
                    || sp.window_events.predates_replace(action.events_applied)
                {
                    ActionFate::Superseded
                } else {
                    ActionFate::Refused
                };
                (
                    QuestAttempt {
                        window,
                        personal,
                        session: sp.quest_session.clone(),
                        view,
                        rewards: Vec::new(),
                        origin: None,
                        engagement: None,
                        note,
                    },
                    fate,
                )
            }
        };
        let QuestAttempt {
            window,
            personal,
            session,
            view,
            rewards,
            origin,
            engagement,
            note,
        } = attempt;
        let Ok(carried) = crate::pack_wire::PackWindow::capture(
            &window.window,
            &window.debt,
            &self.server.registry,
        ) else {
            return false;
        };
        let sp = &mut self.server.players[i];
        let Some(cost_event) = sp.window_events.last_number().checked_add(1) else {
            return false;
        };
        let Some(last) = cost_event
            .checked_add(rewards.len() as u32)
            .and_then(|event| event.checked_add(1))
        else {
            return false;
        };
        let Some(revision) = sp.personal_delivery.revision.checked_add(1) else {
            return false;
        };
        let Some(namespace) = sp
            .action_frontier
            .as_ref()
            .map(|frontier| frontier.header(0, 0))
        else {
            return false;
        };
        let Ok(snapshot) = crate::personal_sync::snapshot(
            &personal,
            namespace.session,
            sp.respawns,
            revision,
            last,
        ) else {
            return false;
        };
        let mut events = Vec::with_capacity(rewards.len() + 2);
        events.push(WindowEvent::PackCanonical {
            actor_respawns: request.actor_respawns,
            window: window.window,
            debt: window.debt,
        });
        let mut followups = Vec::with_capacity(rewards.len() + 1);
        for (at, stack) in rewards.into_iter().enumerate() {
            let Some(origin) = origin else {
                return false;
            };
            let Some(packet) = super::grant_packet(&stack, cost_event + at as u32 + 1) else {
                return false;
            };
            events.push(WindowEvent::RewardGrant { stack, origin });
            followups.push(packet.into_boxed_slice());
        }
        events.push(WindowEvent::PersonalSync(Box::new(snapshot.clone())));
        followups.push(
            protocol::serialize_packet(
                protocol::PacketType::PersonalSync,
                &protocol::PersonalSyncPacket::Snapshot(snapshot.clone()),
            )
            .into_boxed_slice(),
        );
        let Ok(prepared_events) =
            crate::window_events::prepare_pack_events(sp, events.into_boxed_slice())
        else {
            return false;
        };
        debug_assert_eq!(prepared_events.last_number(), last);
        let result = QuestResult {
            actor_respawns: request.actor_respawns,
            action,
            fate,
            note,
            window_event: cost_event,
            included_events: last,
            window: carried,
            view,
        };
        let carrier = result.encode().map(|bytes| {
            let mut carrier = Vec::with_capacity(bytes.len() + 1);
            carrier.push(protocol::PacketType::QuestResult as u8);
            carrier.extend_from_slice(&bytes);
            carrier.into_boxed_slice()
        });
        let prepared_result = carrier.ok().and_then(|carrier| {
            sp.action_results.as_mut().and_then(|results| {
                results
                    .prepare_quest(
                        ActionReceipt {
                            action,
                            fate,
                            included_events: last,
                        },
                        carrier,
                        followups.into_boxed_slice(),
                    )
                    .ok()
            })
        });
        let Some(prepared_result) = prepared_result else {
            prepared_events.abandon(sp);
            return false;
        };
        if let Some((entity, npc)) = engagement
            && crate::quest_npcs::commit_engagement(&mut self.server.ecs, entity, npc).is_err()
        {
            prepared_events.abandon(&mut self.server.players[i]);
            return false;
        }
        let sp = &mut self.server.players[i];
        if fate == ActionFate::Accepted {
            sp.personal = Some(personal);
            sp.gameplay_clock.committed(self.server.tick_counter);
            sp.quest_session = session;
            if let Some(session) = &sp.quest_session {
                sp.quest_next_incarnation = sp.quest_next_incarnation.max(session.incarnation);
            }
            sp.pack_session = None;
            crate::window_ops::close_container_view(sp);
        }
        prepared_events.commit(sp, self.server.tick_counter);
        let personal_ack = crate::personal_sync::acknowledgement(&snapshot);
        sp.personal_delivery
            .issue(snapshot, self.server.tick_counter);
        sp.pack_flight = Some(crate::pack_state::PackFlight {
            action,
            event: last,
            requires_personal: true,
            personal_ack: Some(personal_ack),
        });
        self.complete_typed_result(i, action, fate, prepared_result);
        true
    }
    /// Refresh only when gameplay revision, holdings or the live giver change.
    /// Remaining timers do not generate a moving per-tick progression fence.
    pub(super) fn push_open_quests(&mut self) {
        for i in 0..self.server.players.len() {
            let Some(mut session) = self.server.players[i].quest_session.clone() else {
                continue;
            };
            if !self.handshake_done.get(i).copied().unwrap_or(false)
                || self.disconnected.get(i).copied().unwrap_or(true)
            {
                continue;
            }
            let sp = &self.server.players[i];
            let namespace = sp
                .action_frontier
                .as_ref()
                .map(|frontier| frontier.header(0, 0));
            let Some(namespace) = namespace else {
                continue;
            };
            let raw = crate::window_events::pack_canonical_window(sp, &self.server.registry);
            let Ok(raw) = raw else {
                continue;
            };
            let digest = crate::window::digest_with(
                &raw.window.inv,
                &raw.window.armour,
                &raw.window.cursor,
                &raw.window.grid,
                sp.station,
                None,
            );
            let request = QuestRequest {
                actor_respawns: sp.respawns,
                target: session.target,
                before: digest,
                incarnation: session.incarnation,
                operation: crate::quest_wire::QuestOperation::Talk,
            };
            let record = sp
                .personal
                .as_ref()
                .and_then(|personal| personal.record().ok());
            let view = if session.actor_respawns == sp.respawns
                && self.current_quest_giver(i, &request).is_ok()
            {
                record
                    .as_ref()
                    .and_then(|record| self.quest_view(i, &session, record).ok())
            } else {
                None
            };
            if view.is_some()
                && record
                    .as_ref()
                    .is_some_and(|record| record.revision == session.revision)
                && digest == session.window_digest
            {
                continue;
            }
            let sp = &mut self.server.players[i];
            let Ok(events) =
                crate::window_events::prepare_pack_events(sp, Box::new([WindowEvent::PackView]))
            else {
                continue;
            };
            let packet = crate::quest_wire::QuestViewPush {
                actor_respawns: sp.respawns,
                session: namespace.session,
                epoch: namespace.epoch,
                life: namespace.life,
                window_event: events.last_number(),
                target: session.target,
                incarnation: session.incarnation,
                view,
            };
            if packet.validate().is_err() {
                events.abandon(sp);
                continue;
            }
            let bytes = protocol::serialize_packet(protocol::PacketType::QuestView, &packet);
            if bytes.len() > crate::action_journal::MAX_PAGE_BYTES + 1 {
                events.abandon(sp);
                continue;
            }
            events.commit(sp, self.server.tick_counter);
            session.window_digest = digest;
            if let Some(record) = record {
                session.revision = record.revision;
            }
            sp.quest_session = packet.view.as_ref().map(|_| session);
            self.send_to_joined_slot(i, &bytes);
        }
    }
}
