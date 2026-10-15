//! Genuine native clock ownership and signed physical/personal delivery.
use super::*;
use crate::{
    action_identity::InventoryMode,
    action_wire::ActionIntent,
    hosted_server::{HostWorld, HostedServer, RemoteTransport, quest_lifecycle_actor::Actor},
    progression::GameplayRecord,
    protocol,
    transport::ClientTransport as _,
};

pub(super) const VILLAGE: crate::reputation::VillageId = (14, 15);
pub(super) const SECRET: [u8; 32] = [101; 32];
pub(super) fn score(record: &GameplayRecord) -> i16 {
    record.reputation.iter().find(|(id, _)| *id == VILLAGE).map_or(0, |(_, n)| *n)
}

pub(super) struct DayRig {
    pub game: HeadlessGame,
    pub actor: Option<Actor>,
    pub units: u64,
    pub _root: crate::save::WorldsRootGuard,
}
impl DayRig {
    pub fn new(name: &str) -> Self {
        let root = crate::save::WorldsRootGuard::new(name);
        let mut game = HeadlessGame::boot_into_world(name);
        clear_pad(&mut game, None);
        game.state.difficulty = "peaceful".into();
        assert!(!game.state.joined());
        assert!(game.state.hosted_server.is_none());
        assert_eq!(game.state.world_time_step, 1);
        let units = game.state.tick_counter;
        assert!(units < 100, "fresh load has barely started");
        assert_eq!(game.state.world_time, 7500 + units as u32);
        assert_eq!(u64::from(game.state.reputation_day.remainder()), units);
        game.state.prepare_local_gameplay().unwrap();
        let (selected, initial) = super::quest_preparation::selected_record(&crate::save::world_dir(name));
        let binding = game.state.players[0].personal_binding.as_ref().unwrap();
        assert_eq!(initial.context, binding.context());
        assert_eq!(selected.quest_world.unwrap().scope, game.state.world.quest_scope.unwrap());
        assert!(game.state.local_personal.as_ref() == Some(binding));
        Self { game, actor: None, units, _root: root }
    }
    pub fn native(&self) -> GameplayRecord {
        let slot = &self.game.state.players[0];
        GameplayRecord::from_slot(slot.personal_binding.as_ref().unwrap().context(), slot).unwrap()
    }
    pub fn remote(&self) -> GameplayRecord {
        let host = self.game.state.hosted_server.as_ref().unwrap();
        let sp = &host.server.players[self.actor.as_ref().unwrap().i()];
        sp.gameplay_clock.project(&sp.personal.as_ref().unwrap().record().unwrap(), host.server.tick_counter).0
    }
    pub fn command(&mut self, line: &str) {
        let s = &mut self.game.state;
        let before = (s.tick_counter, s.world.total_ticks, s.reputation_day.remainder());
        let mut log = Vec::new();
        let (mut cheats, mut creative, mut broken) = (false, false, false);
        let mut ctx = crate::commands::CommandContext {
            world: &mut s.world,
            world_time: &mut s.world_time,
            world_time_step: &mut s.world_time_step,
            is_creative: &mut s.is_creative,
            play_mode: &mut s.play_mode,
            seed: s.biome_gen.seed,
            world_name: &s.world_name,
            players: &mut s.players,
            player_idx: 0,
            op_level: crate::world_exit::local_command_op_level(s.remote_client.is_some()),
            current_tick: s.tick_counter,
            log: &mut log,
            registry: &s.cmd_registry,
            cheats_used_marker: &mut cheats,
            ever_creative_marker: &mut creative,
            pure_survival_broken_marker: &mut broken,
        };
        assert_eq!(crate::commands::dispatch(line, &mut ctx, &s.cmd_registry), crate::commands::CommandResult::Success);
        assert_eq!(
            (s.tick_counter, s.world.total_ticks, s.reputation_day.remainder()),
            before,
            "actual command dispatch cannot invent an advance"
        );
    }
    pub fn input(&mut self) {
        if let Some(actor) = &mut self.actor {
            actor.input_seq += 1;
            let at = actor.slot.player.pos;
            actor.client.send_to_server(&protocol::serialize_packet(
                protocol::PacketType::ClientInput,
                &protocol::InputPacket {
                    tick: actor.input_seq,
                    x: at.x,
                    y: at.y,
                    z: at.z,
                    yaw: std::f32::consts::PI,
                    health: actor.slot.combat.health,
                    events_applied: actor.applied,
                    ..Default::default()
                },
            ));
        }
    }
    pub fn drain(&mut self) {
        if let Some(actor) = &mut self.actor {
            let host = self.game.state.hosted_server.as_ref().unwrap();
            actor.drain(&host.server.registry);
            assert!(actor.rejects.is_empty());
            assert!(!host.disconnected[actor.i()]);
        }
    }
    /// Observe the actual clock and normal hosted window. A standalone preflight
    /// refusal returns from native simulation early; an empty no-host report
    /// does not prove that partial native simulation completed.
    pub fn step(&mut self) -> Option<crate::reputation_day::DayOffer> {
        self.input();
        let s = &self.game.state;
        assert!(matches!(s.mode, GameMode::Playing));
        assert!(!s.joined() && !s.handover_holds_play());
        let (tick, phase, total, step, tally) =
            (s.tick_counter, s.world_time, s.world.total_ticks, s.world_time_step, s.world.sim_tally);
        let native_owner = s.reputation_day.owner();
        let owned = s
            .hosted_server
            .as_ref()
            .filter(|h| !h.lends_host_world())
            .map(|h| (h.server.tick_counter, h.server.reputation_day.remainder()));
        self.game.state.tick();
        let offer = self.game.state.reputation_day_offer;
        self.game.state.network_send_input();
        let report = self.game.state.tick_hosted_server(tally);
        assert!(report.faults.is_empty(), "normal hosted tally (empty without a host): {:?}", report.faults);
        self.game.state.finish_reputation_day_tick();
        let s = &self.game.state;
        assert_eq!(s.tick_counter, tick + 1);
        assert_eq!(s.world.total_ticks, total + 1, "actual ActiveTicks");
        assert_eq!(s.world_time, (phase + step) % 24_000, "actual raw solar step");
        assert_eq!(s.reputation_day.owner(), native_owner);
        self.units += u64::from(step);
        assert_eq!(u64::from(s.reputation_day.remainder()), self.units % 24_000);
        assert!(s.reputation_day_offer.is_none(), "completed offer cannot survive its window");
        if let Some((old_tick, old_remainder)) = owned {
            let server = &s.hosted_server.as_ref().unwrap().server;
            assert_eq!(server.tick_counter, old_tick + 1);
            assert_eq!(server.world_time, (s.world_time + 1) % 24_000);
            assert_eq!(
                server.reputation_day.remainder(),
                (old_remainder + 1) % 24_000,
                "no-lend contributes its actual owning +1"
            );
        }
        self.drain();
        offer
    }
    pub fn ticks(&mut self, count: u32) {
        for _ in 0..count {
            self.step();
        }
    }
    pub fn to_units(&mut self, target: u64) {
        assert!(self.units <= target);
        while self.units < target {
            assert!(
                self.units + u64::from(self.game.state.world_time_step) <= target,
                "choose a real speed that reaches the exact boundary"
            );
            self.step();
        }
    }
    pub fn quiesce(&mut self) {
        for _ in 0..24 {
            self.step();
            if let Some(actor) = &self.actor {
                let host = self.game.state.hosted_server.as_ref().unwrap();
                let sp = &host.server.players[actor.i()];
                if actor.idle()
                    && sp.window_events.is_quiescent()
                    && sp.action_results.as_ref().is_none_or(|r| r.is_quiescent())
                    && sp.pack_flight.is_none()
                    && sp.personal_delivery.quiescent(sp.personal.as_ref().unwrap(), sp.respawns)
                {
                    return;
                }
            } else {
                return;
            }
        }
        panic!("actual physical carriers and returned current ACK did not settle");
    }
    pub fn start_host(&mut self, mode: HostWorld) {
        assert!(self.actor.is_none());
        let s = &mut self.game.state;
        let mut host = HostedServer::start_host(
            1,
            s.world_name.clone(),
            s.biome_gen.seed,
            0,
            RemoteTransport::WebSocket { port: 0 },
            mode,
        )
        .unwrap();
        if host.lends_host_world() {
            host.select_inventory_mode(InventoryMode::Strict).unwrap();
        } else {
            assert_eq!(host.inventory_mode(), InventoryMode::Mirror, "actual no-lend selector refuses Strict");
        }
        host.server.column_streamer = None;
        host.server.column_refill_per_tick = 0;
        host.server.difficulty = crate::survival::Difficulty::Peaceful;
        if !host.lends_host_world() {
            clear_pad(&mut self.game, Some(&mut host));
        }
        self.game.state.hosted_server = Some(host);
        self.step();
    }
    pub fn connect(&mut self) {
        let host = self.game.state.hosted_server.as_mut().unwrap();
        let client = host.attach_test_remote_with_binding(Some([102; 32]));
        crate::hosted_server::tests::send_signed_join(&client, SECRET, Some([102; 32]));
        self.step();
        let bytes = client.try_recv_from_server().expect("real signed accept before physical carriers");
        let (kind, body) = protocol::deserialize_header(&bytes).unwrap();
        assert_eq!(kind, protocol::PacketType::JoinAccept);
        let accepted: protocol::JoinAcceptPacket = protocol::safe_deserialize(body).unwrap();
        assert_eq!(accepted.inventory_mode, self.game.state.hosted_server.as_ref().unwrap().inventory_mode());
        let slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
        let inbox = crate::personal_sync::Inbox::new(accepted.action_session, accepted.action_life);
        let mut actor = Actor::new(client, accepted, slot, inbox, 0);
        let host = self.game.state.hosted_server.as_mut().unwrap();
        actor.drain(&host.server.registry);
        assert_eq!(actor.applied, 2, "real initial Sync1 then Personal2");
        let key = nostr::Keys::new(nostr::SecretKey::from_slice(&SECRET).unwrap()).public_key().to_bytes();
        let sp = &mut host.server.players[actor.i()];
        assert_eq!(sp.verified_pubkey, Some(key));
        assert!(sp.server_simulated && sp.is_in_world());
        let at = self.game.state.players[0].player.pos + glam::Vec3::X * 2.0;
        sp.player.pos = at;
        sp.player.velocity = glam::Vec3::ZERO;
        sp.player.on_ground = true;
        sp.player.reset_fall();
        actor.slot.player.pos = at;
        self.actor = Some(actor);
        self.quiesce();
    }
    pub fn kill_native(&mut self) {
        let before = self.native();
        assert_eq!(before.quests.penalty_left, 0, "a genuine expired penalty cooldown");
        let old_score = score(&before);
        let old_kills =
            self.game.state.players[0].kill_counter.get(&crate::mob::MobType::Villager).copied().unwrap_or(0);
        let s = &mut self.game.state;
        let pos = s.players[0].player.pos + glam::Vec3::X;
        s.world.village_anchors.insert(VILLAGE, pos.to_array().map(|v| v.floor() as i32));
        let entity = crate::entity::spawn_mob(&mut s.ecs, crate::mob::MobType::Villager, pos);
        s.ecs.get::<&mut crate::combat::Health>(entity).unwrap().current = 1.0;
        let slot = &s.players[0];
        let eye = slot.player.eye_pos();
        let damage = slot.inventory.hotbar_attack_damage(slot.hotbar_slot);
        let owner = crate::tameable::local_owner_key(0);
        assert!(crate::combat::strike(
            &mut s.ecs,
            entity,
            &crate::combat::Swing {
                attacker: crate::combat::Attacker::Local(0),
                owner_key: &owner,
                eye,
                look_dir: (pos - eye).normalize(),
                crit: false,
                sprinting: false,
                sneaking: false,
                base_damage: damage,
            }
        ));
        self.step();
        assert!(!self.game.state.ecs.contains(entity));
        assert_eq!(score(&self.native()), (old_score - 25).max(-100), "actual combat and attributed corpse sweep");
        assert_eq!(self.game.state.players[0].kill_counter[&crate::mob::MobType::Villager], old_kills + 1);
        assert_eq!(self.native().quests.penalty_left, 600);
    }
    pub fn kill_remote(&mut self) {
        self.quiesce();
        let actor = self.actor.as_ref().unwrap();
        let at = actor.slot.player.pos + glam::Vec3::Z * 2.5;
        let host = self.game.state.hosted_server.as_mut().unwrap();
        let lent = host.lends_host_world();
        let (world, ecs) = if lent {
            (&mut self.game.state.world, &mut self.game.state.ecs)
        } else {
            (&mut host.server.world, &mut host.server.ecs)
        };
        world.village_anchors.insert(VILLAGE, at.to_array().map(|v| v.floor() as i32));
        let entity = crate::entity::spawn_mob(ecs, crate::mob::MobType::Villager, at);
        self.step();
        let host = self.game.state.hosted_server.as_mut().unwrap();
        let ecs = if lent { &mut self.game.state.ecs } else { &mut host.server.ecs };
        ecs.get::<&mut crate::entity::Position>(entity).unwrap().0 = at;
        ecs.get::<&mut crate::combat::Health>(entity).unwrap().current = 1.0;
        let id = ecs.get::<&crate::entity::ProtocolId>(entity).unwrap().0;
        let actor = self.actor.as_mut().unwrap();
        assert!(actor.slot.inventory.slot(8).is_none());
        actor.legacy_seq += 1;
        let request = protocol::EntityAttackPacket {
            seq: actor.legacy_seq,
            entity: id,
            held_kind: protocol::item_kind::EMPTY,
            held_id: 0,
            held_full: protocol::WireItem::None,
            sprint: false,
            sneak: false,
            hotbar_slot: 8,
            events_applied: actor.applied,
        };
        if actor.accepted.inventory_mode == InventoryMode::Strict {
            actor.send(ActionIntent::Attack(request));
        } else {
            actor.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::EntityAttack, &request));
        }
        self.step();
        let actor = self.actor.as_ref().unwrap();
        assert!(actor.outcomes.iter().any(|out| out.entity == id && out.accepted));
        let ecs = if lent { &self.game.state.ecs } else { &self.game.state.hosted_server.as_ref().unwrap().server.ecs };
        assert!(!ecs.contains(entity), "actual signed attack was attributed and swept");
        self.quiesce();
        assert_eq!(score(&self.remote()), -25);
        assert_eq!(actor_score(self.actor.as_ref().unwrap()), -25);
    }
    /// Replay an observed offer through the actual lending consumer, without a native advance.
    pub fn repeat_window(&mut self, offer: Option<crate::reputation_day::DayOffer>) {
        self.repeat_clock(self.observed_clock(offer));
    }
    pub fn observed_clock(&self, offer: Option<crate::reputation_day::DayOffer>) -> crate::sim_lend::HostClock {
        let s = &self.game.state;
        crate::sim_lend::HostClock {
            world_time: s.world_time,
            tick_counter: s.tick_counter,
            weather: crate::weather::Weather { rain_until: s.weather_rain_until, storm_until: s.weather_storm_until },
            reputation_owner: s.reputation_day.owner(),
            reputation_offer: offer,
        }
    }
    /// Isolated negative day-event replay seam: an extra server window without
    /// a native advance, excluding whole-simulation once-per-tick credit.
    pub fn repeat_clock(&mut self, clock: crate::sim_lend::HostClock) {
        let s = &mut self.game.state;
        let parts = crate::sim_lend::SimParts {
            reputation_owner: s.reputation_day.owner(),
            world: &mut s.world,
            ecs: &mut s.ecs,
            water: &mut s.water,
            lava: &mut s.lava,
            fire: &mut s.fire,
            leaf_decay: &mut s.leaf_decay,
            loaded_columns: &mut s.loaded_columns,
        };
        crate::sim_lend::LentSim::lend(s.hosted_server.as_mut().unwrap(), parts, clock).tick();
        self.drain();
    }
}
pub(super) fn actor_score(actor: &Actor) -> i16 {
    actor.slot.reputation.score(VILLAGE)
}

