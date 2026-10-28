//! Real notes and a real host broadcast force authoritative work across frames.
use super::*;

fn pending_unloaded(j: &Joined) -> Vec<(i32, i32)> {
    let mut columns: Vec<_> = j
        .host
        .loaded_columns
        .iter()
        .copied()
        .filter(|column| {
            !j.hg.state.loaded_columns.contains(column)
                && j.hg.state.chunk_intake.has_pending_check(*column)
        })
        .collect();
    columns.sort_unstable();
    columns
}

fn acknowledge_actual_world(j: &mut Joined) {
    let body = &j.hs.server.players[j.slot];
    let input = protocol::InputPacket {
        x: body.player.pos.x,
        y: body.player.pos.y,
        z: body.player.pos.z,
        health: body.combat.health,
        chunk_ack: j.hg.state.chunk_intake.applied(),
        render_distance: 8,
        ..Default::default()
    };
    j.hg.state
        .remote_client
        .as_mut()
        .unwrap()
        .send_input(&input)
        .unwrap();
}

#[test]
#[ignore = "needs a GPU adapter and an actual multi-column authoritative burst"]
fn game_harness_ordered_park_paused_carried_world_blocks_raw_reads_until_real_application_finishes()
{
    let _worlds = crate::save::WorldsRootGuard::new("park-carried-world");
    let mut j = joined("park-carried-world");
    j.hg.state.mode = crate::GameMode::Paused {
        confirm_quit: false,
        confirm_creative: false,
    };
    // Adopt the real original admission's generator/rules before choosing any
    // fresh columns. Neither the slot nor either application Inbox is replaced.
    j.hg.state.handover_test_adopt_joined_world();
    assert_eq!(j.hg.state.biome_gen.seed, j.hs.server.biome_gen.seed);
    assert_eq!(
        j.hg.state.biome_gen.generator(),
        j.hs.server.biome_gen.generator()
    );
    let centre = crate::chunk_stream::column_of(j.hs.server.players[j.slot].player.pos);
    // Declared natural host terrain, using its existing generator/loader, so
    // the normal source sends genuine Local notes for previously unseen land.
    for dx in -8..=8 {
        for dz in -8..=8 {
            let column = (centre.0 + dx, centre.1 + dz);
            if !j.host.loaded_columns.contains(&column) {
                crate::chunk_stream::load_column_blocks(
                    &mut j.host.world,
                    column.0,
                    column.1,
                    &j.hs.server.biome_gen,
                    false,
                );
                j.host.loaded_columns.insert(column);
            }
        }
    }
    let needed = 2 * crate::chunk_stream::FORCED_CHECKS_PER_FRAME + 1;
    for _ in 0..256 {
        acknowledge_actual_world(&mut j);
        j.host.lend_tick(&mut j.hs);
        j.hg.state.handover_test_network_receive();
        if pending_unloaded(&j).len() >= needed {
            break;
        }
    }
    // Finish the actual prior ACK stream before consent; no subsequent input
    // send is used to manufacture the carried-work observation or the park.
    j.host.lend_tick(&mut j.hs);
    j.hg.state.handover_test_network_receive();
    assert!(j.hg.state.chunk_intake.column_mismatch().is_none());
    let columns: Vec<_> = pending_unloaded(&j).into_iter().take(needed).collect();
    assert_eq!(
        columns.len(),
        needed,
        "at least17 real Local notes remain pending in unloaded columns"
    );
    let mut src = source(&mut j);
    j.hg.state.handover_test_network_receive();
    j.hg.state.poll_handover();
    assert_eq!(
        recipient_stage(&j.hg),
        Some(RecipientStage::AwaitingConsent)
    );
    arm_consent(&mut j.hg);
    j.hg.state.handover_click = Some(UiAction::Accept);
    j.hg.frames(1);
    assert_eq!(recipient_stage(&j.hg), Some(RecipientStage::Draining));
    assert!(!j.hg.state.remote_client.as_ref().unwrap().is_parked());
    assert!(
        columns
            .iter()
            .all(|column| !j.hg.state.loaded_columns.contains(column)
                && j.hg.state.chunk_intake.has_pending_check(*column))
    );
    let y = (crate::world::MAX_CHUNK_Y + 1) * crate::chunk::CHUNK_SIZE as i32 - 2;
    let changes: Vec<_> = columns
        .iter()
        .map(|&(cx, cz)| {
            let cell = (
                cx * crate::chunk::CHUNK_SIZE as i32 + 1,
                y,
                cz * crate::chunk::CHUNK_SIZE as i32 + 1,
            );
            assert_eq!(
                j.host.world.get_block(cell.0, cell.1, cell.2),
                crate::block::AIR
            );
            j.host
                .world
                .set_block(cell.0, cell.1, cell.2, crate::block::GLASS);
            protocol::BlockChange::with_meta(
                cell.0,
                cell.1,
                cell.2,
                crate::block::GLASS,
                j.host.world.meta_at(cell.0, cell.1, cell.2),
            )
        })
        .collect();
    // This is the host's existing /we-style authoritative broadcast path.
    // It emits one real normal StateUpdate before the source learns consent.
    j.hs.queue_host_broadcast(changes.clone());
    j.host.lend_tick(&mut j.hs);
    src.pump(&mut j.hs, live(), now()).unwrap();
    assert_eq!(src.stage(), SourceStage::AwaitingParked);
    assert!(j.hs.handover_frozen());
    let initial_reads = j.received.load(Ordering::Relaxed);
    j.hg.frames(1); // Existing early network_receive/apply in actual Paused Draining.
    assert!(matches!(j.hg.state.mode, crate::GameMode::Paused { .. }));
    assert!(
        j.hg.state.chunk_intake.carried_len() > 0,
        "the actual authoritative suffix must exist"
    );
    let burst_reads = j.received.load(Ordering::Relaxed);
    assert_eq!(
        burst_reads,
        initial_reads + 1,
        "only the real ordinary burst was read"
    );
    for _ in 0..3 {
        let before = j.hg.state.chunk_intake.carried_len();
        let remote = j.hg.state.remote_client.as_mut().unwrap();
        remote.poll();
        remote.poll();
        assert!(remote.handover_drain_fence(src.transfer()).is_none());
        assert_eq!(
            j.received.load(Ordering::Relaxed),
            burst_reads,
            "queued notice/fence stays raw while actual world work is carried"
        );
        assert_eq!(j.hg.state.chunk_intake.carried_len(), before);
    }
    assert!(!j.hg.state.remote_client.as_ref().unwrap().is_parked());
    for _ in 0..needed {
        if !j.hg.state.chunk_intake.has_carried_world_work() {
            break;
        }
        j.hg.state.handover_test_network_receive();
        assert_eq!(
            j.received.load(Ordering::Relaxed),
            burst_reads,
            "application drains without another raw read"
        );
        assert!(!j.hg.state.remote_client.as_ref().unwrap().is_parked());
    }
    assert_eq!(j.hg.state.chunk_intake.carried_len(), 0);
    assert!(j.hg.state.chunk_intake.column_mismatch().is_none());
    for change in &changes {
        assert_eq!(
            j.hg.state.world.get_block(change.x, change.y, change.z),
            crate::block::GLASS
        );
    }
    for _ in 0..32 {
        j.hg.frames(1);
        if j.hg.state.remote_client.as_ref().unwrap().is_parked() {
            break;
        }
    }
    assert!(
        j.hg.state.remote_client.as_ref().unwrap().is_parked(),
        "actual park follows full normal authoritative application"
    );
    src.pump(&mut j.hs, live(), now()).unwrap();
    assert_eq!(
        src.stage(),
        SourceStage::Frozen,
        "source verified the genuinely parked recipient's proof"
    );
    assert!(
        !src.test_current_cut_committed(),
        "this control proves pre-current park, not complete carry"
    );
    eprintln!(
        "actual carried-world park: columns={} burst_reads={} final_reads={} carried={}",
        columns.len(),
        burst_reads - initial_reads,
        j.received.load(Ordering::Relaxed) - initial_reads,
        j.hg.state.chunk_intake.carried_len()
    );
    src.abort(&mut j.hs, "carried-world control finished");
}
