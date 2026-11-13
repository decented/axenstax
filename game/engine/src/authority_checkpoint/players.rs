//! A player slot's server state in the checkpoint: every field the `players`
//! digest section reads, the identity a sidecar is keyed by, and nothing that
//! belongs to one connection.
//!
//! **Left out, so a restored joiner must re-attach (slice 3) before it acts:**
//! the action, pack and quest lanes, the window-event and block-view models, the
//! personal delivery state and the chat policy, all of which `ServerPlayer` calls
//! fresh per attach. The bound personal record comes back from the sidecar
//! (`restore::reload_sidecars`).
//!
//! **`riding` is left out too:** it names a hecs entity, and hecs ids change on
//! restore. A rider restores on its feet at the seat it was pinned to, its ride
//! state kept, which is the restoration ledger's one gap in `players`.

use super::codec::{Malformed, Reader, Writer};
use crate::combat::PlayerCombat;
use crate::server::{QueuedInput, ServerPlayer};

const SLOTS: usize = 36;
/// The widest server there is: the pipe's `Hello` names up to 255 joiners (the
/// `--max-players` clamp is lower), and a host adds its two local seats.
const MAX_SLOTS: usize = u8::MAX as usize + 2;

/// Every field of `ServerPlayer`, named once and marked: encoded by this module, or left
/// out with its reason. There is no `..`, so a field added to the struct doesn't compile
/// until someone has said which it is. A field this module doesn't encode comes back as
/// `ServerPlayer::new` made it, or as the sidecar reload sets it.
#[allow(dead_code)]
fn every_field_is_accounted_for(sp: &ServerPlayer) {
    let ServerPlayer {
        // left out: set again from the host's mode when the joiner re-attaches (`hosted_server.rs` at admission)
        inventory_mode: _,
        // left out: an action lane, drawn again per attach
        action_frontier: _,
        // left out: an action lane, drawn again per attach
        action_queue: _,
        // left out: an action lane, drawn again per attach
        action_results: _,
        // left out: a pack lane, drawn again per attach
        pack_session: _,
        // left out: a quest lane, drawn again per attach
        quest_session: _,
        // encoded
        quest_next_incarnation: _,
        // left out: set to the restored tick by `restore::reload_sidecars`
        gameplay_clock: _,
        // left out: a pack lane in flight, dropped with the connection
        pack_flight: _,
        // encoded
        pack_next_incarnation: _,
        // encoded
        pack_unfit_feedback: _,
        // encoded
        death_handled: _,
        // encoded (position, velocity, ground and water flags, fall state)
        player: _,
        // encoded (through the window section)
        inventory: _,
        // encoded
        combat: _,
        // encoded (through the window section)
        hotbar_slot: _,
        // encoded
        yaw: _,
        // encoded
        pitch: _,
        // encoded (through the window section)
        held_item: _,
        // encoded (through the window section)
        held_kind: _,
        // encoded (through the window section)
        held_id: _,
        // encoded
        pending_intent: _,
        // encoded
        intent_queue: _,
        // encoded
        step_credit: _,
        // encoded
        last_applied_input: _,
        // encoded
        server_simulated: _,
        // encoded
        connected: _,
        // encoded
        last_input_tick: _,
        // encoded
        last_sneak: _,
        // encoded
        last_acting: _,
        // encoded
        last_move_mag: _,
        // encoded
        skin_key: _,
        // encoded
        verified_pubkey: _,
        // encoded
        display_name: _,
        // left out: the chat policy, fresh per attach
        comms: _,
        // left out: the chat policy, fresh per attach
        contacts: _,
        // left out: a rate limiter, fresh per attach
        chat_rate: _,
        // encoded
        worldgen_mismatch: _,
        // encoded
        spawn_pos: _,
        // encoded
        dead_ticks: _,
        // encoded
        respawns: _,
        // encoded
        awaiting_join: _,
        // left out: set by the sidecar reload, which is what says the slot isn't saved yet
        awaiting_reattach: _,
        // encoded
        spawn_protection: _,
        // encoded
        armour_points: _,
        // encoded
        armour_wear_hits: _,
        // encoded
        interact_cooldown: _,
        // encoded
        attach_gen: _,
        // encoded
        next_swing_tick: _,
        // encoded
        next_shot_tick: _,
        // encoded
        fishing: _,
        // encoded
        drop_bucket: _,
        // left out: a log-only tally that refills on a restart (handoff section 5)
        possession: _,
        // left out: a tally that refills on a restart (handoff section 5)
        strict_refusals: _,
        // left out: a rate that refills on a restart (handoff section 5)
        strict_corrections: _,
        // encoded
        eat_cooldown: _,
        // encoded
        slept_night: _,
        // encoded
        progression_tracked: _,
        // left out: the bound personal record, which comes back from the sidecar
        personal: _,
        // left out: set by a read of the personal record at attach
        personal_read_failed: _,
        // left out: delivery state of the personal record, per attach
        #[cfg(not(target_arch = "wasm32"))]
        personal_delivery: _,
        // encoded
        progression_credits: _,
        // encoded (through the window section)
        armour: _,
        // encoded (through the window section, schema 7)
        offhand: _,
        // left out: presentation only, relayed from the next input
        held_tool: _,
        // left out: presentation only, relayed from the next input
        off_held: _,
        // encoded (through the window section)
        cursor: _,
        // encoded (through the window section)
        craft_grid: _,
        // encoded (through the window section)
        station: _,
        // encoded (through the window section)
        table_gone_ticks: _,
        // left out: window events, per attach
        window_events: _,
        // left out: the correction debt stays in the sidecar, which merges it back
        obligations: _,
        // encoded (through the window section)
        open_container: _,
        // left out: what the client was sent, per attach
        container_sent: _,
        // encoded (through the window section)
        last_window_op_seq: _,
        // left out: what the client was sent, per attach
        block_views: _,
        // left out: refusals waiting to be sent, dropped with the connection
        refused_uses: _,
        // left out: names a hecs entity, and hecs ids change on restore; the rider is put on its feet
        riding: _,
        // encoded (through the seat section)
        ride: _,
        // encoded (through the seat section)
        seat_input: _,
        // encoded (through the seat section)
        local_seated: _,
        // left out: the raised flag is re-sent by the client's next input and the server counts the raise itself
        shield_latch: _,
        // left out: a raise count, rebuilt from the latch (a restored body starts lowered)
        shield_ticks: _,
        // left out: a wear tally for the tick, drained into window events the same tick
        shield_wear: _,
        // left out: a log-once flag
        shield_flag_logged: _,
    } = sp;
}

