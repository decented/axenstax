//! Reserve joined window history before the actual shared click changes items.
use crate::{armour::ArmourItem, inventory::Inventory, item::Item, window::{ClickCtx, CraftGrid, WindowClick}};

#[derive(serde::Serialize, serde::Deserialize)]
pub struct WindowRecipe {
    pub click: WindowClick,
    pub sources: crate::action_wire::BoundedList<crate::protocol::WireSlot, 50>,
}

/// Bodies stay in live holdings; the journal retains only immutable markers.
/// All current bodies plus one fixed encoding scratch are bounded before store.
pub fn retain_private_window(inv: &Inventory, cursor: &Option<crate::item::ItemStack>, grid: &CraftGrid) -> Result<(), String> {
    let stacks = || inv.slots_iter().chain(std::iter::once(cursor.as_ref())).chain(grid.iter().flatten().map(Option::as_ref));
    let mut residency = crate::body_residency::BodyResidency::default();
    for stack in stacks().flatten() {
        if let Item::Plan(body) = &stack.item { residency.observe(body).map_err(|e| format!("Private Plan residency unavailable: {e:?}"))?; }
    }
    for stack in stacks().flatten() {
        if let Item::Plan(body) = &stack.item { crate::plan_store::remember_checked(body, residency.bytes())?; }
    }
    Ok(())
}

pub fn reserve_click(remote: &mut crate::remote_client::RemoteClient, inv: &Inventory, armour: &[Option<ArmourItem>; 4], cursor: &Option<crate::item::ItemStack>, grid: &CraftGrid, click: &WindowClick, ctx: &ClickCtx, registry: &crate::block::BlockRegistry) -> Result<crate::action_identity::ActionHeader, String> {
    retain_private_window(inv, cursor, grid)?;
    let raw = crate::staged_window::MarkerWindow::capture_parts(inv, armour, cursor, grid, registry);
    let mut staged = crate::staged_window::stage(&raw, click, ctx);
    let station = crate::window::station_after(ctx.station, click, &staged.result);
    let digest = crate::window::digest(&staged.after.view(), station);
    let intent = crate::action_wire::ActionIntent::Window(crate::protocol::WindowOpPacket {
        op_seq: 0, op: crate::protocol::WireWindowOp::Click(click.clone()), digest,
        events_applied: remote.applied_window_boundary(), touched: Vec::new(), claims: Vec::new(), client_ok: staged.result.ok(),
    });
    let recipe = WindowRecipe { click: click.clone(), sources: crate::action_wire::BoundedList::new(staged.sources.into_iter().collect())? };
    let bytes = bincode::serialize(&recipe).map_err(|e| e.to_string())?.into_boxed_slice();
    let cells = match ctx.station { crate::window::Station::Table { cell } => vec![cell].into_boxed_slice(), _ => Box::new([]) };
    remote.reserve_action(intent, bytes, cells)
}

/// C3d — reserve a Strict window op that isn't a personal-window click
/// (configuration, container entry, a shared container click) BEFORE the
/// screen or any holding changes. The op itself is the recipe; the
/// container's cell, if any, is the action's footprint.
pub fn reserve_op(remote: &mut crate::remote_client::RemoteClient, op: crate::protocol::WireWindowOp, digest: u32, client_ok: bool, cells: Box<[[i32; 3]]>) -> Result<crate::action_identity::ActionHeader, String> {
    let recipe = bincode::serialize(&op).map_err(|e| e.to_string())?.into_boxed_slice();
    let intent = crate::action_wire::ActionIntent::Window(crate::protocol::WindowOpPacket {
        op_seq: 0, op, digest, events_applied: remote.applied_window_boundary(), touched: Vec::new(), claims: Vec::new(), client_ok,
    });
    remote.reserve_action(intent, recipe, cells)
}
