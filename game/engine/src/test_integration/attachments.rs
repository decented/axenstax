//! C3c-3b (2026-10-09, protocol v84) — face attachments live in
//! the server's world (Spec 04 §4.2f "Face attachments").
//!
//! Driven through the C3b-2 rig (`block_use::Rig`): a REAL `HostedServer`
//! over the in-process transport (a dedicated server, or a lending host) and
//! joiners whose client half is the real one — its window, its requests in
//! flight with their claims, the outcomes and grants applied in arrival
//! order, and a world of its own that takes in the server's attachment stream
//! (`chunk_intake::apply_attachment_change`). An attach or a peel is sent as
//! `GameState::send_attach` / `send_detach` send it; what the joined client's
//! own arms do (send, and change nothing) is the GPU harness's to drive.

use crate::block;
use crate::item::{Item, ItemStack};
use crate::item_actions::ItemNote;
use crate::joiner_actions::{Asked, Pending};
use crate::mesh::Face;
use crate::protocol::{self, ItemAction, ItemActionOutcomePacket, PushedAttachment};
use crate::transport::ClientTransport;
use crate::world::FaceAttachment;

use super::block_use::{Client, Rig, accepted, refused};

const TOP: usize = 0;
/// The face of a block two ahead (-z) of the joiners that looks at them.
const SOUTH: usize = 3;

impl Client {
    /// `GameState::send_attach`: the block in slot `hot` on face `face` of
    /// `cell`, claimed, recorded, sent. `false` when the claim stops it.
    fn attach(&mut self, cell: [i32; 3], face: usize, hot: usize) -> bool {
        let held = self.inv.hotbar_slot(hot).map(|s| s.item.clone());
        let (held_kind, held_id) = held.as_ref().map_or(protocol::ItemRef::Empty.to_wire(), |i| crate::inventory::item_to_ref(i).to_wire());
        let held_full = held.as_ref().map_or(protocol::WireItem::None, crate::inventory::item_to_wire_full);
        let asked = Asked::Attach { cell, face: face as u8 };
        if !self.actions.can_afford(&self.inv, &self.ui, asked, held.as_ref()) {
            return false;
        }
        let seq = self.actions.record(Pending { kind: asked, mob: None, hotbar_slot: hot, held }, self.input_seq + 1);
        let action = ItemAction::Attach {
            x: cell[0],
            y: cell[1],
            z: cell[2],
            face: face as u8,
            hotbar_slot: hot as u8,
            held_kind,
            held_id,
            held_full,
        };
        let pkt = protocol::ItemActionPacket { seq, action, events_applied: self.events };
        self.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &pkt));
        true
    }

    /// `GameState::send_detach`: peel face `face` of `cell`.
    fn detach(&mut self, cell: [i32; 3], face: usize) -> bool {
        let asked = Asked::Detach { cell, face: face as u8 };
        let seq = self.actions.record(Pending { kind: asked, mob: None, hotbar_slot: 0, held: None }, self.input_seq + 1);
        let action = ItemAction::Detach { x: cell[0], y: cell[1], z: cell[2], face: face as u8 };
        let pkt = protocol::ItemActionPacket { seq, action, events_applied: self.events };
        self.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &pkt));
        true
    }

    /// What this joiner's world copy holds on face `face` of `cell`.
    fn sees(&self, cell: [i32; 3], face: usize) -> Option<&FaceAttachment> {
        self.world.face_attachment_at((cell[0], cell[1], cell[2]), face)
    }
}

impl Rig {
    /// Joiner `n` sends what `send` sends; the server answers once and the
    /// window events and the stream land on every side.
    fn request_once(&mut self, n: usize, send: impl FnOnce(&mut Client) -> bool) -> ItemActionOutcomePacket {
        let before = self.cs[n].outcomes.len();
        assert!(send(&mut self.cs[n]), "sent");
        self.ticks(3);
        assert_eq!(self.cs[n].outcomes.len(), before + 1, "answered once");
        self.cs[n].last().clone()
    }

    /// What the world the server simulates holds on face `face` of `cell`.
    fn holds(&mut self, cell: [i32; 3], face: usize) -> Option<FaceAttachment> {
        self.world().face_attachment_at((cell[0], cell[1], cell[2]), face).cloned()
    }