pub(super) fn write_all(w: &mut Writer, players: &[ServerPlayer]) -> Result<(), String> {
    w.count(players.len());
    for sp in players {
        write_one(w, sp)?;
    }
    Ok(())
}

pub(super) fn read_all(r: &mut Reader<'_>) -> Result<Vec<ServerPlayer>, Malformed> {
    // A slot takes at least a body's worth of bytes.
    let n = r.count(64)?;
    if n > MAX_SLOTS {
        return Err(Malformed("more players than a server holds"));
    }
    (0..n).map(|_| read_one(r)).collect()
}

fn write_one(w: &mut Writer, sp: &ServerPlayer) -> Result<(), String> {
    write_body(w, sp);
    write_combat(w, &sp.combat);
    write_session(w, sp);
    write_window(w, sp)?;
    write_seat(w, sp);
    Ok(())
}

fn read_one(r: &mut Reader<'_>) -> Result<ServerPlayer, Malformed> {
    let mut sp = ServerPlayer::new(glam::Vec3::ZERO);
    read_body(r, &mut sp)?;
    read_combat(r, &mut sp.combat)?;
    read_session(r, &mut sp)?;
    read_window(r, &mut sp)?;
    read_seat(r, &mut sp)?;
    Ok(sp)
}

fn write_body(w: &mut Writer, sp: &ServerPlayer) {
    let p = &sp.player;
    w.vec3(p.pos);
    w.vec3(p.velocity);
    w.bool(p.on_ground);
    w.bool(p.flying);
    w.bool(p.in_water);
    w.f32(p.sprint_boots_mult);
    w.f32(p.fall_distance);
    w.opt(p.pending_landing, |w, l| {
        w.f32(l.fall_distance);
        w.u16(l.landing_block);
    });
    w.f32(sp.yaw);
    w.f32(sp.pitch);
    w.vec3(sp.spawn_pos);
}

