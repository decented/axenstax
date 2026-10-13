//! C3d run 13 — a Strict client's typed requests (a shot; run 15, an attack).
//!
//! On a Strict seat a shot is ONE typed `ActionIntent::Item` (`Shoot`),
//! reserved in the action journal BEFORE anything else changes, as a typed
//! edit or click is (`strict_edits.rs`, `window_prediction.rs`): its recipe
//! holds the weapon's hotbar slot and the digest of the window it was made
//! on, and its footprint is empty (a shot names no cell). Nothing is
//! predicted: no ammo is spent and the weapon doesn't wear until the
//! server's numbered Take and Wear arrive on its `ItemActionOutcome`, which
//! the client applies through the same `joiner_actions` path Mirror's
//! outcome takes (so the request is still recorded there, for its `seq` and
//! the in-flight ammo claim). Mirror keeps the legacy request byte for byte
//! (`ShotRoute::Legacy`: the caller sends it as before).
use crate::action_identity::{ActionHeader, InventoryMode};
use crate::armour::ArmourItem;
use crate::craft_ui::CraftingUi;
use crate::inventory::Inventory;
use crate::joiner_actions::{JoinerActions, Pending};
use crate::protocol;
use crate::remote_client::RemoteClient;

/// What a reserved typed shot retains in its journal entry.
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShotRecipe {
    /// The weapon's hotbar slot (the server reads its own copy's slot).
    pub slot: u8,
    pub weapon: protocol::ShotWeapon,
    /// The window it was made on; a shot predicts nothing, so it is also
    /// the window after it until the outcome arrives.
    pub digest: u32,
}

/// How a shot goes.
#[derive(Debug, PartialEq)]
pub enum ShotRoute {
    /// Reserved and sent as a typed action (a Strict seat).
    Typed(ActionHeader),
    /// The ammo is already claimed by shots in flight: nothing asked.
    Unaffordable,
    /// Not a Strict seat: the caller sends the legacy request, unchanged.
    Legacy,
}

/// Does this client send a shot typed? Only a Strict session.
pub fn shot_goes_typed(remote: Option<&RemoteClient>) -> bool {
    remote.is_some_and(|r| r.inventory_mode == InventoryMode::Strict)
}

/// Route player 0's shot `action` (an `ItemAction::Shoot`), recorded as
/// `pending` (what Mirror's `send_use_request` records): typed on a Strict
/// seat, else [`ShotRoute::Legacy`]. On a Strict seat the session's
/// auto-refill setting goes first (as before an edit), then the record and
/// the reservation, then the send; a failed reservation releases the
/// record and changes nothing else.
#[allow(clippy::too_many_arguments)]
pub fn route_shot(
    remote: Option<&mut RemoteClient>,
    ui: &mut CraftingUi,
    inv: &Inventory,
    armour: &[Option<ArmourItem>; 4],
    actions: &mut JoinerActions,
    pending: Pending,
    action: &protocol::ItemAction,
) -> Result<ShotRoute, String> {
    let Some(remote) = remote.filter(|r| shot_goes_typed(Some(&**r))) else { return Ok(ShotRoute::Legacy) };
    let protocol::ItemAction::Shoot { weapon, hotbar_slot, .. } = action else { return Err("Only a shot is typed here".into()) };
    if !actions.can_afford(inv, ui, pending.kind, pending.held.as_ref()) {
        return Ok(ShotRoute::Unaffordable);
    }
    ui.sync_auto_refill_joined(remote, inv, armour)?;
    let recipe = ShotRecipe { slot: *hotbar_slot, weapon: *weapon, digest: ui.digest(inv, armour) };
    let recipe = bincode::serialize(&recipe).map_err(|e| e.to_string())?.into_boxed_slice();
    // The claim ends with the next input's acknowledgement, as a request
    // sent at once does; the typed packet never waits in the request queue.
    let seq = actions.record(pending, remote.next_input_seq());
    let intent = crate::action_wire::ActionIntent::Item(protocol::ItemActionPacket {
        seq,
        action: action.clone(),
        events_applied: remote.applied_window_boundary(),
    });
    let header = match remote.reserve_action(intent, recipe, Box::new([])) {
        Ok(header) => header,
        Err(error) => {
            actions.release(seq);
            return Err(error);
        }
    };
    // Nothing is predicted, so a send failure (only a recovery prepared
    // after the reservation) leaves the retained reservation to recovery.
    if let Err(error) = remote.send_reserved_action(header) {
        log::warn!("Reserved shot {} could not be sent: {error}", header.seq);
    }
    Ok(ShotRoute::Typed(header))
}

/// What a reserved typed attack retains in its journal entry.
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AttackRecipe {
    /// The hotbar slot the swing was made from (the server reads its own
    /// copy's item there, never the claim).
    pub slot: u8,
    /// The window it was made on; an attack predicts nothing, so it is also
    /// the window after it until the outcome arrives.
    pub digest: u32,
}

