//! C3d run 13 — a Strict client's typed requests (a shot today).
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
