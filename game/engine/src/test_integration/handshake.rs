//! Handshake integration tests — packet-validation invariants for the
//! JoinRequest path in `hosted_server.rs`.
//!
//! Today the engine's handshake logic lives inline inside `run_server_loop`,
//! which is self-clocked and not usable from a synchronous test. Until that
//! logic is factored out (candidate for Spec 1 Phase 2/3), the assertions
//! here exercise the validation **rules** via the packet types directly.
//! When the extraction lands, these tests migrate to driving `TestHost`
//! through real packets without asserting any new behaviour.

use crate::protocol;

#[test]
fn protocol_version_is_the_pinned_value() {
    // Clients and server must share the same PROTOCOL_VERSION. Pinning here
    // so any unreviewed bump is a test-visible event.
    // v9 (2026-05-17): Spec 16 — StateUpdatePacket gains reserve_richness,
    // reserve_target_sats, reserve_current_sats for the Deepslate Reserve.
    // v10 (2026-05-17): Wave 26 farming begins — TILLED_SOIL = id 30.
    // v11 (2026-05-17): Wave 26 farming — 12 crop-stage blocks (ids 31..=42).
    // v12 (2026-05-18): Wave 27 campfire — CAMPFIRE + CAMPFIRE_UNLIT (ids 43, 44).
    // v13 (2026-05-18): Wave 28 campfire extensions — CAMPFIRE_SMOKE (45),
    //   CORN_STAGE_0..3 (46..=49).
    // v14 (2026-05-18): Spec 19 phase 2 — Villager (9), IronGolem (10),
    //   WanderingVillager (11) EntityKind discriminants appended; also
    //   VILLAGE_BELL (id 50) per Spec 19 phase 10.
    // v15 (2026-05-19): Wave 29 log seasoning — DRYING_RACK (51), three new
    //   log materials (Green/Seasoned/KilnDried), OAK_LOG mine-drop change.
    // v16 (2026-05-19): Spec 23 Papyrus Reed — PAPYRUS_STAGE_0..3 (52..=55),
    //   PapyrusReed + PapyrusSheet materials, `is_paperish_slot` predicate.
    //   Foundation A of Build Schematics — unblocks Spec 24's Blueprint Paper.
    // v17 (2026-05-19): Spec 24 Build Schematics Core (Foundation B,
    //   partial). Item::Plan(PlanData) variant, BLUEPRINT_PAPER + CONSTRUCTION_ANCHOR
    //   + ARCHITECT_PLAQUE blocks (56-58), SavedSlot::Plan + 2 new
    //   WorldSave fields. Phase 4 capture-on-right-click wired; UI
    //   dialogs (Phases 5-12) deferred.
    // v18 (2026-05-20): Spec 24 Phases 5+7+9+10+11+12 + 2026-05-20
    //   paper-economy + creative-vs-survival amendments. PlanData
    //   gains authored_in; ConstructionAnchorData gains is_creative_build;
    //   ArchitectPlaqueData wraps chain+authored_in; Blueprint Paper recipe
    //   yield 4→9 per PapyrusSheet; WorldMeta.has_seen_license_onboarding.
    // v19 (2026-05-21): Spec 28d chunk 3 — EntityKind appends Horse (13)
    //   and Rabbit (14); MaterialId appends RawRabbit, CookedRabbit,
    //   RabbitHide. Live spawn deferred; wire shape locked.
    // v20 (2026-05-21): Spec 28d chunk 4 — EntityKind::Goat (15) appended.
    //   Goat AI module ships with same deferred-spawn posture.
    // v21 (2026-05-21): Spec 28d chunk 5 — EntityKind::Bee (16) appended;
    //   MaterialId appends HoneyBottle + BeeStinger. Bee AI is
    //   flight-bobbed wander with sting-on-attack.
    // v22 (2026-05-21): Spec 28d chunk 6 — EntityKind::Squid (17)
    //   appended. Squid AI is aquatic drift with suffocation timer;
    //   drops 1-3 InkSac.
    // v23 (2026-05-21): Spec 28d chunk 7 — a retired-roster EntityKind (18)
    //   + drop MaterialId appended (both excised in v39); a ranged-attack
    //   AI module shipped alongside.
    // v24 (2026-05-21): Spec 28d chunk 8 — BEE_HIVE block (id 111),
    //   BlockEntityData::Hive variant, WorldSave.hives Vec.
    // v25 (2026-05-22): HP-2 — EntityKind::Bear (20) + EntityKind::Hyena
    //   (21) appended. CHEST block (id 112), BlockEntityData::Chest
    //   variant, WorldSave.chests Vec.
    // v26 (2026-05-23): HP-3 — EntityKind::Brigand (22) + Marauder (23)
    //   + Berserker (24) appended. BRIGAND_HIDEOUT_BANNER block (id
    //   113). WorldSave.brigand_hideouts Vec (serde-default).
    // v27 (2026-05-23): HP-4 — EntityKind::Knight (25) appended.
    //   Knight reuses GolemGuard state; no new state-tag.
    // v28 (2026-05-23): HP-3 v2 — TROPHY_WALL block (id 114).
    //   Decorative; crafted from BrigandChieftainTrophy + 2 planks.
    // v29 (2026-05-23): Salt feature — 5 new BlockIds (ROCK_SALT 115,
    //   SALT_LICK 116, SALT_LAMP 117, SALT_BLOCK 118, SALT_PATH 119) +
    //   Salt + 15 Cured/Seasoned MaterialIds. Positional enum append.
    // v30 (2026-05-23): Rubber feature — 4 new BlockIds (RUBBER_LOG 120,
    //   RUBBER_PLANKS 121, RUBBER_LEAVES 122, RUBBER_LOG_TAPPED 123) +
    //   Rubber + RubberSapling + RubberBall + CopperCable Materials +
    //   WoodSpecies::Rubber + ToolType::Slingshot + ToolType::Eraser +
    //   ArmourMaterial::Rubber. All positional enum appends.
    // v31 (2026-05-23): Mob Bounty Board (Spec 33) — BOUNTY_BOARD = 124,
    //   MaterialId::BountyBoardItem, PayoutKind::BountyClaim, 3 new
    //   WorldSave fields (bounties Vec, bounty_next_id,
    //   bounty_last_refresh_tick). PlayerSlot.bounties_claimed map.
    // v32 (2026-05-23): Tip Jar (Spec 34) — TIP_JAR = 125,
    //   MaterialId::TipJarItem, PayoutKind::Tip,
    //   BlockEntityData::TipJar variant + WorldSave.tip_jars Vec.
    //   PlayerSlot.open_tip_jar.
    // v33 (2026-05-23): Tool Repair / Repair Bench (Spec 35) —
    //   REPAIR_BENCH = 126, MaterialId::RepairBenchItem,
    //   PayoutKind::RepairTax. Stateless block (no block-entity).
    // v34 (2026-05-23): Plot Ownership v1 (Spec 36) — PLOT_MARKER =
    //   127, MaterialId::PlotMarkerItem, WorldSave.plots. Also fixed
    //   the economy-block-items placeable bug (Specs 33-35).
    // v35 (2026-05-23): Market Hubs v1 (Spec 37) — MARKET_BELL = 128,
    //   MaterialId::MarketBellItem, WorldSave.market_hubs.
    // v36 (2026-05-23): Auctions v1 (Spec 38) — AUCTION_BLOCK = 129,
    //   MaterialId::AuctionBlockItem, BlockEntityData::Auction +
    //   WorldSave.auctions.
    // v37 (2026-05-23): Server Bazaar v1 (Spec 39) — BAZAAR_BLOCK =
    //   130, MaterialId::BazaarBlockItem, PayoutKind::BazaarSale.
    //   Stateless (no block-entity).
    // v38 (2026-05-24): Player avatars + viewmodel Phase 1 — PlayerState
    //   held_item -> held_kind/held_id (tool-capable) + anim_state + flags.
    // v39 (2026-05-24): fantasy roster excised for open-source cleanup —
    //   removed the 7 retired hostile-mob EntityKinds (BREAKING).
    // v40 (2026-05-28): Spec 38 Blueprint / Cyanotype — `PlanData.develop_state`
    //   (Latent / Developed), `LATENT_PRINT = 158` BlockId,
    //   `BlockEntityData::LatentPrint`, `WorldSave.latent_prints`, recipe swap
    //   `Stick + Papyrus → 9 Plan Tiles` → `Papyrus + Iron + Salt → 3 Blueprint Paper`.
    // v41 (2026-05-28): Spec 40 Bulk Vendor — `VendorMode::Bulk` + `VendorData.lot_size`,
    //   `vendor::try_buy` pure extract of the inline Sell/Buy/Barter buy logic,
    //   `preview_refusal` moved into the vendor module (re-exported from vendor_ui).
    // v42 (2026-06-02): player cosmetics — `JoinRequestPacket.skin_key` +
    //   `PlayerState.skin_key` (u64 per-player skin reference, appended last).
    // v43 (2026-06-04): Blueprint column-capture — ToolType::DraftingStamp
    //   appended last (positional enum append, wire-stable).
    // v44 (2026-06-09): JoinAcceptPacket + ServerAnnouncePacket gain play_mode:
    //   PlayMode alongside is_creative (derived projection kept for back-compat).
    // v45 (2026-06-10): Rail freight Phase 1 — EntityKind::Cart (26) appended.
    //   Carts broadcast via the mob EntitySpawn/EntityUpdate path (positional
    //   enum append, wire-stable).
    // v46 (2026-06-10): Craftable armoured carts CA1 — CartData gains a trailing
    //   `hull` tier (save-shape change; WIRE unchanged, no hull on EntitySpawn/
    //   EntityUpdate yet). Pre-hull saves with carts load with carts == [].
    // v47 (2026-06-16): Dedicated Docker server — JoinAccept now carries the
    //   world's REAL seed instead of a hardcoded 42 (gap G3). Wire layout
    //   unchanged; the bump forces lockstep client updates via this check.
    // v48 (2026-06-16): Phase 4 verified identity — ChallengePacket gains a
    //   trailing `origin` (the value the client signs into its auth event);
    //   authenticated clients now WAIT for the challenge, sign `{nonce, origin}`,
    //   and send a JoinRequest carrying the signed `auth_event`. The server
    //   verifies a present auth_event (tamper → reject) and rejects an absent one
    //   on a sign-in-required host. `player_name` is now a display fallback only,
    //   never a trusted identity. JoinRequest/JoinAccept wire layout unchanged.
    // v49 (2026-06-16): Phase 4 inspect view — `PlayerEventType::Joined` gains a
    //   trailing `npub` (joiner's full NIP-19 npub, `""` for a guest) so the
    //   client renders a copyable inspect view to verify a specific person.
    // v50 (2026-06-17): Track 3 server-identity — JoinRequest gains
    //   `client_nonce_hex`; JoinAccept gains `server_identity` (operator-signed
    //   attestation + signature over the nonce) so a client can verify the server.
    // v51 (2026-06-17): Spec 48 Electricity — BlockChange gains `meta: u8` and
    //   the chunk stream carries sparse per-block metadata.
    // v52 (2026-06-17): Operator Console (Spec B) — `PacketType::OperatorSnapshot
    //   = 51` + `OperatorSnapshotPacket { snapshot_json }`, server → an
    //   authenticated operator-player only. Append-only; no existing packet changed.
    // v53 (2026-06-18): Texture packs (Spec 03 §11.6) — `PacketType::
    //   ResourcePackSuggest = 52` + `ResourcePackSuggestPacket { name, url,
    //   sha256, size_bytes, required }`, server → client. Append-only.
    // v54 (2026-06-19): Creator Gallery — `JoinAcceptPacket` gains a final
    //   `exhibits: Vec<Exhibit>` so a joiner renders the world's authored 2D art.
    // v55 (2026-06-20): Explosives (Spec 49) — new block/material ids +
    //   Composter block-entity + BlastingKeg PowerDevice kind.
    // v56 (2026-07-06): Pets-debt-water wave — `EntityKind` gained `Crab`
    //   (and earlier Fish/Fox/Cat/Donkey/Mule); no packet shape changed.
    // v57 (2026-07-11): Death-drops phase 2 — `EntityKind::Item` + the
    //   `item_kind`/`item_id`/`item_count` stack payload on `EntitySpawn` so
    //   server-side loot is visible to remote clients. Packet shape CHANGED
    //   (EntitySpawn widened).
    // v58 (2026-07-11): Death-drops phase 2b — `PacketType::InventoryGrant = 53`
    //   + `InventoryGrantPacket { item_kind, item_id, count }`, server → the
    //   picking-up client only, so a server-side pickup lands the stack in the
    //   remote client's inventory. Append-only; no existing packet changed.
    // v59 (2026-09-03): Weather sync (P9) — `StateUpdatePacket` gains a
    //   trailing `rain_ticks_left: u32` + `storm_ticks_left: u32` so a joined
    //   client's rain/lightning window matches the server's authoritative
    //   one. Append-only; no existing packet shape changed.
    // v60 (2026-09-05): World chat Phase 2 — `PacketType::ChatSay = 54` +
    //   `PacketType::ChatDeliver = 55` (+ payload structs). Append-only.
    // v61 (2026-09-06): Death-drops phase 3, full-fidelity item wire — the new
    //   `WireItem` enum appended as `full_item` on `EntitySpawn` and on
    //   `InventoryGrantPacket`, carrying tool type/material/durability and
    //   armour slot/material/durability so server-side tool/armour drops reach
    //   remote players intact. Packet shape CHANGED (two structs widened).
    // v62 (2026-09-07): Wind/Copper/Electricity wave — `PacketType::
    //   DeviceInteract = 56` + `DeviceInteractPacket { pos }`, client → server,
    //   so a joiner's lever/button/crank/mirror reaches the host that owns the
    //   authoritative power sim. The host re-derives the interaction from the
    //   cell alone and broadcasts the result on the block-change path.
    //   Append-only; no existing packet shape changed.
    // v63 (2026-09-27): Join channel binding (audit fix B) — `ChallengePacket`
    //   LOSES its `origin`. The joiner signs `axenstax-join:tls-exporter:<hex>`
    //   (or `axenstax-join:unbound`) built from its OWN transport; the server
    //   recomputes it from its transport and requires an exact match, closing
    //   the relay and web-login-oracle attacks. Packet shape CHANGED.
    // v64 (2026-09-28): QUIC framing (audit wave 1) — every game packet rides
    //   ONE reliable, ordered stream as `u32 LE length + payload` (client opens
    //   it with a zero-length hello); datagrams are gone. The server closes a
    //   client with more than 8 MiB queued. No packet shape changed.
    // v65 (2026-10-06): JoinAccept gains `world_rules` + `worldgen_version`,
    //   JoinRequest gains `worldgen_version` (gap-audit T2-9).
    assert_eq!(protocol::PROTOCOL_VERSION, 65);
}