/// C3d run 15 — route player 0's swing `packet` (its `events_applied` is
/// stamped here), recorded as `pending` (what Mirror's
/// `GameState::send_entity_attack` records): on a Strict seat, the
/// session's refill setting, the record, ONE reserved
/// `ActionIntent::Attack`, then the send, and `Some(header)`; nothing is
/// predicted (the weapon wears when the server's `InteractOutcome` names
/// its numbered Wear). `None` off a Strict seat: the caller sends the
/// legacy `EntityAttack` request, unchanged. A failed reservation releases
/// the record and changes nothing else.
#[allow(clippy::too_many_arguments)]
pub fn route_attack(
    remote: Option<&mut RemoteClient>,
    ui: &mut CraftingUi,
    inv: &Inventory,
    armour: &[Option<ArmourItem>; 4],
    actions: &mut JoinerActions,
    pending: Pending,
    mut packet: protocol::EntityAttackPacket,
) -> Result<Option<ActionHeader>, String> {
    let Some(remote) = remote.filter(|r| shot_goes_typed(Some(&**r))) else { return Ok(None) };
    ui.sync_auto_refill_joined(remote, inv, armour)?;
    let recipe = AttackRecipe { slot: packet.hotbar_slot, digest: ui.digest(inv, armour) };
    let recipe = bincode::serialize(&recipe).map_err(|e| e.to_string())?.into_boxed_slice();
    packet.seq = actions.record(pending, remote.next_input_seq());
    packet.events_applied = remote.applied_window_boundary();
    let seq = packet.seq;
    // C3d run 16 — nothing is predicted for a swing, and the server relies on
    // it: a world-fact refusal is a receipt alone (`refuse_world_fact`). Run
    // 17: the game loop's `NoPrediction` guard checks it on the window.
    let intent = crate::action_wire::ActionIntent::Attack(packet);
    let header = match remote.reserve_action(intent, recipe, Box::new([])) {
        Ok(header) => header,
        Err(error) => {
            actions.release(seq);
            return Err(error);
        }
    };
    if let Err(error) = remote.send_reserved_action(header) {
        log::warn!("Reserved attack {} could not be sent: {error}", header.seq);
    }
    Ok(Some(header))
}

/// What a reserved typed interaction retains in its journal entry (C3d run
/// 23): the hotbar slot it was made from (the server reads its own copy's
/// item there, never the claim) and the window it was made on (nothing is
/// predicted, so also the window after it until the outcome arrives).
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InteractRecipe {
    pub slot: u8,
    pub digest: u32,
}

/// C3d run 23 — route player 0's entity interaction `packet` (feed, tame,
/// shear, milk, a Lead on, off or on a post, a pet command, a mount; its
/// `events_applied` is stamped here), recorded as `pending` (what Mirror's
/// `GameState::send_interaction` records): on a Strict seat, the session's
/// refill setting, the record, ONE reserved `ActionIntent::Interact`, then
/// the send, and `Some(header)`. Nothing is predicted: the held item is
/// taken, and a product added, only by the server's accepted outcome and
/// numbered grants (`joiner_actions::apply_outcome`). `None` off a Strict
/// seat: the caller sends the legacy `EntityInteract`, unchanged. A failed
/// reservation releases the record and changes nothing else.
#[allow(clippy::too_many_arguments)]
pub fn route_interact(
    remote: Option<&mut RemoteClient>,
    ui: &mut CraftingUi,
    inv: &Inventory,
    armour: &[Option<ArmourItem>; 4],
    actions: &mut JoinerActions,
    pending: Pending,
    mut packet: protocol::EntityInteractPacket,
) -> Result<Option<ActionHeader>, String> {
    let Some(remote) = remote.filter(|r| shot_goes_typed(Some(&**r))) else { return Ok(None) };
    ui.sync_auto_refill_joined(remote, inv, armour)?;
    let recipe = InteractRecipe { slot: packet.hotbar_slot, digest: ui.digest(inv, armour) };
    let recipe = bincode::serialize(&recipe).map_err(|e| e.to_string())?.into_boxed_slice();
    packet.seq = actions.record(pending, remote.next_input_seq());
    packet.events_applied = remote.applied_window_boundary();
    let seq = packet.seq;
    // The server relies on nothing predicted: a world-fact refusal (the mob
    // gone, the cooldown, milk not ready, `NotOnThisServer`) is a receipt
    // and its note alone (`refuse_world_fact`); the game loop's
    // `NoPrediction` bracket checks it on the window.
    let intent = crate::action_wire::ActionIntent::Interact(packet);
    let header = match remote.reserve_action(intent, recipe, Box::new([])) {
        Ok(header) => header,
        Err(error) => {
            actions.release(seq);
            return Err(error);
        }
    };
    if let Err(error) = remote.send_reserved_action(header) {
        log::warn!("Reserved interaction {} could not be sent: {error}", header.seq);
    }
    Ok(Some(header))
}

