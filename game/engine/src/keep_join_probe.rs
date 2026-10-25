//! A headless join probe for a Keep's AxeNStax world: test tooling, not part of the
//! game. One `#[ignore]`d test, driven by environment variables, that joins a Keep's
//! world by room code through the engine's real client path and prints one JSON line.
//!
//! The path is the game's own, with no protocol of its own: the code is looked up on
//! the relays and the room dialled over Fray QUIC (`fray_play::PendingJoin`), the
//! Signet challenge is signed with a persona key over the connection's exporter
//! (`RemoteClient::connect_authed_on_transport`, the signer the Join Game tests use),
//! the personal window is installed as the game loop does, pushed chunks go through
//! `ChunkIntake`, and a block is placed with the movement packet's edit, as a Mirror
//! session's placement is.
//!
//! Run (the test binary directly, so cargo is not needed to run it):
//!
//! ```text
//! PROBE_CODE=ABCDEF PROBE_KEY_FILE=persona.key PROBE_OUT=probe.json \
//!   <test binary> keep_join_probe::probe --ignored --exact --nocapture
//! ```
//!
//! Environment:
//! * `PROBE_CODE` (required): the room code, spaces and case ignored by the lookup.
//! * `PROBE_KEY_FILE`: the persona's secret key as hex; created if absent, so a rejoin
//!   with the same file is the same persona. Absent: a fresh key for this run.
//! * `PROBE_NAME`: the player name sent (default `Probe`).
//! * `PROBE_DATA_DIR`: where Fray may keep its files (default: the temp dir).
//! * `PROBE_RELAYS`: comma-separated wss:// relays, replacing Fray's defaults.
//! * `PROBE_RD`: render distance in columns (default 2).
//! * `PROBE_PLACE`: `x,y,z`, or `auto` (the cell above the spawn's head) or `auto:DX`
//!   (the same, DX blocks east). Places one block there once the chunks have settled.
//! * `PROBE_BLOCK`: the block id placed (default 1, stone).
//! * `PROBE_READ`: `x,y,z` cells (`;` between several) to read from the pushed chunks.
//! * `PROBE_HOLD_SECS`: stay seated this long after the above (default 0).
//! * `PROBE_SETTLE_SECS`: most to wait for the chunk push to go quiet (default 15).
//! * `PROBE_OUT`: a file the JSON line is also appended to.
//!
//! The result line has `joined`, `reason`, `reject_via`, `chunks`, `local_notes`,
//! `persona_npub`, `inventory_mode`, `spawn`, timings in ms, `reads`, `place`.
//! The test fails (non-zero exit) unless the join succeeded and every requested
//! read or place got its answer.

use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use decented_net::{Identity, Keys};
use serde_json::{json, Value};

use crate::block::BlockRegistry;
use crate::chunk_intake::{ChunkIntake, StreamItem};
use crate::fray_play::{JoinPoll, PendingJoin};
use crate::fray_transport::net_config;
use crate::protocol;
use crate::remote_client::{ConnectionState, RemoteClient, RequestOutcome, SignDriverFn, SignedJoin};
use crate::window_events::{InboxItem, WindowInbox};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn env_secs(name: &str, default: u64) -> Duration {
    Duration::from_secs(env(name).and_then(|v| v.trim().parse().ok()).unwrap_or(default))
}

fn parse_cell(text: &str) -> Option<[i32; 3]> {
    let parts: Vec<i32> = text.split(',').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
    <[i32; 3]>::try_from(parts).ok()
}

/// The persona key: from the file when it holds one, else new (and written there).
fn persona(path: Option<String>) -> Keys {
    let Some(path) = path else { return Keys::generate() };
    if let Ok(text) = std::fs::read_to_string(&path) {
        return Keys::parse(text.trim()).expect("the persona key file holds a hex secret key");
    }
    let keys = Keys::generate();
    std::fs::write(&path, keys.secret_key().to_secret_hex()).expect("the persona key file is writable");
    keys
}

/// Signs the challenge with `keys` over the origin the client built, as the
/// Join Game tests' driver does.
fn driver(keys: Keys) -> SignDriverFn {
    Box::new(move |nonce: String, origin: String| {
        let signed = pollster::block_on(crate::signet::native_signer::build_join_auth_event(&keys, &nonce, &origin))
            .map(|auth_event| SignedJoin { auth_event, credential: None })
            .map_err(|e| format!("{e:?}"));
        let (tx, rx) = mpsc::channel();
        tx.send(signed).unwrap();
        rx
    })
}

