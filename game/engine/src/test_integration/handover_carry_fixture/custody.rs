//! Explicitly authored physical custody fixture, separate from natural quest activity.
//! Creative grants prove backed delivery of private markers resolving locally;
//! they do not claim earned items, Plan capture, or transfer of a private body.
use super::{HeadlessGame, Seat, settle};
use crate::{
    armour::{ArmourItem, ArmourMaterial, ArmourSlot},
    item::{Item, ItemStack},
    plan::PlanData,
    player_slot::PlayerSlot,
    window::WindowClick,
};
use nostr::ToBech32 as _;

#[derive(Debug, PartialEq, Eq)]
struct Allocation {
    pointer: usize,
    length: usize,
    capacity: usize,
}
impl Allocation {
    fn vector<T>(value: &Vec<T>) -> Self {
        Self {
            pointer: value.as_ptr() as usize,
            length: value.len(),
            capacity: value.capacity(),
        }
    }
    fn string(value: &String) -> Self {
        Self {
            pointer: value.as_ptr() as usize,
            length: value.len(),
            capacity: value.capacity(),
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
struct Body {
    place: usize,
    bytes: Vec<u8>,
    allocations: Vec<Allocation>,
}
#[derive(Debug, PartialEq, Eq)]
struct Physical {
    // Place addresses are additional witnesses, never substitutes for body pointers.
    places: [usize; 4],
    bytes: Vec<u8>,
    bodies: Vec<Body>,
}
impl Physical {
    fn capture(slot: &PlayerSlot) -> Self {
        let held: Vec<_> = slot.inventory.slots_iter().collect();
        let mut bodies = Vec::new();
        for (place, stack) in held
            .iter()
            .copied()
            .enumerate()
            .chain(std::iter::once((36, slot.crafting_ui.cursor_item.as_ref())))
            .chain(
                slot.crafting_ui
                    .grid
                    .iter()
                    .flatten()
                    .enumerate()
                    .map(|(i, s)| (37 + i, s.as_ref())),
            )
        {
            let Some(ItemStack {
                item: Item::Plan(plan),
                count,
            }) = stack
            else {
                continue;
            };
            assert_eq!(*count, 1);
            assert!(
                plan.marker.is_none() && !plan.cells.is_empty(),
                "a marker/stub is not a private body"
            );
            let mut allocations = vec![
                Allocation::vector(&plan.cells),
                Allocation::string(&plan.name),
                Allocation::string(&plan.author_npub),
                Allocation::string(&plan.authored_in),
                Allocation::vector(&plan.derivation_chain),
            ];
            for ancestor in &plan.derivation_chain {
                allocations.push(Allocation::string(&ancestor.author_npub));
                allocations.push(Allocation::string(&ancestor.plan_name));
            }
            assert!(
                allocations
                    .iter()
                    .all(|a| a.length > 0 && a.capacity >= a.length)
            );
            bodies.push(Body {
                place,
                bytes: bincode::serialize(plan).unwrap(),
                allocations,
            });
        }
        assert_eq!(
            bodies.len(),
            3,
            "authored bag, cursor and grid bodies must all be real"
        );
        assert!(bodies.iter().any(|body| (9..36).contains(&body.place)));
        assert!(bodies.iter().any(|body| body.place == 36));
        assert!(bodies.iter().any(|body| body.place == 41)); // grid [1][1]
        assert!(slot.armour_slots[ArmourSlot::Helmet as usize].is_some());
        Self {
            places: [
                &slot.inventory as *const _ as usize,
                &slot.armour_slots as *const _ as usize,
                &slot.crafting_ui.cursor_item as *const _ as usize,
                &slot.crafting_ui.grid as *const _ as usize,
            ],
            bytes: bincode::serialize(&(
                held,
                slot.armour_slots,
                &slot.crafting_ui.cursor_item,
                &slot.crafting_ui.grid,
                (0..36)
                    .map(|i| slot.inventory.is_locked(i))
                    .collect::<Vec<_>>(),
                slot.inventory.auto_refill,
            ))
            .unwrap(),
            bodies,
        }
    }
    fn assert_contents(&self, slot: &PlayerSlot) {
        let current = Self::capture(slot);
        assert_eq!(
            current.bytes, self.bytes,
            "physical contents or their places changed"
        );
        for (actual, expected) in current.bodies.iter().zip(&self.bodies) {
            assert_eq!(
                (actual.place, &actual.bytes),
                (expected.place, &expected.bytes)
            );
        }
    }
}

pub(in crate::test_integration) struct Custody {
    source: Physical,
    recipient: Physical,
}
impl Custody {
    /// Use while both live slots remain in place, including cut/abort/commit.
    pub(in crate::test_integration) fn assert_unchanged(
        &self,
        source: &HeadlessGame,
        peer: &HeadlessGame,
    ) {
        assert_eq!(
            Physical::capture(&source.state.players[0]),
            self.source,
            "source private allocation moved or changed"
        );
        assert_eq!(
            Physical::capture(&peer.state.players[0]),
            self.recipient,
            "recipient private allocation moved or changed"
        );
    }
    /// A checked native reopen may allocate again; exact contents/places remain required.
    pub(in crate::test_integration) fn assert_source_contents(&self, game: &HeadlessGame) {
        self.source.assert_contents(&game.state.players[0]);
    }
    pub(in crate::test_integration) fn assert_recipient_contents(&self, game: &HeadlessGame) {
        self.recipient.assert_contents(&game.state.players[0]);
    }
}
fn authored(owner: [u8; 32], label: &str) -> PlanData {
    let mut plan = PlanData::debug_3x3_stone();
    plan.name = format!("Authored custody {label}");
    plan.author_npub = nostr::PublicKey::from_slice(&owner)
        .unwrap()
        .to_bech32()
        .unwrap();
    plan.authored_in = "creative".into();
    let hash = crate::plan::content_hash(&plan);
    let link = &mut plan.derivation_chain[0];
    link.author_npub = plan.author_npub.clone();
    link.plan_name = plan.name.clone();
    link.plan_hash = hash;
    assert_eq!(plan.cells.len(), 9);
    crate::plan_store::remember_checked(
        &plan,
        crate::body_residency::allocated_body_bytes(&plan).unwrap(),
    )
    .unwrap();
    plan
}
fn find_plan(game: &HeadlessGame, expected: &PlanData) -> usize {
    game.state.players[0].inventory.slots_iter().enumerate().find_map(|(i, stack)| {
        stack.filter(|stack| matches!(&stack.item, Item::Plan(body) if body == expected && body.marker.is_none())).map(|_| i)
    }).expect("actual grant/add did not install the complete authored Plan")
}
fn click(game: &mut HeadlessGame, operation: WindowClick) {
    assert!(
        game.state.apply_window_click(0, &operation).ok(),
        "real window move refused: {operation:?}"
    );
}
fn pick(game: &mut HeadlessGame, slot: usize) {
    click(game, WindowClick::Slot { slot, right: false });
}
fn place(game: &mut HeadlessGame, plans: &[PlanData; 3]) {
    assert!(game.state.players[0].crafting_ui.cursor_item.is_none());
    assert!(
        game.state.players[0]
            .crafting_ui
            .grid
            .iter()
            .flatten()
            .all(Option::is_none)
    );
    game.state.open_crafting(0, None);
    let helmet = game.state.players[0]
        .inventory
        .slots_iter()
        .enumerate()
        .find_map(|(i, stack)| {
            stack
                .filter(|s| matches!(&s.item, Item::Armour(a) if a.slot == ArmourSlot::Helmet))
                .map(|_| i)
        })
        .expect("authored helmet was not physically installed");
    assert!(game.state.players[0].armour_slots[ArmourSlot::Helmet as usize].is_none());
    pick(game, helmet);
    click(
        game,
        WindowClick::Armour {
            slot: ArmourSlot::Helmet as usize,
        },
    );
    let from = find_plan(game, &plans[0]);
    let bag = (9..36)
        .find(|&i| game.state.players[0].inventory.slot(i).is_none())
        .unwrap();
    pick(game, from);
    pick(game, bag);
    let from = find_plan(game, &plans[1]);
    pick(game, from);
    click(
        game,
        WindowClick::Grid {
            row: 1,
            col: 1,
            right: false,
        },
    );
    let from = find_plan(game, &plans[2]);
    pick(game, from);
    let slot = &game.state.players[0];
    assert!(matches!(&slot.inventory.slot(bag).unwrap().item, Item::Plan(p) if p == &plans[0]));
    assert!(
        matches!(&slot.crafting_ui.grid[1][1].as_ref().unwrap().item, Item::Plan(p) if p == &plans[1])
    );
    assert!(
        matches!(&slot.crafting_ui.cursor_item.as_ref().unwrap().item, Item::Plan(p) if p == &plans[2])
    );
}
fn grant(source: &mut HeadlessGame, peer: &mut HeadlessGame, stack: ItemStack, seat: Seat) {
    let before = peer
        .state
        .remote_client
        .as_ref()
        .unwrap()
        .applied_window_event();
    let wire = crate::inventory::stack_to_wire(&stack);
    let state = &mut peer.state;
    let slot = &mut state.players[0];
    let header = crate::strict_requests::route_item(
        state.remote_client.as_mut(),
        &mut slot.crafting_ui,
        &slot.inventory,
        &slot.armour_slots,
        &mut state.joiner_actions,
        crate::joiner_actions::Pending {
            kind: crate::joiner_actions::Asked::CreativeGive,
            mob: None,
            hotbar_slot: 0,
            held: None,
        },
        crate::protocol::ItemActionPacket {
            seq: 0,
            action: crate::protocol::ItemAction::CreativeGive { item: wire },
            events_applied: 0,
        },
    )
    .unwrap()
    .expect("the actual joined grant must use a retained Strict action");
    assert!(header.seq > 0);
    assert_eq!(settle(source, peer), seat);
    assert!(
        peer.state
            .remote_client
            .as_ref()
            .unwrap()
            .applied_window_event()
            > before,
        "accepted backed grant did not physically reach its actual numbered frontier"
    );
}

/// Authored physical items only: quest activity, proof bindings and ACKs remain genuine.
/// The temporary fixture modes deliberately author Creative grants; they do not
/// claim an operator command or an earned/captured Plan. All original modes return
/// before the final real current ACK/readiness boundary is observed.
pub(in crate::test_integration) fn seed_authored(
    source: &mut HeadlessGame,
    peer: &mut HeadlessGame,
    seat: Seat,
) -> Custody {
    assert_eq!(settle(source, peer), seat);
    assert_eq!(
        peer.state.remote_client.as_ref().unwrap().inventory_mode,
        crate::action_identity::InventoryMode::Strict
    );
    let source_owner = source.state.players[0]
        .personal_binding
        .as_ref()
        .unwrap()
        .context()
        .participant()
        .unwrap();
    let source_plans = [
        authored(source_owner, "source bag"),
        authored(source_owner, "source grid"),
        authored(source_owner, "source cursor"),
    ];
    let peer_plans = [
        authored(seat.2, "recipient bag"),
        authored(seat.2, "recipient grid"),
        authored(seat.2, "recipient cursor"),
    ];
    let helmet = ItemStack {
        item: Item::Armour(ArmourItem::new(ArmourSlot::Helmet, ArmourMaterial::Iron)),
        count: 1,
    };
    for plan in &source_plans {
        assert!(
            source.state.players[0]
                .inventory
                .add_item(ItemStack {
                    item: Item::Plan(plan.clone()),
                    count: 1
                })
                .is_none()
        );
    }
    assert!(
        source.state.players[0]
            .inventory
            .add_item(helmet.clone())
            .is_none()
    );
    let source_mode = (source.state.play_mode, source.state.is_creative);
    let peer_mode = (peer.state.play_mode, peer.state.is_creative);
    let server_mode = source
        .state
        .hosted_server
        .as_ref()
        .unwrap()
        .server
        .play_mode;
    // Keep the native owner and its borrowed server consistent at each real tick.
    source.state.play_mode = crate::play_mode::PlayMode::Creative;
    source.state.is_creative = true;
    source
        .state
        .hosted_server
        .as_mut()
        .unwrap()
        .server
        .play_mode = crate::play_mode::PlayMode::Creative;
    peer.state.play_mode = crate::play_mode::PlayMode::Creative;
    peer.state.is_creative = true;
    for plan in &peer_plans {
        grant(
            source,
            peer,
            ItemStack {
                item: Item::Plan(plan.clone()),
                count: 1,
            },
            seat,
        );
        find_plan(peer, plan);
    }
    grant(source, peer, helmet, seat);
    place(source, &source_plans);
    place(peer, &peer_plans);
    assert_eq!(settle(source, peer), seat);
    source.state.play_mode = source_mode.0;
    source.state.is_creative = source_mode.1;
    source
        .state
        .hosted_server
        .as_mut()
        .unwrap()
        .server
        .play_mode = server_mode;
    peer.state.play_mode = peer_mode.0;
    peer.state.is_creative = peer_mode.1;
    assert_eq!(settle(source, peer), seat);
    let sp = &source.state.hosted_server.as_ref().unwrap().server.players[seat.0];
    let slot = &peer.state.players[0];
    assert_eq!(
        crate::window::digest_parts(
            &slot.inventory,
            &slot.armour_slots,
            &slot.crafting_ui.cursor_item,
            &slot.crafting_ui.grid,
            slot.crafting_ui.station()
        ),
        crate::window::digest_parts(
            &sp.inventory,
            &sp.armour,
            &sp.cursor,
            &sp.craft_grid,
            sp.station
        ),
        "actual recipient/backed window places disagree"
    );
    Custody {
        source: Physical::capture(&source.state.players[0]),
        recipient: Physical::capture(slot),
    }
}