/// C3d run 24 — what a reserved typed eat, creative give or device
/// right-click retains: the hotbar slot it names and the window it was made
/// on (nothing is predicted, so also the window after it).
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ItemRecipe {
    pub slot: u8,
    pub digest: u32,
}

/// C3d run 24 — route player 0's eat or creative give `packet` (an
/// `ItemAction::Eat` or `ItemAction::CreativeGive`; its `events_applied` is
/// stamped here), recorded as `pending`, as [`route_interact`] routes an
/// interaction: on a Strict seat the refill setting, the record, ONE
/// reserved `ActionIntent::Item`, the send, and `Some(header)`. Nothing is
/// predicted: the food is taken only by the server's accepted outcome, a
/// give lands only by its numbered `InventoryGrant`. `None` off a Strict
/// seat: the caller sends the legacy `ItemAction`, unchanged.
#[allow(clippy::too_many_arguments)]
pub fn route_item(
    remote: Option<&mut RemoteClient>,
    ui: &mut CraftingUi,
    inv: &Inventory,
    armour: &[Option<ArmourItem>; 4],
    actions: &mut JoinerActions,
    pending: Pending,
    mut packet: protocol::ItemActionPacket,
) -> Result<Option<ActionHeader>, String> {
    let Some(remote) = remote.filter(|r| shot_goes_typed(Some(&**r))) else { return Ok(None) };
    debug_assert!(matches!(packet.action, protocol::ItemAction::Eat { .. } | protocol::ItemAction::CreativeGive { .. }), "route_item carries an eat or a give");
    ui.sync_auto_refill_joined(remote, inv, armour)?;
    let slot = u8::try_from(pending.hotbar_slot).unwrap_or(u8::MAX);
    let recipe = ItemRecipe { slot, digest: ui.digest(inv, armour) };
    let recipe = bincode::serialize(&recipe).map_err(|e| e.to_string())?.into_boxed_slice();
    packet.seq = actions.record(pending, remote.next_input_seq());
    packet.events_applied = remote.applied_window_boundary();
    let seq = packet.seq;
    let intent = crate::action_wire::ActionIntent::Item(packet);
    debug_assert!(intent.predicts_nothing());
    let header = match remote.reserve_action(intent, recipe, Box::new([])) {
        Ok(header) => header,
        Err(error) => {
            actions.release(seq);
            return Err(error);
        }
    };
    if let Err(error) = remote.send_reserved_action(header) {
        log::warn!("Reserved item action {} could not be sent: {error}", header.seq);
    }
    Ok(Some(header))
}

/// C3d run 24 — route player 0's right-click on the power device at `cell`
/// made with hotbar `hotbar_slot` in hand (the server reaches with ITS
/// copy's item there): on a Strict seat the refill setting, ONE reserved
/// `ActionIntent::Device` (no request record: a device right-click has no
/// outcome), the send, and `Some(header)`. Nothing is predicted: the device
/// flips only by the server's broadcast. `None` off a Strict seat: the
/// caller sends the legacy `DeviceInteract`, unchanged.
pub fn route_device(
    remote: Option<&mut RemoteClient>,
    ui: &mut CraftingUi,
    inv: &Inventory,
    armour: &[Option<ArmourItem>; 4],
    cell: [i32; 3],
    hotbar_slot: u8,
) -> Result<Option<ActionHeader>, String> {
    let Some(remote) = remote.filter(|r| shot_goes_typed(Some(&**r))) else { return Ok(None) };
    ui.sync_auto_refill_joined(remote, inv, armour)?;
    let recipe = ItemRecipe { slot: hotbar_slot, digest: ui.digest(inv, armour) };
    let recipe = bincode::serialize(&recipe).map_err(|e| e.to_string())?.into_boxed_slice();
    let intent = crate::action_wire::ActionIntent::Device { cell, hotbar_slot };
    debug_assert!(intent.predicts_nothing());
    let header = remote.reserve_action(intent, recipe, Box::new([]))?;
    if let Err(error) = remote.send_reserved_action(header) {
        log::warn!("Reserved device right-click {} could not be sent: {error}", header.seq);
    }
    Ok(Some(header))
}

/// C3d run 17 (run-16 review LOW-1) — the observed half of
/// `ActionIntent::predicts_nothing`: a Strict swing or shot changes nothing
/// in the joined window before its answer, and the server relies on it (a
/// world-fact refusal is a receipt alone, no correction). The game loop
/// takes the window's digest before the swing or shot branch
/// ([`Self::begin`]) and checks it after the branch ([`Self::end`]), so a
/// prediction added anywhere in that branch (a weapon worn, ammo taken)
/// is an error, never a silent divergence. No window event is applied
/// inside the branch, so only the action itself can change the digest.
pub struct NoPrediction {
    digest: u32,
    what: &'static str,
}