/// The client's half of a session: what the game loop does with what `poll` reads.
struct Session {
    rc: RemoteClient,
    reg: BlockRegistry,
    world: crate::world::World,
    loaded: ahash::AHashSet<(i32, i32)>,
    intake: ChunkIntake,
    inv: crate::inventory::Inventory,
    armour: [Option<crate::armour::ArmourItem>; 4],
    ui: crate::craft_ui::CraftingUi,
    inbox: WindowInbox,
    chunks: u32,
    local_notes: u32,
    spawn: Option<glam::Vec3>,
    render_distance: u8,
    /// The latest word the server sent about each cell that changed.
    changes: HashMap<[i32; 3], u16>,
    last_chunk_at: Instant,
    last_input: Instant,
}

impl Session {
    fn new(rc: RemoteClient, render_distance: u8) -> Session {
        Session {
            rc,
            reg: BlockRegistry::new(),
            world: crate::world::World::new(),
            loaded: Default::default(),
            intake: Default::default(),
            inv: crate::inventory::Inventory::new(),
            armour: [None; 4],
            ui: crate::craft_ui::CraftingUi::new(),
            inbox: Default::default(),
            chunks: 0,
            local_notes: 0,
            spawn: None,
            render_distance,
            changes: HashMap::new(),
            last_chunk_at: Instant::now(),
            last_input: Instant::now(),
        }
    }

    /// One pass of the client: read the link, install the window, take in the
    /// chunk stream and the block changes, and (once the personal window is in) send
    /// a movement packet every 100 ms, which also acknowledges the chunks.
    fn step(&mut self) {
        self.rc.poll();
        if let Some(joined) = self.rc.pending_joined_world.take() {
            self.spawn = joined.spawn;
            if let Some(spawn) = joined.spawn {
                self.intake.expect_notes(joined.chunk_note_radius, crate::chunk_stream::column_of(spawn));
            }
        }
        let outcomes = std::mem::take(&mut self.rc.pending_outcomes);
        let grants = std::mem::take(&mut self.rc.pending_grants);
        self.inbox.add(outcomes, grants, Vec::new());
        let (items, _) = self.inbox.take_ordered();
        for item in items {
            let event = item.event();
            match item {
                InboxItem::Outcome(RequestOutcome::Sync(pkt)) => {
                    let mut w = crate::window::WindowMut {
                        inv: &mut self.inv,
                        armour: &mut self.armour,
                        cursor: &mut self.ui.cursor_item,
                        grid: &mut self.ui.grid,
                        container: None,
                    };
                    let station = crate::inventory_sync::apply(
                        &mut w,
                        &pkt,
                        &self.reg,
                        &mut crate::use_edits::SentUses::default(),
                        &mut self.inbox.debt,
                        self.rc.inventory_mode,
                    )
                    .expect("a readable inventory sync");
                    if let Some(station) = station {
                        self.ui.adopt_station(station);
                    }
                    self.rc.note_initial_personal_window_applied(event, pkt.locks);
                }
                InboxItem::Grant(grant) => {
                    crate::remote_entities::apply_inventory_grant(&mut self.inv, &grant, &self.reg);
                }
                _ => {}
            }
            self.rc.note_window_event(event);
        }
        crate::hosted_server::tests::client_apply_personal(&mut self.rc);

        for (_, item) in std::mem::take(&mut self.rc.chunk_queue) {
            match item {
                StreamItem::Chunk(pkt) => {
                    self.chunks += 1;
                    self.last_chunk_at = Instant::now();
                    self.intake.apply(&mut self.world, &mut self.loaded, &self.reg, &pkt);
                }
                StreamItem::Local(col, hash) => {
                    self.local_notes += 1;
                    self.last_chunk_at = Instant::now();
                    self.intake.note_local(col, hash);
                }
                _ => {}
            }
        }
        for change in std::mem::take(&mut self.rc.pending_block_changes) {
            self.changes.insert([change.x, change.y, change.z], change.new_block);
            self.world.apply_remote_block_change(&change);
        }
        // What the probe does not use is dropped, not kept for the whole hold.
        self.rc.pending_entity_batches.clear();
        self.rc.pending_chat.clear();
        self.rc.pending_kills.clear();

        if self.rc.is_connected() && self.last_input.elapsed() >= Duration::from_millis(100) {
            self.last_input = Instant::now();
            let at = self.spawn.unwrap_or_default();
            let input = protocol::InputPacket {
                x: at.x,
                y: at.y,
                z: at.z,
                health: 20.0,
                chunk_ack: self.intake.applied(),
                render_distance: self.render_distance,
                ..Default::default()
            };
            self.rc.send_input(&input);
        }
    }