pub(super) fn trace(rig: &DayRig, label: &str) {
    let s = &rig.game.state;
    eprintln!(
        "day-authority {label} tick={} raw={} effective={} step={} ActiveTicks={} natural_units={} remainder={} native_score={} native_revision={} penalty={}",
        s.tick_counter,
        s.world_time,
        s.world.effective_world_time(s.world_time),
        s.world_time_step,
        s.world.total_ticks,
        rig.units,
        s.reputation_day.remainder(),
        score(&rig.native()),
        rig.native().revision,
        rig.native().quests.penalty_left
    );
    if let Some(actor) = &rig.actor {
        let host = s.hosted_server.as_ref().unwrap();
        let sp = &host.server.players[actor.i()];
        eprintln!(
            "day-authority remote tick={} raw={} own_remainder={} score={} revision={} physical={} initialised={} current_ack={} server_simulated={} in_world={}",
            host.server.tick_counter,
            host.server.world_time,
            host.server.reputation_day.remainder(),
            score(&rig.remote()),
            rig.remote().revision,
            actor.applied,
            sp.personal_delivery.initialised,
            sp.personal_delivery.quiescent(sp.personal.as_ref().unwrap(), sp.respawns),
            sp.server_simulated,
            sp.is_in_world()
        );
    }
}