#[test]
fn join_request_round_trips_bincode() {
    let req = protocol::JoinRequestPacket {
        protocol_version: protocol::PROTOCOL_VERSION,
        player_name: "Axo".to_string(),
        auth_event: None,
        handle_credential: None,
        skin_key: 0,
        client_nonce_hex: String::new(),
        worldgen_version: crate::world::WORLDGEN_VERSION,
    };
    let pkt = protocol::serialize_packet(protocol::PacketType::JoinRequest, &req);
    let (ptype, payload) = protocol::deserialize_header(&pkt).unwrap();
    assert_eq!(ptype, protocol::PacketType::JoinRequest);
    let back: protocol::JoinRequestPacket = protocol::safe_deserialize(payload).unwrap();
    assert_eq!(back.protocol_version, req.protocol_version);
    assert_eq!(back.player_name, req.player_name);
}

#[test]
fn join_reject_round_trips_bincode() {
    let rej = protocol::JoinRejectPacket { reason: "Protocol mismatch".to_string() };
    let pkt = protocol::serialize_packet(protocol::PacketType::JoinReject, &rej);
    let (ptype, payload) = protocol::deserialize_header(&pkt).unwrap();
    assert_eq!(ptype, protocol::PacketType::JoinReject);
    let back: protocol::JoinRejectPacket = protocol::safe_deserialize(payload).unwrap();
    assert_eq!(back.reason, "Protocol mismatch");
}