    fn run_for(&mut self, span: Duration) {
        let end = Instant::now() + span;
        while Instant::now() < end && self.rc.is_connected() {
            self.step();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// What the pushed chunks say about `cell`: the block id, or why there is no answer.
    fn read(&self, cell: [i32; 3]) -> Value {
        let cs = crate::chunk::CHUNK_SIZE as i32;
        let chunk = (cell[0].div_euclid(cs), cell[1].div_euclid(cs), cell[2].div_euclid(cs));
        if self.intake.holds_chunk(chunk) {
            json!({ "cell": cell, "block": self.world.get_block(cell[0], cell[1], cell[2]), "source": "pushed" })
        } else {
            json!({ "cell": cell, "block": null, "source": "not pushed (the server called the column local)" })
        }
    }
}

fn state_name(state: &ConnectionState) -> String {
    match state {
        ConnectionState::Disconnected => "disconnected".into(),
        ConnectionState::Connecting => "connecting".into(),
        ConnectionState::Connected { .. } => "connected".into(),
        ConnectionState::Failed(why) => format!("failed: {why}"),
    }
}

fn emit(result: &Value) {
    let line = result.to_string();
    eprintln!("PROBE_JSON {line}");
    if let Some(path) = env("PROBE_OUT") {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{line}");
        }
    }
}

/// Report a join that did not happen and fail the test.
fn fail(mut result: Value, reason: String, via: &str) -> ! {
    result["reason"] = json!(reason);
    result["reject_via"] = json!(via);
    result["chunks"] = json!(0);
    emit(&result);
    panic!("the probe did not join: {reason}");
}

#[test]
#[ignore = "needs a running Keep and PROBE_CODE; see the module doc"]
fn probe() {
    let started = Instant::now();
    let ms = |t: Instant| t.duration_since(started).as_millis() as u64;
    let code = env("PROBE_CODE").expect("PROBE_CODE is the Keep world's room code");
    let keys = persona(env("PROBE_KEY_FILE"));
    let npub = crate::hosted_server::pubkey_to_npub(&keys.public_key().to_bytes());
    let render_distance = env("PROBE_RD").and_then(|v| v.parse::<u8>().ok()).unwrap_or(2);
    crate::remote_client::set_join_render_distance(i32::from(render_distance));

    let data_dir = env("PROBE_DATA_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&data_dir).expect("the data dir is creatable");
    let mut cfg = net_config(data_dir);
    if let Some(relays) = env("PROBE_RELAYS") {
        cfg.relays = relays.split(',').map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect();
        cfg.relay_fallback = false;
    }

    let mut result = json!({ "joined": false, "persona_npub": npub, "code": code, "render_distance": render_distance });
    // 1. Look the code up and reach the room.
    let handle = decented_net::join(&cfg, &Identity::ephemeral(), &code);
    let mut pending = PendingJoin::over(handle, code.clone());
    let deadline = Instant::now() + Duration::from_secs(60);
    let transport = loop {
        match pending.poll() {
            JoinPoll::Waiting => {}
            JoinPoll::Connected(transport) => break transport,
            JoinPoll::Failed(why) => fail(result, why, "fray"),
        }
        if Instant::now() > deadline {
            fail(result, "no answer from the room within 60 s".into(), "fray");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let reached = Instant::now();
    result["t_reached_ms"] = json!(ms(reached));

    // 2. The game's handshake: challenge, signature, JoinRequest, JoinAccept.
    let name = env("PROBE_NAME").unwrap_or_else(|| "Probe".into());
    let rc = RemoteClient::connect_authed_on_transport(Box::new(transport), &name, driver(keys), None);
    let mut s = Session::new(rc, render_distance);
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        s.step();
        match &s.rc.state {
            ConnectionState::Connected { .. } if s.rc.personal_ready() => break,
            ConnectionState::Failed(why) => {
                let via = if why.to_lowercase().contains("kick") || why.contains("Disconnected") { "link closed" } else { "JoinReject" };
                fail(result, why.clone(), via);
            }
            ConnectionState::Disconnected => fail(result, "disconnected before the join finished".into(), "link closed"),
            _ => {}
        }
        if Instant::now() > deadline {
            let why = format!("no JoinAccept within 60 s ({})", state_name(&s.rc.state));
            fail(result, why, "none");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    result["joined"] = json!(true);
    result["t_joined_ms"] = json!(ms(Instant::now()));
    result["inventory_mode"] = json!(format!("{:?}", s.rc.inventory_mode));
    result["spawn"] = json!(s.spawn.map(|p| [p.x, p.y, p.z]));

    // 3. Let the chunk push settle: it is quiet for a second, or the time is up.
    let settle_end = Instant::now() + env_secs("PROBE_SETTLE_SECS", 15);
    let mut first_chunk = None;
    loop {
        s.step();
        if first_chunk.is_none() && (s.chunks + s.local_notes) > 0 {
            first_chunk = Some(Instant::now());
            result["t_first_chunk_ms"] = json!(ms(Instant::now()));
        }
        let quiet = first_chunk.is_some() && s.last_chunk_at.elapsed() >= Duration::from_secs(1);
        if quiet || Instant::now() > settle_end || !s.rc.is_connected() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    result["t_settled_ms"] = json!(ms(Instant::now()));
    result["chunks"] = json!(s.chunks);
    result["local_notes"] = json!(s.local_notes);

    // 4. Read cells, then place one block, as asked.
    let mut ok = s.chunks > 0 && s.rc.is_connected();
    let mut reads = Vec::new();
    if let Some(list) = env("PROBE_READ") {
        for text in list.split(';') {
            let cell = parse_cell(text).expect("PROBE_READ is x,y,z[;x,y,z]");
            let read = s.read(cell);
            ok &= !read["block"].is_null();
            reads.push(read);
        }
    }
    result["reads"] = json!(reads);

    if let Some(spec) = env("PROBE_PLACE") {
        let at = s.spawn.expect("the join accept named a spawn");
        let cell = match spec.strip_prefix("auto") {
            Some(rest) => {
                let dx = rest.strip_prefix(':').and_then(|d| d.parse::<i32>().ok()).unwrap_or(0);
                [at.x.floor() as i32 + dx, at.y.floor() as i32 + 2, at.z.floor() as i32]
            }
            None => parse_cell(&spec).expect("PROBE_PLACE is x,y,z or auto[:DX]"),
        };
        let block = env("PROBE_BLOCK").and_then(|v| v.parse::<u16>().ok()).unwrap_or(crate::block::STONE);
        let before = s.read(cell);
        let (kind, id) = crate::inventory::item_to_ref(&crate::item::Item::Block(block)).to_wire();
        let change = protocol::BlockChange { x: cell[0], y: cell[1], z: cell[2], new_block: block, meta: 0 };
        let placed_at = Instant::now();
        let sent = s.rc.send_input(&protocol::InputPacket {
            x: at.x,
            y: at.y,
            z: at.z,
            health: 20.0,
            chunk_ack: s.intake.applied(),
            render_distance,
            held_kind: kind,
            held_id: id,
            block_changes: vec![change],
            edit_hands: vec![(0, kind, id)],
            ..Default::default()
        });
        // The server sends a refused edit's cell back; an accepted one stays quiet.
        s.run_for(Duration::from_secs(3));
        let sent_back = s.changes.get(&cell).copied();
        let sent_ok = sent.is_some();
        let refused = sent_back.is_some_and(|b| b != block);
        ok &= sent_ok && !refused;
        result["place"] = json!({
            "cell": cell, "block": block, "before": before, "sent": sent_ok,
            "refused_and_sent_back": refused, "server_sent_back": sent_back,
            "t_placed_ms": ms(placed_at),
        });
    }

    // 5. Hold the seat, then leave cleanly.
    let hold = env_secs("PROBE_HOLD_SECS", 0);
    if !hold.is_zero() {
        eprintln!("PROBE seated, holding {} s", hold.as_secs());
        s.run_for(hold);
    }
    ok &= s.rc.is_connected();
    s.rc.disconnect();
    // A moment for the goodbye to leave before the link is dropped.
    std::thread::sleep(Duration::from_millis(500));
    drop(s);
    result["held_secs"] = json!(hold.as_secs());
    result["t_left_ms"] = json!(ms(Instant::now()));
    result["ok"] = json!(ok);
    emit(&result);
    assert!(ok, "the probe's reads, placement or seat did not all hold; see the JSON line");
}
