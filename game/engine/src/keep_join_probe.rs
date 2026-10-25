//! A headless join probe for a Keep's AxeNStax world: test tooling, not part of the
//! game. One `#[ignore]`d test, driven by environment variables, that joins a Keep's
//! world by room code, or by invite (the Keep's key and a secret, as a pass or an
//! `axenstax://keep/` link carries them), through the engine's real client path and
//! prints one JSON line.
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
//! * `PROBE_CODE`: the room code, spaces and case ignored by the lookup. Joins by code.
//! * `PROBE_HOST_KEY` and `PROBE_SECRET` (both, 64 hex characters each; instead of
//!   `PROBE_CODE`): joins by invite. The persona must be the one the invite is bound to
//!   (a return pass is bound to the persona that got it: use the same `PROBE_KEY_FILE`).
//! * `PROBE_PASS_FILE`: where a return pass the Keep sends is written, as JSON
//!   `{"host_key":..,"secret":..}`, mode 0600, so a later run can join by it. The pass's
//!   secret is never in the result line, the log or the test output.
//! * `PROBE_JOIN_TIMEOUT_SECS`: most to wait for the Keep to answer (default 60); a join
//!   that is refused or finds no one waits this long before it reports.
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
//! The result line has `joined`, `joined_by` (`code` or `invite`), `reason`, `reject_via`,
//! `chunks`, `local_notes`, `persona_npub`, `inventory_mode`, `spawn`, `host` (the host's
//! proven key's first six hex characters, and the name it gave), `passes` (the first six
//! hex characters of each return pass's key, never the secret), timings in ms, `reads`,
//! `place`.
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

/// A 32-byte value from a variable that holds 64 hex characters.
fn env_key(name: &str) -> Option<[u8; 32]> {
    let text = env(name)?;
    Some(fray_keep::parse_hex32(text.trim()).unwrap_or_else(|| panic!("{name} is 64 hex characters")))
}

/// Write a return pass to `path` for a later run to join by, private to this user.
fn write_pass_file(path: &str, pass: &protocol::KeepPass) {
    use std::io::Write;
    let text = json!({ "host_key": fray_keep::hex_encode(&pass.host_key), "secret": fray_keep::hex_encode(&pass.secret) }).to_string();
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).expect("the pass file is writable");
    // `mode` above applies only when the file is created: a file that was already there keeps
    // its old mode, so narrow it before the secret goes in.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600)).expect("the pass file is private");
    }
    file.write_all(text.as_bytes()).expect("the pass file is writable");
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
    /// Return passes the Keep sent (held here, never printed).
    passes: Vec<protocol::KeepPass>,
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
            passes: Vec::new(),
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
        self.passes.extend(self.rc.take_keep_passes());
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
    // By room code, or by invite (the Keep's key and a secret).
    let invite = env_key("PROBE_HOST_KEY").map(|host_key| {
        let secret = env_key("PROBE_SECRET").expect("PROBE_SECRET goes with PROBE_HOST_KEY");
        fray_keep::Invite { host_key, secret }
    });
    let code = env("PROBE_CODE");
    assert!(
        invite.is_some() != code.is_some(),
        "give PROBE_CODE (join by code) or PROBE_HOST_KEY and PROBE_SECRET (join by invite), not both or neither"
    );
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

    let mut result = json!({
        "joined": false, "persona_npub": npub, "render_distance": render_distance,
        "joined_by": if invite.is_some() { "invite" } else { "code" },
    });
    // 1. Look the code (or the invite) up and reach the room.
    let (handle, label) = match (&invite, &code) {
        (Some(invite), _) => {
            result["keep"] = json!(fray_keep::describe(&invite.host_key));
            (decented_net::join_invite(&cfg, &Identity::ephemeral(), &invite.secret, invite.host_key), crate::keeps::label(&invite.host_key))
        }
        (None, Some(code)) => {
            result["code"] = json!(code);
            (decented_net::join(&cfg, &Identity::ephemeral(), code), code.clone())
        }
        (None, None) => unreachable!("checked above"),
    };
    let mut pending = PendingJoin::over(handle, label);
    pending.set_invite(invite.is_some());
    let wait = env_secs("PROBE_JOIN_TIMEOUT_SECS", 60);
    let deadline = Instant::now() + wait;
    let transport = loop {
        match pending.poll() {
            JoinPoll::Waiting => {}
            JoinPoll::Connected(transport) => break transport,
            JoinPoll::Failed(why) => fail(result, why, "fray"),
        }
        if Instant::now() > deadline {
            fail(result, format!("no answer from the room within {} s", wait.as_secs()), "fray");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let reached = Instant::now();
    result["t_reached_ms"] = json!(ms(reached));

    // 2. The game's handshake: challenge, signature, JoinRequest, JoinAccept.
    let name = env("PROBE_NAME").unwrap_or_else(|| "Probe".into());
    let rc = RemoteClient::connect_authed_on_transport(Box::new(transport), &name, driver(keys), None);
    let mut s = Session::new(rc, render_distance);
    if let Some(host) = s.rc.proven_host() {
        result["host"] = json!({ "key": fray_keep::describe(&host.key), "name": host.name });
    }
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
    // A pass can arrive after the join settles: take what is waiting before leaving.
    s.step();
    let passes = std::mem::take(&mut s.passes);
    if let (Some(path), Some(pass)) = (env("PROBE_PASS_FILE"), passes.first()) {
        write_pass_file(&path, pass);
    }
    s.rc.disconnect();
    // A moment for the goodbye to leave before the link is dropped.
    std::thread::sleep(Duration::from_millis(500));
    drop(s);
    result["passes"] = json!(passes.iter().map(|p| fray_keep::describe(&p.host_key)).collect::<Vec<_>>());
    result["held_secs"] = json!(hold.as_secs());
    result["t_left_ms"] = json!(ms(Instant::now()));
    result["ok"] = json!(ok);
    emit(&result);
    assert!(ok, "the probe's reads, placement or seat did not all hold; see the JSON line");
}

#[cfg(all(test, unix))]
mod pass_file_tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn the_pass_file_is_private_whether_or_not_it_was_already_there() {
        let dir = std::env::temp_dir().join(format!("axenstax-probe-pass-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pass.json");
        let path_str = path.to_str().unwrap();
        let pass = protocol::KeepPass { host_key: [0xab; 32], secret: [0x01; 32] };
        let mode = || std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;

        write_pass_file(path_str, &pass);
        assert_eq!(mode(), 0o600, "a new file");
        // A file left from an earlier run with a wider mode is narrowed before the secret goes in.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_pass_file(path_str, &pass);
        assert_eq!(mode(), 0o600, "an existing file");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(&fray_keep::hex_encode(&pass.secret)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