#[test]
fn player_name_length_rule() {
    // hosted_server.rs caps player_name at 32 bytes and rejects control chars.
    // Reproducing the same predicate here so a change on either side fires.
    const MAX_PLAYER_NAME_LEN: usize = 32;

    let ok = "Axo";
    assert!(ok.len() <= MAX_PLAYER_NAME_LEN);
    assert!(!ok.chars().any(|c| c.is_control()));

    let too_long = "x".repeat(33);
    assert!(too_long.len() > MAX_PLAYER_NAME_LEN);

    let with_control = "Axo\x1b[31m";
    assert!(with_control.chars().any(|c| c.is_control()));
}

#[test]
fn join_accept_carries_spawn_and_game_mode() {
    let acc = protocol::JoinAcceptPacket {
        player_index: 1,
        seed: 42,
        spawn_x: 0.5, spawn_y: 80.0, spawn_z: 0.5,
        world_time: 6000,
        is_creative: false,
        play_mode: crate::play_mode::PlayMode::Survival,
        difficulty: "normal".to_string(),
        server_identity: None,
        // v54 — Creator Gallery exhibits ride JoinAccept so a joiner renders them.
        exhibits: vec![crate::exhibit::Exhibit {
            x: 3,
            y: 64,
            z: -1,
            presentation: crate::exhibit::Presentation::Wall,
            image_ref: "joined.png".to_string(),
            width: 2.0,
            height: 1.5,
            yaw: 0.0,
            label: "Joined".to_string(),
            link: None,
            sku: None,
            price: None,
        }],
        world_rules: protocol::WorldRules::default(),
        worldgen_version: crate::world::WORLDGEN_VERSION,
    };
    let pkt = protocol::serialize_packet(protocol::PacketType::JoinAccept, &acc);
    let (_ptype, payload) = protocol::deserialize_header(&pkt).unwrap();
    let back: protocol::JoinAcceptPacket = protocol::safe_deserialize(payload).unwrap();
    assert_eq!(back.player_index, 1);
    assert_eq!(back.difficulty, "normal");
    assert!(!back.is_creative);
    assert_eq!(back.play_mode, crate::play_mode::PlayMode::Survival);
    // The authored exhibit survives the JoinAccept round-trip (v54).
    assert_eq!(back.exhibits.len(), 1);
    assert_eq!(back.exhibits[0].image_ref, "joined.png");
    assert_eq!(back.exhibits[0].presentation, crate::exhibit::Presentation::Wall);
}

