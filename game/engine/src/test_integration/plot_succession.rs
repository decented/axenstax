//! Living world Tier 2 S5: joiners' Plots, last seen and succession.
//!
//! A real `HostedServer` with joiners over the channel transport: a verified
//! joiner's Plot Marker claims a Plot in its npub, a guest's claims nothing, an
//! overlapping claim is refused and the ghost put back, a second joiner is kept
//! out and the owner is not. Succession runs on the server's own tick with the
//! wall clock offset from now (so nothing sleeps and nothing is flaky), and with
//! an injected `now` on the pure phase; the save half runs a real folder.
use super::joiner_authority::{block_changes_seen, cell_beside, join_guest, send_edits, start_open_server};
use crate::authorship::AuthorKey;
use crate::block;
use crate::hosted_server::pubkey_to_npub;
use crate::plot::{PlotData, PlotOwner};
use crate::save::{read_world_save, world_dir, WorldsRootGuard};
use crate::server::GameServer;
use crate::succession::{SavedSuccession, Succession, GONE_SECS};
use crate::world::World;

const DAY: u64 = 86_400;

fn owner_plot(npub: &str, x: i32, y: i32, z: i32) -> PlotData {
    PlotData::from_marker(PlotOwner::Npub(npub.into()), x, y, z)
}

/// A server whose joiners' marker claims count: a dedicated one (no host client). `start_open_server` is an owning host with a local client, the
/// `--no-lend` shape, where a joiner's claim is deliberately not taken.
fn start_claiming_server(tag: &str) -> crate::hosted_server::HostedServer {
    crate::hosted_server::HostedServer::start(
        0,
        format!("plot-succession-{tag}-{}", std::process::id()),
        42,
        0,
        crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
    )
    .expect("hosted server starts")
}

fn now() -> u64 {
    crate::succession::now_unix_secs().expect("native has a wall clock")
}

// ── Claims ──────────────────────────────────────────────────────────