    /// `att` on face `face` of `cell` in the world the server simulates and,
    /// as that column's push would carry it, in every joiner's copy (as its
    /// render stub).
    fn lay(&mut self, cell: [i32; 3], face: usize, att: FaceAttachment) {
        let pos = (cell[0], cell[1], cell[2]);
        let stub = crate::chunk_push::pushed_attachment(&att);
        self.world().set_face_attachment(pos, face, att);
        for c in &mut self.cs {
            let change = protocol::AttachmentChange { x: cell[0], y: cell[1], z: cell[2], face: face as u8, att: Some(stub) };
            crate::chunk_intake::apply_attachment_change(&mut c.world, &change);
        }
    }

    /// The floor block two ahead (-z) of the joiners' feet.
    fn floor_ahead(&self) -> [i32; 3] {
        [self.at.x.floor() as i32, self.at.y as i32 - 1, self.at.z.floor() as i32 - 2]
    }
}

fn wallpaper() -> Item {
    Item::Block(block::WALLPAPER_RED)
}

fn paper() -> Item {
    Item::Block(block::BLUEPRINT_PAPER)
}

// ─── Attach ────────────────────────────────────────────────────────────────

/// Decision 2 — a joiner paints wallpaper on a lending host's wall: the
/// server's world (the host's own) has it, the joiner's window lost the
/// block (the server's copy too, in lockstep), and both joiners' copies —
/// the painter's included, which changed nothing itself — show it through
/// the stream. The host's client is told to remesh the cell.
#[test]
fn a_joiner_paints_wallpaper_on_the_servers_world_and_every_seat_sees_it() {
    let mut rig = Rig::lent("paint", 2);
    let wall = rig.place(-2, block::STONE);
    rig.give(0, 0, wallpaper(), 2);
    let _ = rig.hs.take_lent_changes();
    assert!(rig.cs[0].sees(wall, SOUTH).is_none());
    let out = rig.request_once(0, |c| c.attach(wall, SOUTH, 0));
    accepted(&out, 1);
    assert_eq!(rig.holds(wall, SOUTH), Some(FaceAttachment::Wallpaper(block::WALLPAPER_RED)), "the server's world");
    assert_eq!(rig.cs[0].count(&wallpaper()), 1, "the painter paid one");
    rig.assert_lockstep(0, "after the paint");
    for n in 0..2 {
        assert_eq!(rig.cs[n].sees(wall, SOUTH), Some(&FaceAttachment::Wallpaper(block::WALLPAPER_RED)), "joiner {n} sees it");
    }
    assert_eq!(
        rig.cs[1].attachments.iter().filter(|a| [a.x, a.y, a.z] == wall).count(),
        1,
        "streamed once: {:?}",
        rig.cs[1].attachments
    );
    let (_, cells) = rig.hs.take_lent_changes();
    assert!(cells.contains(&(wall[0], wall[1], wall[2])), "the host's client remeshes the painted wall: {cells:?}");
}

/// Decision 2 — blank paper lies on a floor's Top face only: on a wall it is
/// refused with the note and nothing changes; on the floor it is laid; a
/// second lay on the covered face is refused with its own note. A creative
/// joiner lays it for nothing, as single-player's creative lay is free.
#[test]
fn a_joiner_lays_blank_paper_on_a_top_face_only() {
    let mut rig = Rig::dedicated("paper", 1);
    let wall = rig.place(-2, block::STONE);
    let floor = rig.floor_ahead();
    rig.give(0, 0, paper(), 3);
    refused(&rig.request_once(0, |c| c.attach(wall, SOUTH, 0)), ItemNote::NotAFloor);
    assert_eq!(rig.holds(wall, SOUTH), None, "a refusal lays nothing");
    assert_eq!(rig.cs[0].count(&paper()), 3, "and takes nothing");
    accepted(&rig.request_once(0, |c| c.attach(floor, TOP, 0)), 1);
    assert_eq!(rig.holds(floor, TOP), Some(FaceAttachment::BlueprintBlank));
    assert_eq!(rig.cs[0].sees(floor, TOP), Some(&FaceAttachment::BlueprintBlank), "the layer's own copy, through the stream");
    assert_eq!(rig.cs[0].count(&paper()), 2);
    refused(&rig.request_once(0, |c| c.attach(floor, TOP, 0)), ItemNote::FaceCovered);
    assert_eq!(rig.cs[0].count(&paper()), 2);
    rig.assert_lockstep(0, "survival lays");
    // Out of reach, and holding something that isn't paper.
    let far = [floor[0], floor[1], floor[2] - 12];
    refused(&rig.request_once(0, |c| c.attach(far, TOP, 0)), ItemNote::OutOfReach);
    rig.give(0, 1, Item::Block(block::STONE), 1);
    refused(&rig.request_once(0, |c| c.attach([floor[0], floor[1], floor[2] + 1], TOP, 1)), ItemNote::NothingToTake);
    // Creative: laid, nothing taken on either side.
    rig.hs.server.play_mode = crate::play_mode::PlayMode::Creative;
    let next = [floor[0] + 1, floor[1], floor[2]];
    let out = rig.request_once(0, |c| c.attach(next, TOP, 0));
    accepted(&out, 0);
    assert_eq!(out.window_event, 0, "no take to apply");
    assert_eq!(rig.holds(next, TOP), Some(FaceAttachment::BlueprintBlank));
    assert_eq!(rig.cs[0].count(&paper()), 2, "creative takes nothing");
    let slot = rig.cs[0].slot;
    assert_eq!(rig.hs.server.players[slot].inventory.slot(0).map(|s| s.count), Some(2), "nor from the server's copy");
}