/// Use the real supported commands for a bounded tail to an exact units boundary.
pub(super) fn fast_to(rig: &mut DayRig, target: u64) {
    rig.command("/time speed 64");
    let count = (target - rig.units) / 64;
    assert!(count < 800, "a bounded accelerated acceptance tail");
    rig.ticks(count as u32);
    rig.command("/time speed 1");
    rig.to_units(target);
}

/// Deliberately refuse private-body storage at the real first autosave/day cut.
pub(super) fn private_body_refusal_day(rig: &mut DayRig) {
    assert!(rig.game.state.hosted_server.is_none());
    assert_eq!(rig.units, 48_000);
    let tick = rig.game.state.tick_counter;
    assert!(tick < 2500);
    let extra = 24_000 - 4 * (6000 - tick);
    let adjustment = extra % 3;
    if adjustment != 0 {
        rig.command(&format!("/time speed {}", 4 + adjustment));
        rig.step();
    }
    rig.command("/time speed 7");
    rig.ticks(((extra - adjustment) / 3) as u32);
    rig.command("/time speed 4");
    while rig.game.state.tick_counter < 5999 {
        rig.step();
    }
    assert_eq!(rig.units, 71_996);
    let mut body = crate::plan::PlanData::debug_3x3_stone();
    body.name = "x".repeat(crate::action_capacity::MAX_PRIVATE_BODY_BYTES + 1);
    let body_bytes = bincode::serialize(&body).unwrap();
    assert!(body_bytes.len() > crate::action_capacity::MAX_PRIVATE_BODY_BYTES);
    let pointers = (body.name.as_ptr(), body.cells.as_ptr());
    let previous = rig.game.state.players[0]
        .crafting_ui
        .cursor_item
        .replace(crate::item::ItemStack { item: crate::item::Item::Plan(body), count: 1 });
    let directory = crate::save::world_dir(&rig.game.state.world_name);
    let files = || {
        ["world.dat", "world_meta.json", "autosave/world.dat", "autosave/world_meta.json"]
            .map(|name| std::fs::read(directory.join(name)).ok())
    };
    let selected = files();
    let before = rig.native();
    before.encode().unwrap(); // The common progression remains valid throughout.
    let error = rig.game.state.preflight_personal_local().unwrap_err();
    assert!(error.contains("Plan body exceeds the store limit"), "actual checked-bound refusal: {error}");
    let owner = rig.game.state.reputation_day.owner();
    assert!(rig.game.state.hosted_server.is_none());
    // Actual Clock then native decay then the real autosave preflight RETURN.
    // This standalone error path is partial native simulation, with no lend.
    rig.step();
    assert_eq!(rig.game.state.tick_counter, 6000);
    assert_eq!(rig.game.state.autosave_counter, 0, "genuine autosave boundary was reached");
    assert_eq!(rig.units, 72_000);
    assert_eq!(rig.game.state.reputation_day.owner(), owner);
    assert_eq!(rig.game.state.reputation_day.remainder(), 0);
    assert_eq!(
        score(&rig.native()),
        score(&before) + 1,
        "OWN: private-body preflight return cannot lose the genuine native day"
    );
    assert_eq!(rig.native().revision, before.revision + 1);
    assert_eq!(files(), selected, "checked preflight wrote no selected or autosave files");
    let held = rig.game.state.players[0].crafting_ui.cursor_item.as_ref().unwrap();
    let crate::item::Item::Plan(retained) = &held.item else { panic!("private Plan custody was lost") };
    assert_eq!(held.count, 1);
    assert_eq!((retained.name.as_ptr(), retained.cells.as_ptr()), pointers);
    assert_eq!(bincode::serialize(retained).unwrap(), body_bytes, "complete private body survives byte-for-byte");
    assert!(rig.game.state.save_live_world().is_err());
    assert_eq!(files(), selected);
    rig.game.state.players[0].crafting_ui.cursor_item = previous;
    let settled = rig.native();
    rig.game.state.save_live_world().unwrap();
    let (_, saved) = super::quest_preparation::selected_record(&directory);
    assert_eq!(saved, settled, "honest retry saves the actual once-decayed holder");
    assert_eq!(rig.units, 72_000, "retry is no natural advance");
    rig.step();
    assert_eq!(rig.units, 72_004);
    assert_eq!(rig.native().revision, settled.revision);
    assert_eq!(score(&rig.native()), score(&settled));
    trace(rig, "native-private-body-preflight-return");
}