#[test]
fn a_no_lend_hosts_joiner_marker_claims_nothing_so_the_mirror_cannot_wipe_a_promise() {
    let mut hs = start_open_server("s5-no-lend");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    hs.server.players[slot].verified_pubkey = Some([0x3c; 32]);
    let marker = cell_beside(&hs, slot, 2, 0, 0);
    hs.server.world.set_block(marker.0, marker.1, marker.2, block::AIR);
    send_edits(&hs, &client, slot, 1, &[(marker, block::PLOT_MARKER)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(marker.0, marker.1, marker.2), block::PLOT_MARKER, "a plain block");
    assert!(hs.server.world.plots.is_empty(), "no Plot to be wiped a tick later");
}

#[test]
fn a_verified_joiners_plot_marker_claims_a_plot_in_its_npub_and_breaking_it_releases() {
    let mut hs = start_claiming_server("s5-claim");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let pk = [0x3c; 32];
    hs.server.players[slot].verified_pubkey = Some(pk);
    let npub = pubkey_to_npub(&pk);
    let marker = cell_beside(&hs, slot, 2, 0, 0);
    hs.server.world.set_block(marker.0, marker.1, marker.2, block::AIR);

    send_edits(&hs, &client, slot, 1, &[(marker, block::PLOT_MARKER)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(marker.0, marker.1, marker.2), block::PLOT_MARKER);
    assert_eq!(hs.server.world.plots.len(), 1, "one claim");
    let plot = &hs.server.world.plots[0];
    assert_eq!(plot.owner, PlotOwner::Npub(npub.clone()));
    assert_eq!(plot.marker, marker);
    assert_eq!((plot.min_x, plot.max_x), (marker.0 - 16, marker.0 + 16), "the same 33 x 33 as a seat's");

    // Re-sending the same marker (the same block) claims nothing more.
    send_edits(&hs, &client, slot, 2, &[(marker, block::PLOT_MARKER)]);
    hs.tick();
    assert_eq!(hs.server.world.plots.len(), 1);

    // Breaking it releases the claim: that wiring is old, this is its Npub case.
    send_edits(&hs, &client, slot, 3, &[(marker, block::AIR)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(marker.0, marker.1, marker.2), block::AIR);
    assert!(hs.server.world.plots.is_empty(), "released");
}

#[test]
fn a_guests_plot_marker_stays_a_plain_block_that_claims_nothing() {
    let mut hs = start_claiming_server("s5-guest");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    assert!(hs.server.players[slot].verified_pubkey.is_none());
    let marker = cell_beside(&hs, slot, 2, 0, 0);
    hs.server.world.set_block(marker.0, marker.1, marker.2, block::AIR);
    send_edits(&hs, &client, slot, 1, &[(marker, block::PLOT_MARKER)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(marker.0, marker.1, marker.2), block::PLOT_MARKER, "it is placed");
    assert!(hs.server.world.plots.is_empty(), "and claims nothing");
}

#[test]
fn another_joiner_and_a_guest_cannot_build_or_break_in_a_joiners_plot_and_the_owner_can() {
    let mut hs = start_claiming_server("s5-protect");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let owner_pk = [0x3c; 32];
    hs.server.players[slot].verified_pubkey = Some(owner_pk);
    let marker = cell_beside(&hs, slot, 2, 0, 0);
    let inside = cell_beside(&hs, slot, 1, 0, 1);
    hs.server.world.set_block(marker.0, marker.1, marker.2, block::AIR);
    hs.server.world.set_block(inside.0, inside.1, inside.2, block::AIR);
    send_edits(&hs, &client, slot, 1, &[(marker, block::PLOT_MARKER)]);
    hs.tick();
    send_edits(&hs, &client, slot, 2, &[(inside, block::STONE)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(inside.0, inside.1, inside.2), block::STONE, "the owner builds in it");
    let _ = block_changes_seen(&client);

    // Another verified joiner: refused to place, to break, and to break the marker; each ghost is undone.
    hs.server.players[slot].verified_pubkey = Some([0x4d; 32]);
    let spare = cell_beside(&hs, slot, 1, 0, -1);
    hs.server.world.set_block(spare.0, spare.1, spare.2, block::AIR);
    send_edits(&hs, &client, slot, 3, &[(spare, block::STONE)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(spare.0, spare.1, spare.2), block::AIR, "cannot place");
    send_edits(&hs, &client, slot, 4, &[(inside, block::AIR)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(inside.0, inside.1, inside.2), block::STONE, "cannot break");
    send_edits(&hs, &client, slot, 5, &[(marker, block::AIR)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(marker.0, marker.1, marker.2), block::PLOT_MARKER, "cannot release it either");
    assert_eq!(hs.server.world.plots.len(), 1);
    let back = block_changes_seen(&client);
    assert!(back.iter().any(|bc| (bc.x, bc.y, bc.z) == inside && bc.new_block == block::STONE), "un-ghosted: {back:?}");

    // A guest: the same.
    hs.server.players[slot].verified_pubkey = None;
    send_edits(&hs, &client, slot, 6, &[(inside, block::AIR)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(inside.0, inside.1, inside.2), block::STONE, "nor a guest");

    // The owner again: breaks its own block.
    hs.server.players[slot].verified_pubkey = Some(owner_pk);
    send_edits(&hs, &client, slot, 7, &[(inside, block::AIR)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(inside.0, inside.1, inside.2), block::AIR, "the owner may");
}

#[test]
fn a_claim_that_would_overlap_another_owners_plot_is_refused_and_the_ghost_put_back() {
    let mut hs = start_claiming_server("s5-overlap");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let pk = [0x3c; 32];
    hs.server.players[slot].verified_pubkey = Some(pk);
    let marker = cell_beside(&hs, slot, 2, 0, 0);
    hs.server.world.set_block(marker.0, marker.1, marker.2, block::AIR);
    // Someone else's plot whose near edge is 12 blocks to the +x of the player:
    // the marker's own cell is outside it, its footprint is not.
    hs.server.world.plots.push(owner_plot("npub1someoneelse", marker.0 + 28, marker.1, marker.2));
    let _ = block_changes_seen(&client);

    send_edits(&hs, &client, slot, 1, &[(marker, block::PLOT_MARKER)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(marker.0, marker.1, marker.2), block::AIR, "not placed");
    assert_eq!(hs.server.world.plots.len(), 1, "no new claim");
    let back = block_changes_seen(&client);
    assert!(
        back.iter().any(|bc| (bc.x, bc.y, bc.z) == marker && bc.new_block == block::AIR),
        "the ghost is put back: {back:?}"
    );

    // 34 apart clears two 33-wide squares: the same marker is accepted.
    hs.server.world.plots[0] = owner_plot("npub1someoneelse", marker.0 + 40, marker.1, marker.2);
    send_edits(&hs, &client, slot, 2, &[(marker, block::PLOT_MARKER)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(marker.0, marker.1, marker.2), block::PLOT_MARKER);
    assert_eq!(hs.server.world.plots.len(), 2);
}

#[test]
fn a_claim_beside_the_owners_own_plot_is_allowed_and_a_commons_refuses_a_joiners_claim() {
    let mut hs = start_claiming_server("s5-own-commons");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let pk = [0x3c; 32];
    hs.server.players[slot].verified_pubkey = Some(pk);
    let npub = pubkey_to_npub(&pk);
    let marker = cell_beside(&hs, slot, 2, 0, 0);
    hs.server.world.set_block(marker.0, marker.1, marker.2, block::AIR);
    // The same npub's plot nearby overlaps the new footprint: that is its own land.
    hs.server.world.plots.push(owner_plot(&npub, marker.0 + 28, marker.1, marker.2));
    send_edits(&hs, &client, slot, 1, &[(marker, block::PLOT_MARKER)]);
    hs.tick();
    assert_eq!(hs.server.world.plots.len(), 2, "your own neighbour is no conflict");

    // A Commons beside it refuses a claim whose footprint touches it.
    let mut hs = start_claiming_server("s5-commons");
    let (client, slot) = join_guest(&mut hs, "Visitor");
    hs.server.players[slot].verified_pubkey = Some(pk);
    let marker = cell_beside(&hs, slot, 2, 0, 0);
    hs.server.world.set_block(marker.0, marker.1, marker.2, block::AIR);
    hs.server.world.set_block(marker.0 + 28, marker.1, marker.2, block::COMMONS_STONE);
    send_edits(&hs, &client, slot, 1, &[(marker, block::PLOT_MARKER)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(marker.0, marker.1, marker.2), block::AIR);
    assert!(hs.server.world.plots.is_empty());
}

#[test]
fn creative_does_not_bypass_the_claim_rule_for_a_joiner() {
    let mut hs = start_claiming_server("s5-creative");
    hs.server.set_play_mode(crate::play_mode::PlayMode::Creative);
    let (client, slot) = join_guest(&mut hs, "Visitor");
    let pk = [0x3c; 32];
    hs.server.players[slot].verified_pubkey = Some(pk);
    let marker = cell_beside(&hs, slot, 2, 0, 0);
    hs.server.world.set_block(marker.0, marker.1, marker.2, block::AIR);
    hs.server.world.plots.push(owner_plot("npub1someoneelse", marker.0 + 28, marker.1, marker.2));
    send_edits(&hs, &client, slot, 1, &[(marker, block::PLOT_MARKER)]);
    hs.tick();
    assert_eq!(hs.server.world.get_block(marker.0, marker.1, marker.2), block::AIR, "refused even in Creative");
}

#[test]
fn a_local_seat_is_a_stranger_to_a_joiners_plot() {
    // The local break and place arms ask `plot::is_in_foreign_plot` (seat index):
    // an Npub owner is never a seat.
    let plots = vec![owner_plot("npub1joiner", 0, 64, 0)];
    for seat in 0..4 {
        assert!(crate::plot::is_in_foreign_plot(&plots, 3, 3, seat), "seat {seat} is foreign to it");
    }
    assert!(!crate::plot::is_in_foreign_plot(&plots, 500, 500, 0), "open land is nobody's");
    // And a seat's claim beside it is a conflict, as the other way round.
    assert!(crate::plot::claim_would_conflict(&plots, &PlotOwner::LocalPlayer(0), 20, 0));
}

// ── Succession on the server's tick ─────────────────────────────────

/// A server with a joiner present and an AWAY owner's Plot (marker, an authored
/// block inside, and another builder's block inside).
fn away_owner_world(tag: &str) -> (crate::hosted_server::HostedServer, crate::transport::ChannelClientTransport, usize, (i32, i32, i32)) {
    let mut hs = start_open_server(tag);
    let (client, slot) = join_guest(&mut hs, "Visitor");
    hs.server.players[slot].verified_pubkey = Some([0x4d; 32]);
    let at = cell_beside(&hs, slot, 6, 0, 0);
    hs.server.world.place_player_block_as(at.0, at.1, at.2, block::PLOT_MARKER, Some(&AuthorKey::Npub("npub1away".into())));
    hs.server.world.plots.push(owner_plot("npub1away", at.0, at.1, at.2));
    (hs, client, slot, at)
}

/// A table whose owner was last seen `days` ago while the world was RUNNING until
/// a few seconds ago (the world's clock check shifts nothing; the absence is real).
fn last_seen_days_ago(npub: &str, days: u64) -> Succession {
    Succession::from_saved(&SavedSuccession {
        last_seen: vec![(npub.to_string(), now() - days * DAY)],
        passed_on: vec![],
        last_running: now() - 10,
    })
}

#[test]
fn an_owner_away_sixty_real_days_loses_the_plot_to_a_commons_a_joiner_sees() {
    let (mut hs, client, _slot, at) = away_owner_world("s5-succession");
    let away = AuthorKey::Npub("npub1away".into());
    let other = AuthorKey::Npub("npub1other".into());
    let theirs = (at.0 + 2, at.1, at.2 + 2);
    let others = (at.0 - 3, at.1, at.2 + 3);
    hs.server.world.place_player_block_as(theirs.0, theirs.1, theirs.2, block::STONE, Some(&away));
    hs.server.world.place_player_block_as(others.0, others.1, others.2, block::STONE, Some(&other));
    hs.server.world.plot_succession = last_seen_days_ago("npub1away", 61);
    let _ = block_changes_seen(&client);

    hs.tick();

    assert!(hs.server.world.plots.is_empty(), "the Plot is released");
    assert_eq!(hs.server.world.get_block(at.0, at.1, at.2), block::COMMONS_STONE, "the marker is a Commons Stone");
    assert_eq!(hs.server.world.commons_areas().len(), 1, "a real Commons");
    let seen = block_changes_seen(&client);
    assert!(
        seen.iter().any(|bc| (bc.x, bc.y, bc.z) == at && bc.new_block == block::COMMONS_STONE),
        "the joiner is sent the stone: {seen:?}"
    );
    assert_eq!(hs.server.world.author_at(theirs.0, theirs.1, theirs.2), None, "the owner's authorship in the footprint is gone");
    assert!(hs.server.world.is_placed(theirs.0, theirs.1, theirs.2), "the block is still built");
    assert_eq!(hs.server.world.author_at(others.0, others.1, others.2), Some(&other), "another builder's stays theirs");
    assert!(hs.server.world.plot_succession.has_passed_on(&at));
    // The Commons now protects the other builder's block, and not the old owner's.
    assert!(hs.server.world.commons_protects(others.0, others.1, others.2, Some(&AuthorKey::Npub("npub1x".into()))));
    assert!(!hs.server.world.commons_protects(theirs.0, theirs.1, theirs.2, Some(&AuthorKey::Npub("npub1x".into()))));
}

#[test]
fn fifty_nine_days_away_keeps_the_plot_and_a_present_owner_is_never_passed_over() {
    let (mut hs, _client, _slot, at) = away_owner_world("s5-not-yet");
    hs.server.world.plot_succession = last_seen_days_ago("npub1away", 59);
    hs.tick();
    assert_eq!(hs.server.world.plots.len(), 1, "59 days is not 60");
    assert_eq!(hs.server.world.get_block(at.0, at.1, at.2), block::PLOT_MARKER);

    // The owner is a joiner in the world when the check runs: stamped, kept.
    let mut hs = start_open_server("s5-present");
    let (_client, slot) = join_guest(&mut hs, "Visitor");
    let pk = [0x3c; 32];
    hs.server.players[slot].verified_pubkey = Some(pk);
    let npub = pubkey_to_npub(&pk);
    let at = cell_beside(&hs, slot, 6, 0, 0);
    hs.server.world.place_player_block(at.0, at.1, at.2, block::PLOT_MARKER);
    hs.server.world.plots.push(owner_plot(&npub, at.0, at.1, at.2));
    hs.server.world.plot_succession = last_seen_days_ago(&npub, 200);
    hs.tick();
    assert_eq!(hs.server.world.plots.len(), 1, "someone who is here has not been away");
    let saved = hs.server.world.plot_succession.to_saved().unwrap();
    assert!(saved.last_seen[0].1 >= now() - 60, "and is stamped now");
}

#[test]
fn a_world_from_before_the_field_never_passes_a_plot_at_once_and_a_seats_plot_never_passes() {
    let (mut hs, _client, _slot, at) = away_owner_world("s5-upgrade");
    hs.server.world.plots.push(PlotData::from_marker(PlotOwner::LocalPlayer(0), at.0 + 100, at.1, at.2));
    // No table at all (an upgraded world, just loaded: nothing checked yet).
    hs.server.world.plot_succession.clear();
    hs.tick();
    assert_eq!(hs.server.world.plots.len(), 2, "unrecorded owners are stamped, not passed");
    let saved = hs.server.world.plot_succession.to_saved().expect("stamped");
    assert_eq!(saved.last_seen.len(), 1, "only the joiner-owner is tracked");
    assert_eq!(saved.last_seen[0].0, "npub1away");
    assert!(matches!(hs.server.world.plots[1].owner, PlotOwner::LocalPlayer(0)));
}

#[test]
fn succession_with_an_injected_clock_and_no_clock() {
    // The pure phase takes `now`; with none (wasm) nothing is stamped or passed.
    let mut w = World::new();
    w.place_player_block(0, 64, 0, block::PLOT_MARKER);
    w.plots.push(owner_plot("npub1a", 0, 64, 0));
    let t0 = 1_800_000_000u64;
    let step = |w: &mut World, tick: &mut u64, now: &mut u64, until: u64| {
        let mut out = Vec::new();
        while *now < until {
            *now = (*now + 200).min(until);
            *tick += 1;
            out.extend(crate::succession::phase(w, *tick, ((*now - t0) / DAY) as u32 + 1, Some(*now), Vec::new));
        }
        out
    };
    let (mut tick, mut at) = (1u64, t0);
    assert!(crate::succession::phase(&mut w, tick, 1, Some(t0), Vec::new).is_empty());
    assert!(step(&mut w, &mut tick, &mut at, t0 + GONE_SECS - 1).is_empty());
    let none = crate::succession::phase(&mut w, 3, 3, None, Vec::new);
    assert!(none.is_empty() && w.plots.len() == 1, "no clock, nothing passes");
    assert_eq!(step(&mut w, &mut tick, &mut at, t0 + GONE_SECS + DAY).len(), 1, "sixty running days pass it");
}

#[test]
fn sixty_one_days_offline_passes_nothing_on_the_first_server_tick_after_the_load() {
    // The save says: owner last seen 61 days ago, world last ran 61 days ago.
    let (mut hs, _client, _slot, at) = away_owner_world("s5-offline");
    let long_ago = now() - 61 * DAY;
    hs.server.world.plot_succession = Succession::from_saved(&SavedSuccession {
        last_seen: vec![("npub1away".into(), long_ago)],
        passed_on: vec![],
        last_running: long_ago + 5,
    });
    hs.tick();
    assert_eq!(hs.server.world.plots.len(), 1, "downtime is not absence");
    assert_eq!(hs.server.world.get_block(at.0, at.1, at.2), block::PLOT_MARKER);
    let seen = hs.server.world.plot_succession.last_seen_of("npub1away").unwrap();
    assert!(seen >= now() - 20, "the row was pushed forward by the gap: {seen}");
}

#[test]
fn sixty_one_days_running_with_the_owner_absent_passes_the_plot_on_the_server() {
    let (mut hs, _client, _slot, at) = away_owner_world("s5-running");
    // Injected clock: world ran at T, owner last seen 61 days before it.
    let t = 1_900_000_000u64;
    hs.server.succession_clock = Some(t);
    hs.server.world.plot_succession = Succession::from_saved(&SavedSuccession {
        last_seen: vec![("npub1away".into(), t - 61 * DAY)],
        passed_on: vec![],
        last_running: t - 1,
    });
    hs.tick();
    assert!(hs.server.world.plots.is_empty());
    assert_eq!(hs.server.world.get_block(at.0, at.1, at.2), block::COMMONS_STONE);
}

#[test]
fn a_solo_session_of_sixty_one_days_then_hosting_passes_nothing() {
    let (mut hs, _client, _slot, at) = away_owner_world("s5-solo");
    let t = 1_900_000_000u64;
    hs.server.world.plot_succession = Succession::from_saved(&SavedSuccession {
        last_seen: vec![("npub1away".into(), t)],
        passed_on: vec![],
        last_running: t,
    });
    // 61 days at the keyboard alone: the world is closed to joiners (a local-only
    // authority), the phase has no clock, and nothing is stamped.
    hs.server.open_to_joiners = false;
    hs.server.succession_clock = Some(t + 61 * DAY);
    hs.tick();
    assert_eq!(hs.server.world.plot_succession.last_seen_of("npub1away"), Some(t), "solo time moved nothing");
    assert_eq!(hs.server.world.plots.len(), 1);
    // Then the same world is hosted: the first hosted tick shifts the whole gap.
    hs.server.open_to_joiners = true;
    hs.server.succession_clock = Some(t + 61 * DAY + 200);
    hs.tick();
    assert_eq!(hs.server.world.plots.len(), 1, "the owner was never able to visit; nothing passes");
    assert_eq!(hs.server.world.get_block(at.0, at.1, at.2), block::PLOT_MARKER);
    assert_eq!(hs.server.world.plot_succession.last_seen_of("npub1away"), Some(t + 61 * DAY + 200), "shifted by the whole solo gap");
}

// ── Save ────────────────────────────────────────────────────────────

fn saved_world_save(world: &str) -> crate::save::WorldSave {
    read_world_save(&std::fs::read(world_dir(world).join("world.dat")).unwrap()).unwrap()
}

#[test]
fn a_passed_on_plot_stays_passed_on_after_a_save_and_reload() {
    let _g = WorldsRootGuard::new("s5-reload");
    let mut s = GameServer::new(0, "w".into(), 42);
    let away = AuthorKey::Npub("npub1away".into());
    let at = (3, 64, 3);
    let theirs = (5, 64, 5);
    s.world.place_player_block_as(at.0, at.1, at.2, block::PLOT_MARKER, Some(&away));
    s.world.place_player_block_as(theirs.0, theirs.1, theirs.2, block::STONE, Some(&away));
    s.world.plots.push(owner_plot("npub1away", at.0, at.1, at.2));
    s.world.plot_succession = last_seen_days_ago("npub1away", 61);
    s.tick();
    assert_eq!(s.world.get_block(at.0, at.1, at.2), block::COMMONS_STONE);
    assert!(s.pending_block_changes.iter().any(|c| (c.x, c.y, c.z) == at && c.new_block == block::COMMONS_STONE), "queued for joiners");
    s.try_save().unwrap();

    let saved = saved_world_save("w");
    let table = saved.plot_succession.as_ref().expect("the table is written");
    assert_eq!(table.passed_on, vec![at], "the passed-on marker is remembered");
    assert!(saved.plots.is_empty(), "no Plot is saved for it");

    let mut back = GameServer::new(0, "w".into(), 42);
    back.initial_load().unwrap();
    assert!(back.world.plots.is_empty(), "stays released");
    assert_eq!(back.world.get_block(at.0, at.1, at.2), block::COMMONS_STONE, "stays a Commons");
    assert!(back.world.plot_succession.has_passed_on(&at));
    assert_eq!(back.world.author_at(theirs.0, theirs.1, theirs.2), None, "and the old owner's authorship stays gone");
    assert!(back.world.is_placed(theirs.0, theirs.1, theirs.2));
    assert!(crate::succession::in_passed_on_commons(&back.world, at.0 as f32 + 3.0, at.2 as f32));
    // A check after the load finds nothing more to do.
    back.tick();
    assert!(back.world.plots.is_empty());
}

#[test]
fn the_table_round_trips_through_a_real_save_and_an_empty_table_is_not_written() {
    let _g = WorldsRootGuard::new("s5-roundtrip");
    let mut s = GameServer::new(0, "w".into(), 42);
    s.world.place_player_block(5, 64, 5, block::STONE);
    s.try_save().unwrap();
    assert!(saved_world_save("w").plot_succession.is_none(), "nothing recorded: the field is absent");

    s.world.plots.push(owner_plot("npub1zed", 1000, 64, 0));
    s.world.plots.push(owner_plot("npub1amy", 2000, 64, 0));
    s.world.plot_succession.prepare(&s.world.plots.clone(), 1_800_000_000);
    s.try_save().unwrap();
    let saved = saved_world_save("w").plot_succession.expect("written");
    assert_eq!(saved.last_seen, vec![("npub1amy".to_string(), 1_800_000_000), ("npub1zed".to_string(), 1_800_000_000)], "sorted");

    let mut back = GameServer::new(0, "w".into(), 42);
    back.initial_load().unwrap();
    assert_eq!(back.world.plot_succession.to_saved(), Some(saved));
}

#[test]
fn the_bytes_are_the_same_whatever_order_the_owners_were_stamped_in() {
    let plots = [owner_plot("npub1b", 0, 64, 0), owner_plot("npub1a", 100, 64, 0), owner_plot("npub1c", 200, 64, 0)];
    let bytes = |order: &[&str]| {
        let mut s = Succession::default();
        for o in order {
            s.note_present(&plots, &[o.to_string()], 1_800_000_000);
        }
        bincode::serialize(&s.to_saved().unwrap()).unwrap()
    };
    assert_eq!(bytes(&["npub1a", "npub1b", "npub1c"]), bytes(&["npub1c", "npub1b", "npub1a"]));
}

#[test]
fn the_server_digest_covers_the_table() {
    let digest = |days: u64| {
        let _g = WorldsRootGuard::new(&format!("s5-digest-{days}"));
        let mut s = GameServer::new(0, "w".into(), 42);
        s.world.plot_succession = Succession::from_saved(&SavedSuccession { last_seen: vec![("npub1a".into(), 1_000 + days)], passed_on: vec![], last_running: 0 });
        s.sim_digest()
    };
    assert_eq!(digest(1), digest(1));
    assert_ne!(digest(1), digest(2), "a changed last-seen time changes the digest");
}

#[test]
fn a_footer_65_and_66_save_loads_with_no_table() {
    // Footer 66 (S8) and 65 (S4) writers had 59 fields: the same bytes without the new last one.
    for footer in [65u32, 66] {
        let mut save = crate::save::minimal_world_save_for_tests(7);
        save.plot_succession = None;
        save.site_boards = None;
        let mut body = bincode::serialize(&save).unwrap();
        assert_eq!(body.pop(), Some(0), "the absent weather hold is one None byte");
        assert_eq!(body.pop(), Some(0), "the absent site boards are one None byte");
        assert_eq!(body.pop(), Some(0), "the absent field is one None byte");
        body.extend_from_slice(&crate::save_format::footer_bytes(footer));
        let (old, damaged) = crate::save::read_world_save_reporting(&body).expect("an older save loads");
        assert!(!damaged, "footer {footer}");
        assert!(old.plot_succession.is_none());
        let mut w = World::new();
        w.plot_succession.note_present(&[owner_plot("npub1a", 0, 64, 0)], &["npub1a".into()], 5);
        crate::save::apply_world_save_state(&mut w, &old);
        assert!(w.plot_succession.is_empty(), "restoring a save without the field leaves nothing recorded");
    }
}

#[test]
fn an_invalid_row_is_dropped_with_a_warning_never_a_refusal_to_load() {
    let mut save = crate::save::minimal_world_save_for_tests(8);
    save.plot_succession = Some(SavedSuccession {
        last_seen: vec![("npub1ok".into(), 5), ("npub1ok".into(), 6), (String::new(), 7)],
        passed_on: vec![(1, 2, 3), (1, 2, 3)],
        last_running: 0,
    });
    let bytes = crate::save_format::encode_world_save(&save).unwrap();
    let (back, damaged) = crate::save::read_world_save_reporting(&bytes).expect("the world still loads");
    assert!(!damaged);
    let mut w = World::new();
    crate::save::apply_world_save_state(&mut w, &back);
    let kept = w.plot_succession.to_saved().unwrap();
    assert_eq!(kept.last_seen, vec![("npub1ok".to_string(), 5)]);
    assert_eq!(kept.passed_on, vec![(1, 2, 3)]);
}
