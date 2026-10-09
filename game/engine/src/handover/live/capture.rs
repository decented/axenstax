//! The source's own frozen local seat 0, captured into its own records. ONE
//! function builds the sidecar so the obligation-carrier rebase can swap it
//! for `JoinerSave::from_parts` of that version (a local trusted seat has no
//! correction-debt machinery: its record is complete with nothing owed).
//! Capture writes nothing: an aborted handover leaves this machine as it was.
use super::eligibility::Refusal;
use crate::handover::personal::{LocalRestore, PersonalRecord, RecordContext};
use crate::player_slot::PlayerSlot;

/// The seat's window + vitals in exactly the sidecar encoding a joiner's
/// record uses. Pure: nothing on this machine changes at capture.
pub fn local0_sidecar(slot: &PlayerSlot, run_id: [u8; 16]) -> Result<Vec<u8>, Refusal> {
    let window = local0_window(slot);
    let save = crate::joiner_save::JoinerSave::from_parts(
        &window,
        slot.combat.health,
        slot.combat.hunger,
        slot.spawn_pos.to_array(),
        run_id,
        None,
    );
    save.encode().map_err(|_| Refusal("Your inventory couldn't be recorded for the handover."))
}

/// The seat's world-local progression and owner-only local restore.
pub fn local0_progress(slot: &PlayerSlot, context: RecordContext) -> Result<Vec<u8>, Refusal> {
    let locks = slot
        .inventory
        .locked_indices()
        .iter()
        .fold(0u64, |m, i| if *i < 36 { m | 1 << i } else { m });
    let local = LocalRestore {
        pos: slot.player.pos.to_array(),
        yaw: slot.camera.yaw,
        pitch: slot.camera.pitch,
        hotbar: slot.hotbar_slot.min(255) as u8,
        locks,
    };
    PersonalRecord::from_maps(
        context,
        slot.reputation.per_village.iter(),
        slot.kill_counter.iter(),
        slot.bounties_claimed.iter(),
        Some(local),
    )
    .and_then(|r| r.encode())
    .map_err(|_| Refusal("Your village or hunting progress is too large or malformed to move. Nothing was changed."))
}

/// A recipient whose progression is proven default for this session.
pub fn default_progress(context: RecordContext) -> Vec<u8> {
    PersonalRecord::from_maps(context, [], [], [], None)
        .and_then(|r| r.encode())
        .expect("an empty record always encodes")
}

fn local0_window(slot: &PlayerSlot) -> crate::window_events::EffectiveWindow {
    crate::window_events::EffectiveWindow {
        inv: slot.inventory.clone(),
        armour: slot.armour_slots,
        cursor: slot.crafting_ui.cursor_item.clone(),
        grid: slot.crafting_ui.grid.clone(),
    }
}

/// The seat's items whose private Plan bodies this machine must keep
/// resolvable once its record holds only their markers. Remembered in the
/// Plan store at retirement (a later boundary), never at capture.
pub fn local0_plan_items(slot: &PlayerSlot) -> Vec<crate::item::Item> {
    let w = local0_window(slot);
    let inv = (0..36).filter_map(|i| w.inv.slot(i).map(|s| s.item.clone()));
    let rest = w.cursor.iter().chain(w.grid.iter().flatten().flatten()).map(|s| s.item.clone());
    inv.chain(rest)
        .filter(|i| matches!(i, crate::item::Item::Plan(body) if body.marker.is_none()))
        .collect()
}
