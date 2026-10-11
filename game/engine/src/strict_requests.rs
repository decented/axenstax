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
    let header = match remote.reserve_action(crate::action_wire::ActionIntent::Attack(packet), recipe, Box::new([])) {
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