impl NoPrediction {
    /// On a Strict seat, the window's digest before `what`; else `None`.
    pub fn begin(remote: Option<&RemoteClient>, ui: &CraftingUi, inv: &Inventory, armour: &[Option<ArmourItem>; 4], what: &'static str) -> Option<Self> {
        // C3d run 18 (DECIDED after the run-17 review) — the digests run in
        // debug builds only; a release build computes nothing here.
        (cfg!(debug_assertions) && shot_goes_typed(remote)).then(|| NoPrediction { digest: ui.digest(inv, armour), what })
    }

    /// `Err` when the window changed since [`Self::begin`].
    pub fn end(self, ui: &CraftingUi, inv: &Inventory, armour: &[Option<ArmourItem>; 4]) -> Result<(), String> {
        let now = ui.digest(inv, armour);
        if now == self.digest {
            Ok(())
        } else {
            Err(format!("a Strict {} changed the joined window before its answer ({:08x} -> {now:08x}); a world-fact refusal would leave it uncorrected", self.what, self.digest))
        }
    }

    /// [`Self::end`] where the game loop runs it: an error is logged, and a
    /// debug build stops.
    pub fn check(guard: Option<Self>, ui: &CraftingUi, inv: &Inventory, armour: &[Option<ArmourItem>; 4]) {
        #[cfg(test)]
        if let Some(g) = &guard { checked::note(g.what); }
        if let Some(guard) = guard && let Err(error) = guard.end(ui, inv, armour) {
            log::error!("{error}");
            debug_assert!(false, "{error}");
        }
    }
}

/// C3d run 18 (run-17 review LOW-1) — the `NoPrediction` brackets that
/// actually compared on this thread (each test runs on its own), so a test
/// driving the real game-loop branch can show its bracket ran.
#[cfg(test)]
pub mod checked {
    use std::cell::RefCell;
    thread_local!(static SEEN: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) });
    pub(super) fn note(what: &'static str) { SEEN.with(|s| s.borrow_mut().push(what)); }
    /// The brackets checked on this thread so far.
    pub fn seen() -> Vec<&'static str> { SEEN.with(|s| s.borrow().clone()) }
}

#[cfg(test)]
mod bracket_position_tests {
    /// C3d run 18 (run-17 review LOW-1) — a SOURCE-POSITION guard (the GPU
    /// swing test calls `send_entity_attack` directly, so no test drives the
    /// game loop's joined swing branch): the swing bracket opens before
    /// `send_entity_attack` and closes after the local-hit wear block; the
    /// shot bracket encloses `send_shot`. Moving or removing either fails.
    /// Run 21 (run-18 review L1): removing a bracket's close no longer lets
    /// the search run on to the next bracket's. Run 23: the interaction
    /// bracket encloses both the typed route and Mirror's legacy request.
    #[test]
    fn c3d_no_prediction_brackets_enclose_the_swing_the_shot_the_interaction_the_eat_the_give_and_the_device() {
        let src = include_str!("game_loop.rs");
        for (what, inside) in [("\"swing\")", &["self.send_entity_attack(pidx", "inventory.use_hotbar_tool(hotbar)"][..]), ("\"shot\")", &["self.send_shot(pidx, weapon)"][..]), ("\"interaction\")", &["strict_requests::route_interact(", "Request::Interact("][..]), ("\"eat\")", &["self.route_item_request(pidx, pending, action)"][..]), ("\"creative give\")", &["self.route_item_request(pidx, pending, action)"][..]), ("\"device\")", &["strict_requests::route_device("][..])] {
            let begin = src.find(&format!("NoPrediction::begin(self.remote_client.as_ref(), &p.crafting_ui, &p.inventory, &p.armour_slots, {what}")).unwrap_or_else(|| panic!("the {what} bracket opens in the game loop"));
            let end = begin + src[begin..].find("NoPrediction::check(").unwrap_or_else(|| panic!("the {what} bracket closes"));
            // C3d run 21 (run-18 review L1) — the close found is THIS
            // bracket's: it comes before the next bracket opens, and it
            // checks this bracket's own guard.
            let after = begin + 1;
            if let Some(next) = src[after..].find("NoPrediction::begin(").map(|n| after + n) {
                assert!(end < next, "the {what} bracket closes before the next bracket opens");
            }
            assert!(src[end..].starts_with("NoPrediction::check(no_prediction,"), "the {what} bracket checks its own guard");
            for call in inside {
                let at = src[begin..end].find(call);
                assert!(at.is_some(), "the {what} bracket encloses `{call}`");
            }
        }
    }
}