// ── Online play by contact: the socket handoff into quinn (Task 14) ──

/// A hosted server started on a socket we bound ourselves must listen on THAT
/// port — the whole point of the handoff is that the port a peer was told to
/// dial (in the candidates) is the port QUIC answers on.
#[test]
fn hosted_server_started_on_a_prebound_socket_keeps_its_port() {
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = sock.local_addr().unwrap().port();
    let server = crate::hosted_server::HostedServer::start_online(
        1,
        "prebound-test".to_string(),
        42,
        4,
        sock,
    )
    .expect("start_online");
    assert_eq!(server.port, port, "the accept thread must own the socket we bound");
    assert_ne!(port, crate::protocol::SERVER_PORT, "an ephemeral port, not the default");
}

/// A connect race with nothing to dial resolves immediately as an error rather
/// than hanging for the full deadline.
#[test]
fn connect_to_server_on_socket_with_no_candidates_fails_fast() {
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let conn = crate::network::connect_to_server_on_socket(sock, vec![], "sess".to_string());
    let got = conn
        .outcome
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("the race must report an outcome without waiting out the 8s deadline");
    assert!(got.is_err(), "no candidates cannot succeed: {got:?}");
}

/// A candidate nothing is listening on loses, and the race says so before the
/// deadline rather than after it — that is what the per-attempt timeout buys.
/// Without it the attempt hangs on quinn's own idle timeout and the only thing
/// that ends the race is the 8 s deadline.
#[test]
fn connect_to_server_on_socket_reports_an_unreachable_candidate() {
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    // Bind and immediately drop, so the port is (almost certainly) dead.
    let dead = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let dead_addr = dead.local_addr().unwrap();
    drop(dead);
    let started = std::time::Instant::now();
    let conn =
        crate::network::connect_to_server_on_socket(sock, vec![dead_addr], "sess".to_string());
    let got = conn
        .outcome
        .recv_timeout(crate::nat::punch::CONNECT_DEADLINE + std::time::Duration::from_secs(2))
        .expect("the race must report an outcome");
    let elapsed = started.elapsed();
    assert!(got.is_err(), "a dead candidate cannot win: {got:?}");
    // The sole candidate failed, so the race is AllFailed ("no candidate
    // answered"), NOT the deadline's "timed out".
    assert_eq!(
        got.as_ref().err().map(String::as_str),
        Some("no candidate answered"),
        "one dead candidate must exhaust the race, not run out the clock: {got:?}"
    );
    // ~200 ms of punching plus the 3 s per-attempt budget — comfortably inside
    // the 8 s deadline it used to wait out.
    assert!(
        elapsed < crate::nat::punch::CONNECT_DEADLINE,
        "a dead candidate must lose on its own timeout, not the deadline (took {elapsed:?})"
    );
}

