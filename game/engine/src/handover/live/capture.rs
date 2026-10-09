//! The source's own frozen local seat 0, captured into its own records. ONE
//! function builds the sidecar so the obligation-carrier rebase can swap it
//! for `JoinerSave::from_parts` of that version (a local trusted seat has no
//! correction-debt machinery: its record is complete with nothing owed).
use super::eligibility::Refusal;
use crate::handover::personal::{LocalRestore, PersonalRecord, RecordContext};
use crate::player_slot::PlayerSlot;

/// The seat's window + vitals in exactly the sidecar encoding a joiner's
/// record uses. Private Plan bodies stay in this machine's own Plan store
/// (remembered here, before the record keeps only their markers).
pub fn local0_sidecar(slot: &PlayerSlot, run_id: [u8; 16]) -> Result<Vec<u8>, Refusal> {
    let window = crate::window_events::EffectiveWindow {
        inv: slot.inventory.clone(),
        armour: slot.armour_slots,
        cursor: slot.crafting_ui.cursor_item.clone(),
        grid: slot.crafting_ui.grid.clone(),
    };
    for i in 0..36 {
        if let Some(s) = window.inv.slot(i) {
            crate::plan_store::remember_item(&s.item);
        }
    }
    for s in window.cursor.iter().chain(window.grid.iter().flatten().flatten()) {
        crate::plan_store::remember_item(&s.item);
    }
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