fn read_body(r: &mut Reader<'_>, sp: &mut ServerPlayer) -> Result<(), Malformed> {
    let p = &mut sp.player;
    p.pos = r.vec3()?;
    p.velocity = r.vec3()?;
    p.on_ground = r.bool()?;
    p.flying = r.bool()?;
    p.in_water = r.bool()?;
    p.sprint_boots_mult = r.f32()?;
    p.fall_distance = r.f32()?;
    p.pending_landing = r.opt(|r| Ok(crate::physics::Landing { fall_distance: r.f32()?, landing_block: r.u16()? }))?;
    sp.yaw = r.f32()?;
    sp.pitch = r.f32()?;
    sp.spawn_pos = r.vec3()?;
    Ok(())
}

fn write_combat(w: &mut Writer, c: &PlayerCombat) {
    w.f32(c.health);
    w.f32(c.max_health);
    w.u32(c.attack_cooldown);
    w.u32(c.flash_timer);
    w.u32(c.invincible_timer);
    w.f32(c.last_hit_damage);
    w.bool(c.dead);
    w.bool(c.respawn_requested);
    w.bool(c.just_died);
    w.u32(c.poison_ticks);
    w.u8(c.hunger);
    w.u8(c.max_hunger);
    w.u32(c.hunger_drain_ticks);
    w.u32(c.regen_ticks);
    w.u32(c.starvation_ticks);
    w.f32(c.starvation_floor);
    write_damage_cause(w, c.last_damage);
    w.u16(c.breath.air);
    w.u16(c.breath.drown_ticks);
    w.bool(c.breath.underwater);
}

fn read_combat(r: &mut Reader<'_>, c: &mut PlayerCombat) -> Result<(), Malformed> {
    c.health = r.f32()?;
    c.max_health = r.f32()?;
    c.attack_cooldown = r.u32()?;
    c.flash_timer = r.u32()?;
    c.invincible_timer = r.u32()?;
    c.last_hit_damage = r.f32()?;
    c.dead = r.bool()?;
    c.respawn_requested = r.bool()?;
    c.just_died = r.bool()?;
    c.poison_ticks = r.u32()?;
    c.hunger = r.u8()?;
    c.max_hunger = r.u8()?;
    c.hunger_drain_ticks = r.u32()?;
    c.regen_ticks = r.u32()?;
    c.starvation_ticks = r.u32()?;
    c.starvation_floor = r.f32()?;
    c.last_damage = read_damage_cause(r)?;
    c.breath = crate::survival::Breath { air: r.u16()?, drown_ticks: r.u16()?, underwater: r.bool()? };
    Ok(())
}

fn write_damage_cause(w: &mut Writer, cause: crate::survival::DamageCause) {
    use crate::survival::DamageCause as C;
    match cause {
        C::Generic => w.u8(0),
        C::Fall => w.u8(1),
        C::Drowning => w.u8(2),
        C::Starvation => w.u8(3),
        C::Lava => w.u8(4),
        C::Fire => w.u8(5),
        C::Explosion => w.u8(6),
        C::Mob(kind) => {
            w.u8(7);
            // `MobType` holds no float and no container: a fixed encoding.
            w.blob(&bincode::serialize(&kind).expect("a mob kind encodes"));
        }
    }
}