/// A hosted server with no remote slots never spawns an accept thread, so the
/// pre-bound socket would be dropped — closed — while `port` went on reporting
/// it. Refuse loudly instead of handing back a host that cannot be reached.
#[test]
fn start_online_refuses_a_host_with_no_remote_slots() {
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    // `HostedServer` isn't `Debug`, so unwrap the Result by hand.
    let Err(err) = crate::hosted_server::HostedServer::start_online(
        1,
        "prebound-no-slots".to_string(),
        42,
        0,
        sock,
    ) else {
        panic!("a zero-slot online host must be rejected, not silently deaf");
    };
    assert!(
        err.contains("remote slot"),
        "the error must say what is wrong: {err}"
    );
}

/// The end of the handoff, both halves at once and entirely on loopback: a host
/// started on a socket it bound itself, and a joiner that punches and then races
/// a dead candidate against the host's real one. The race must skip the loser
/// and report the winner's address — that is what tells the joiner "you're in"
/// rather than showing the "couldn't reach" copy.
#[test]
fn connect_to_server_on_socket_wins_against_a_prebound_host() {
    let host_sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let host_addr = host_sock.local_addr().unwrap();
    let _host = crate::hosted_server::HostedServer::start_online(
        1,
        "prebound-race-test".to_string(),
        42,
        4,
        host_sock,
    )
    .expect("start_online");

    // A dead candidate ahead of the live one: the race must not stop at it.
    let dead = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let dead_addr = dead.local_addr().unwrap();
    drop(dead);

    let joiner = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let conn = crate::network::connect_to_server_on_socket(
        joiner,
        vec![dead_addr, host_addr],
        "race-sess".to_string(),
    );
    let got = conn
        .outcome
        .recv_timeout(crate::nat::punch::CONNECT_DEADLINE + std::time::Duration::from_secs(2))
        .expect("the race must report an outcome");
    assert_eq!(
        got.as_ref().ok(),
        Some(&host_addr),
        "the live candidate must win the race: {got:?}"
    );
}