// ─── Detach ────────────────────────────────────────────────────────────────

/// Decision 3 — a joiner peels its wallpaper: the server's world loses it,
/// the grant gives the block back (the window and the server's copy in
/// lockstep), and every copy loses it through the stream. A second peel of
/// the bare face is refused silently and grants nothing; a laid Blueprint
/// is never lifted by a joiner.
#[test]
fn a_joiner_peels_its_wallpaper_back() {
    let mut rig = Rig::lent("peel", 2);
    let wall = rig.place(-2, block::STONE);
    rig.give(0, 0, wallpaper(), 1);
    accepted(&rig.request_once(0, |c| c.attach(wall, SOUTH, 0)), 1);
    assert_eq!(rig.cs[0].count(&wallpaper()), 0);
    accepted(&rig.request_once(0, |c| c.detach(wall, SOUTH)), 0);
    assert_eq!(rig.holds(wall, SOUTH), None, "gone from the server's world");
    assert_eq!(rig.cs[0].count(&wallpaper()), 1, "the block is granted back");
    rig.assert_lockstep(0, "after the peel");
    for n in 0..2 {
        assert_eq!(rig.cs[n].sees(wall, SOUTH), None, "gone from joiner {n}'s copy");
    }
    refused(&rig.request_once(0, |c| c.detach(wall, SOUTH)), ItemNote::NothingToTake);
    assert_eq!(rig.cs[0].count(&wallpaper()), 1, "nothing granted twice");
    let floor = rig.floor_ahead();
    rig.lay(floor, TOP, FaceAttachment::Blueprint(Box::new(crate::plan::PlanData::debug_3x3_stone())));
    refused(&rig.request_once(0, |c| c.detach(floor, TOP)), ItemNote::NothingToTake);
    assert!(matches!(rig.holds(floor, TOP), Some(FaceAttachment::Blueprint(_))), "a joiner can't lift a Blueprint");
}

// ─── Breaks ────────────────────────────────────────────────────────────────