fn read_damage_cause(r: &mut Reader<'_>) -> Result<crate::survival::DamageCause, Malformed> {
    use crate::survival::DamageCause as C;
    Ok(match r.u8()? {
        0 => C::Generic,
        1 => C::Fall,
        2 => C::Drowning,
        3 => C::Starvation,
        4 => C::Lava,
        5 => C::Fire,
        6 => C::Explosion,
        7 => C::Mob(bincode::deserialize(r.blob()?).map_err(|_| Malformed("an unknown mob kind"))?),
        _ => return Err(Malformed("an unknown damage cause")),
    })
}

/// Identity, timers, the input lane and the progression counters.
fn write_session(w: &mut Writer, sp: &ServerPlayer) {
    w.opt(sp.verified_pubkey, |w, pk| w.raw(&pk));
    w.str(&sp.display_name);
    w.u64(sp.skin_key);
    w.bool(sp.worldgen_mismatch);
    w.bool(sp.server_simulated);
    w.bool(sp.connected);
    w.bool(sp.awaiting_join);
    w.bool(sp.death_handled);
    w.u32(sp.dead_ticks);
    w.u32(sp.respawns);
    w.u32(sp.spawn_protection);
    w.u8(sp.armour_points);
    w.u8(sp.armour_wear_hits);
    w.u32(sp.interact_cooldown);
    w.u32(sp.eat_cooldown);
    w.u32(sp.attach_gen);
    w.u64(sp.next_swing_tick);
    w.u64(sp.next_shot_tick);
    w.opt(sp.fishing, |w, at| w.u64(at));
    let (tokens, since, quarters, last_read, allowance, last_credited) = sp.drop_bucket.to_parts();
    w.u8(tokens);
    w.u64(since);
    w.u8(quarters);
    w.opt(last_read, |w, at| w.u64(at));
    w.u64(allowance);
    w.u64(last_credited);
    w.opt(sp.slept_night, |w, night| w.u32(night));

    w.u64(sp.last_applied_input);
    w.u64(sp.last_input_tick);
    w.u32(sp.step_credit);
    w.bool(sp.last_sneak);
    w.bool(sp.last_acting);
    w.f32(sp.last_move_mag);
    w.opt(sp.pending_intent.as_ref(), write_input);
    w.count(sp.intent_queue.len());
    for input in &sp.intent_queue {
        write_input(w, input);
    }

    w.bool(sp.progression_tracked);
    w.u32(sp.progression_credits);
    w.u64(sp.quest_next_incarnation);
    w.u64(sp.pack_next_incarnation);
    w.count(sp.pack_unfit_feedback.len());
    for &(id, count) in &sp.pack_unfit_feedback {
        w.u32(id);
        w.u8(count);
    }
}

fn read_session(r: &mut Reader<'_>, sp: &mut ServerPlayer) -> Result<(), Malformed> {
    sp.verified_pubkey = r.opt(|r| r.bytes32())?;
    sp.display_name = r.string()?;
    sp.skin_key = r.u64()?;
    sp.worldgen_mismatch = r.bool()?;
    sp.server_simulated = r.bool()?;
    sp.connected = r.bool()?;
    sp.awaiting_join = r.bool()?;
    sp.death_handled = r.bool()?;
    sp.dead_ticks = r.u32()?;
    sp.respawns = r.u32()?;
    sp.spawn_protection = r.u32()?;
    sp.armour_points = r.u8()?;
    sp.armour_wear_hits = r.u8()?;
    sp.interact_cooldown = r.u32()?;
    sp.eat_cooldown = r.u32()?;
    sp.attach_gen = r.u32()?;
    sp.next_swing_tick = r.u64()?;
    sp.next_shot_tick = r.u64()?;
    sp.fishing = r.opt(|r| r.u64())?;
    let tokens = r.u8()?;
    let since = r.u64()?;
    let quarters = r.u8()?;
    let last_read = r.opt(|r| r.u64())?;
    let allowance = r.u64()?;
    let last_credited = r.u64()?;
    sp.drop_bucket = crate::item_actions::DropBucket::from_parts((tokens, since, quarters, last_read, allowance, last_credited));
    sp.slept_night = r.opt(|r| r.u32())?;

    sp.last_applied_input = r.u64()?;
    sp.last_input_tick = r.u64()?;
    sp.step_credit = r.u32()?;
    sp.last_sneak = r.bool()?;
    sp.last_acting = r.bool()?;
    sp.last_move_mag = r.f32()?;
    sp.pending_intent = r.opt(read_input)?;
    let queued = r.count(INPUT_BYTES)?;
    sp.intent_queue = (0..queued).map(|_| read_input(r)).collect::<Result<_, _>>()?;

    sp.progression_tracked = r.bool()?;
    sp.progression_credits = r.u32()?;
    sp.quest_next_incarnation = r.u64()?;
    sp.pack_next_incarnation = r.u64()?;
    let feedback = r.count(5)?;
    sp.pack_unfit_feedback = (0..feedback).map(|_| Ok((r.u32()?, r.u8()?))).collect::<Result<_, _>>()?;
    Ok(())
}

/// The least an encoded input takes (no hotbar choice).
const INPUT_BYTES: usize = 8 + 4 + 4 + 4 + 4 + 4 + 8 + 8 + 2 + 1 + 4;

/// Which of an intent's flags a server reads; the rest are client-local and
/// never set on an input built from a packet (`PlayerIntent::from_input_packet`).
fn intent_flags(i: &crate::player_intent::PlayerIntent) -> [bool; 11] {
    [
        i.sprint,
        i.sneak,
        i.jump_held,
        i.jump_pressed,
        i.toggle_flight,
        i.break_block,
        i.place_block,
        i.toggle_inventory,
        i.drop_item,
        i.pause,
        i.cursor_captured,
    ]
}

fn write_input(w: &mut Writer, input: &QueuedInput) {
    w.u64(input.seq);
    w.f32(input.yaw);
    w.f32(input.pitch);
    w.f32(input.health_delta);
    let i = &input.intent;
    w.f32(i.move_forward);
    w.f32(i.move_right);
    w.f64(i.look_dx);
    w.f64(i.look_dy);
    let mut flags = 0u16;
    for (bit, on) in intent_flags(i).into_iter().enumerate() {
        flags |= u16::from(on) << bit;
    }
    w.u16(flags);
    w.opt(i.hotbar_select, |w, slot| w.u32(slot as u32));
    w.f32(i.scroll_delta);
}

fn read_input(r: &mut Reader<'_>) -> Result<QueuedInput, Malformed> {
    let seq = r.u64()?;
    let yaw = r.f32()?;
    let pitch = r.f32()?;
    let health_delta = r.f32()?;
    let move_forward = r.f32()?;
    let move_right = r.f32()?;
    let look_dx = r.f64()?;
    let look_dy = r.f64()?;
    let flags = r.u16()?;
    if flags >> intent_flags(&Default::default()).len() != 0 {
        return Err(Malformed("an unknown input flag"));
    }
    let on = |bit: u16| flags & (1 << bit) != 0;
    let hotbar_select = r.opt(|r| Ok(r.u32()? as usize))?;
    let scroll_delta = r.f32()?;
    let intent = crate::player_intent::PlayerIntent {
        move_forward,
        move_right,
        look_dx,
        look_dy,
        sprint: on(0),
        sneak: on(1),
        jump_held: on(2),
        jump_pressed: on(3),
        toggle_flight: on(4),
        break_block: on(5),
        place_block: on(6),
        toggle_inventory: on(7),
        drop_item: on(8),
        pause: on(9),
        cursor_captured: on(10),
        hotbar_select,
        scroll_delta,
        ..Default::default()
    };
    Ok(QueuedInput { seq, yaw, pitch, intent, health_delta })
}

/// One stack: bincode, the save's own item encoding (no float, no hash
/// container), in a length-prefixed blob.
fn write_stack(w: &mut Writer, stack: Option<&crate::item::ItemStack>) -> Result<(), String> {
    match stack {
        None => w.u8(0),
        Some(stack) => {
            w.u8(1);
            w.blob(&bincode::serialize(stack).map_err(|e| e.to_string())?);
        }
    }
    Ok(())
}