/// Decision 4 — a joiner breaks a block carrying wallpaper the host laid
/// and a laid Blueprint: the server recovers each ONCE as a real ground item
/// at the cell (one wallpaper block; the Blueprint as a ground Plan with the
/// server's real body), the attachments are gone from every copy, and the
/// joiner's window gains nothing from it (its client grants itself nothing).
#[test]
fn a_joiner_breaking_a_papered_block_spills_each_attachment_once() {
    let mut rig = Rig::dedicated("break", 2);
    let wall = rig.place(-2, block::STONE);
    let plan = crate::plan::PlanData::debug_3x3_stone();
    rig.lay(wall, SOUTH, FaceAttachment::Wallpaper(block::WALLPAPER_RED));
    rig.lay(wall, TOP, FaceAttachment::Blueprint(Box::new(plan.clone())));
    rig.edit(0, (wall[0], wall[1], wall[2]), block::AIR);
    rig.ticks(3);
    assert_eq!(rig.world().get_block(wall[0], wall[1], wall[2]), block::AIR, "the break landed");
    assert_eq!(rig.ground(&wallpaper()), 1, "one wallpaper block on the ground");
    let plans: Vec<_> = rig
        .ecs()
        .query::<&crate::entity::ItemEntity>()
        .iter()
        .filter_map(|(_, it)| match &it.stack.item {
            Item::Plan(p) => Some(p.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(plans.len(), 1, "one ground Plan");
    assert_eq!(plans[0].cells, plan.cells, "with the server's real body");
    for face in [SOUTH, TOP] {
        assert_eq!(rig.holds(wall, face), None, "gone from the server's world");
        for n in 0..2 {
            assert_eq!(rig.cs[n].sees(wall, face), None, "gone from joiner {n}'s copy");
        }
    }
    assert_eq!(rig.cs[0].count(&wallpaper()), 0, "the breaker was granted nothing");
}

/// Decision 1 — a lending host's OWN peel and break (its client's arms, on
/// its world, outside the lend window) reach every joiner through the same
/// stream: the host's world is the server's, and every setter logs.
#[test]
fn a_hosts_own_paint_peel_and_break_reach_every_joiner() {
    let mut rig = Rig::lent("host-hands", 2);
    let wall = rig.place(-2, block::STONE);
    let pos = (wall[0], wall[1], wall[2]);
    let host = rig.host.as_mut().expect("a lending host");
    host.world.set_face_attachment(pos, SOUTH, FaceAttachment::Wallpaper(block::WALLPAPER_RED));
    rig.ticks(2);
    for n in 0..2 {
        assert_eq!(rig.cs[n].sees(wall, SOUTH), Some(&FaceAttachment::Wallpaper(block::WALLPAPER_RED)), "joiner {n} sees the host's paint");
    }
    let host = rig.host.as_mut().expect("a lending host");
    host.world.remove_face_attachment(pos, SOUTH);
    rig.ticks(2);
    for n in 0..2 {
        assert_eq!(rig.cs[n].sees(wall, SOUTH), None, "joiner {n} sees the host's peel");
    }
    let host = rig.host.as_mut().expect("a lending host");
    host.world.set_face_attachment(pos, TOP, FaceAttachment::BlueprintBlank);
    rig.ticks(2);
    assert_eq!(rig.cs[1].sees(wall, TOP), Some(&FaceAttachment::BlueprintBlank));
    // The host's break arm: the shared recovery, then the block goes.
    let host = rig.host.as_mut().expect("a lending host");
    let items = crate::blueprint_attach::take_recoverable_attachments(&mut host.world, pos);
    host.world.set_block(pos.0, pos.1, pos.2, block::AIR);
    assert_eq!(items, vec![paper()], "the host's own recovery, into its own hand");
    rig.ticks(2);
    for n in 0..2 {
        assert_eq!(rig.cs[n].sees(wall, TOP), None, "joiner {n} sees the host's break take it");
    }
}

/// Decision 1 — an attachment change goes only to joiners holding its
/// chunk: one in a chunk nobody was pushed reaches nobody (its push will
/// carry it).
#[test]
fn an_attachment_change_goes_only_to_joiners_holding_its_chunk() {
    let mut rig = Rig::dedicated("sent-set", 1);
    let near = rig.place(-2, block::STONE);
    let far = [near[0] + 160, near[1], near[2]];
    rig.world().set_block(far[0], far[1], far[2], block::STONE);
    rig.world().set_face_attachment((near[0], near[1], near[2]), SOUTH, FaceAttachment::Wallpaper(block::WALLPAPER_RED));
    rig.world().set_face_attachment((far[0], far[1], far[2]), SOUTH, FaceAttachment::Wallpaper(block::WALLPAPER_RED));
    rig.ticks(2);
    let got: Vec<[i32; 3]> = rig.cs[0].attachments.iter().map(|a| [a.x, a.y, a.z]).collect();
    assert_eq!(got, vec![near], "the held chunk's change only");
}

// ─── The capture commit's paper ────────────────────────────────────────────

/// Decision 5 — a joiner's capture commit (`PlanMinted { CaptureCommit }`,
/// its cell the stamped tile) spends that capture's blank paper in the
/// server's world, found by the same flood from the same tile; every copy
/// loses it, and a second capture over that spot finds none.
#[test]
fn a_joiners_capture_commit_spends_the_servers_paper_once() {
    let mut rig = Rig::dedicated("commit", 2);
    let floor = rig.floor_ahead();
    let tiles = [floor, [floor[0] + 1, floor[1], floor[2]]];
    for t in tiles {
        rig.lay(t, TOP, FaceAttachment::BlueprintBlank);
    }
    rig.set([floor[0], floor[1] + 1, floor[2]], block::OAK_PLANKS);
    let candidate = crate::plan::capture(rig.world(), (floor[0], floor[1], floor[2]), "survival").expect("a capture");
    let marker = crate::plan::marker(&candidate.data);
    let seq = rig.cs[0].actions.unanswered();
    let pkt = protocol::ItemActionPacket {
        seq,
        action: ItemAction::PlanMinted {
            source: crate::plan_mint::MintSource::CaptureCommit.to_wire(),
            x: floor[0],
            y: floor[1],
            z: floor[2],
            face: TOP as u8,
            hotbar_slot: 0,
            spent: None,
            plan: protocol::WireItem::Plan { marker, developed: false },
        },
        events_applied: rig.cs[0].events,
    };
    rig.cs[0].transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &pkt));
    rig.ticks(3);
    for t in tiles {
        assert_eq!(rig.holds(t, TOP), None, "{t:?}'s paper is spent on the server");
        for n in 0..2 {
            assert_eq!(rig.cs[n].sees(t, TOP), None, "and in joiner {n}'s copy");
        }
    }
    assert_ne!(rig.world().get_block(floor[0], floor[1], floor[2]), block::AIR, "the floor stays");
    assert!(
        crate::plan::capture(rig.world(), (floor[0], floor[1], floor[2]), "survival").is_err(),
        "a second capture over that spot finds no paper"
    );
}

// ─── Develop ───────────────────────────────────────────────────────────────

/// Decision 6 — a laid Blueprint develops on the SERVER's world (a
/// dedicated server, and a lending host where the server runs it inside the
/// lend window), and the flip reaches the joiner's copy, whose own develop
/// tick is off: its stub turns developed.
#[test]
fn a_develop_flip_on_the_server_reaches_a_joiner() {
    for lent in [false, true] {
        let tag = if lent { "develop-lent" } else { "develop-dedicated" };
        let mut rig = if lent { Rig::lent(tag, 1) } else { Rig::dedicated(tag, 1) };
        let floor = rig.floor_ahead();
        let mut plan = crate::plan::PlanData::debug_3x3_stone();
        plan.develop_state = crate::plan::DevelopState::Latent { exposure_ticks: crate::plan::DEVELOP_THRESHOLD_TICKS - 2 };
        rig.lay(floor, TOP, FaceAttachment::Blueprint(Box::new(plan)));
        rig.world().set_sky_light_at(floor[0], floor[1] + 1, floor[2], 15);
        match rig.host.as_mut() {
            Some(h) => h.clock.world_time = 11_000,
            None => rig.hs.server.world_time = 11_000,
        }
        assert_eq!(rig.cs[0].sees(floor, TOP).map(|a| crate::chunk_push::pushed_attachment(a)), Some(PushedAttachment::Blueprint { developed: false }));
        rig.ticks(4);
        assert!(
            matches!(rig.holds(floor, TOP), Some(FaceAttachment::Blueprint(p)) if p.develop_state.is_developed()),
            "{tag}: the server's Blueprint developed"
        );
        assert_eq!(
            rig.cs[0].sees(floor, TOP).map(|a| crate::chunk_push::pushed_attachment(a)),
            Some(PushedAttachment::Blueprint { developed: true }),
            "{tag}: the joiner's copy shows it"
        );
        assert_eq!(
            rig.cs[0].attachments.iter().filter(|a| a.att == Some(PushedAttachment::Blueprint { developed: true })).count(),
            1,
            "{tag}: the flip streamed once"
        );
    }
}

// ─── Single-player ─────────────────────────────────────────────────────────

/// Single-player keeps its arms: the shared rule paints and lays as before
/// (no world tracking, nothing logged), and a break recovers into the hand.
#[test]
fn single_player_paints_lays_and_recovers_as_before() {
    let registry = block::BlockRegistry::new();
    let mut world = crate::world::World::new();
    let wall = (3, 64, 5);
    world.set_block(wall.0, wall.1, wall.2, block::STONE);
    assert!(crate::blueprint_attach::attach(&mut world, &registry, wall, Face::North.index(), block::WALLPAPER_RED).is_ok());
    assert!(crate::blueprint_attach::lay_blank_blueprint_paper(&mut world, &registry, wall, [0, 1, 0]));
    assert!(world.take_attachment_changes().is_empty(), "single-player logs nothing");
    let mut inv = crate::inventory::Inventory::new();
    for item in crate::blueprint_attach::take_recoverable_attachments(&mut world, wall) {
        assert!(inv.add_item(ItemStack { item, count: 1 }).is_none());
    }
    let has = |item: &Item| inv.slots_iter().flatten().any(|s| &s.item == item);
    assert!(has(&paper()) && has(&wallpaper()), "both back in the hand");
}