fn read_stack(r: &mut Reader<'_>) -> Result<Option<crate::item::ItemStack>, Malformed> {
    r.opt(|r| bincode::deserialize(r.blob()?).map_err(|_| Malformed("a stack that doesn't decode")))
}

/// The server's copy of a joiner's window.
fn write_window(w: &mut Writer, sp: &ServerPlayer) -> Result<(), String> {
    w.u32(sp.hotbar_slot as u32);
    w.u16(sp.held_item);
    w.u8(sp.held_kind);
    w.u16(sp.held_id);
    for slot in sp.inventory.slots_iter() {
        write_stack(w, slot)?;
    }
    for piece in &sp.armour {
        w.opt(piece.as_ref(), |w, piece| {
            w.blob(&bincode::serialize(piece).expect("an armour piece encodes"));
        });
    }
    write_stack(w, sp.offhand.as_ref())?;
    write_stack(w, sp.cursor.as_ref())?;
    for stack in sp.craft_grid.iter().flatten() {
        write_stack(w, stack.as_ref())?;
    }
    match sp.station {
        crate::window::Station::Player => w.u8(0),
        crate::window::Station::Table { cell } => {
            w.u8(1);
            w.pos((cell[0], cell[1], cell[2]));
        }
    }
    w.opt(sp.table_gone_ticks, |w, ticks| w.u8(ticks));
    w.opt(sp.open_container, |w, cell| w.pos((cell[0], cell[1], cell[2])));
    w.u32(sp.last_window_op_seq);
    Ok(())
}

fn read_window(r: &mut Reader<'_>, sp: &mut ServerPlayer) -> Result<(), Malformed> {
    sp.hotbar_slot = r.u32()? as usize;
    sp.held_item = r.u16()?;
    sp.held_kind = r.u8()?;
    sp.held_id = r.u16()?;
    for slot in 0..SLOTS {
        sp.inventory.set_slot(slot, read_stack(r)?);
    }
    for piece in &mut sp.armour {
        *piece = r.opt(|r| bincode::deserialize(r.blob()?).map_err(|_| Malformed("an armour piece that doesn't decode")))?;
    }
    sp.offhand = read_stack(r)?;
    sp.cursor = read_stack(r)?;
    for stack in sp.craft_grid.iter_mut().flatten() {
        *stack = read_stack(r)?;
    }
    sp.station = match r.u8()? {
        0 => crate::window::Station::Player,
        1 => {
            let (x, y, z) = r.pos()?;
            crate::window::Station::Table { cell: [x, y, z] }
        }
        _ => return Err(Malformed("an unknown station")),
    };
    sp.table_gone_ticks = r.opt(|r| r.u8())?;
    sp.open_container = r.opt(|r| {
        let (x, y, z) = r.pos()?;
        Ok([x, y, z])
    })?;
    sp.last_window_op_seq = r.u32()?;
    Ok(())
}

fn write_seat(w: &mut Writer, sp: &ServerPlayer) {
    w.f32(sp.ride.speed);
    w.f32(sp.ride.heading.x);
    w.f32(sp.ride.heading.y);
    w.f32(sp.seat_input.forward);
    w.f32(sp.seat_input.right);
    w.f32(sp.seat_input.yaw);
    w.bool(sp.seat_input.sneak);
    w.bool(sp.seat_input.jump);
    w.bool(sp.local_seated);
}

fn read_seat(r: &mut Reader<'_>, sp: &mut ServerPlayer) -> Result<(), Malformed> {
    sp.ride.speed = r.f32()?;
    sp.ride.heading = glam::Vec2::new(r.f32()?, r.f32()?);
    sp.seat_input.forward = r.f32()?;
    sp.seat_input.right = r.f32()?;
    sp.seat_input.yaw = r.f32()?;
    sp.seat_input.sneak = r.bool()?;
    sp.seat_input.jump = r.bool()?;
    sp.local_seated = r.bool()?;
    Ok(())
}
