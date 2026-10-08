//! Inventory — 36 slots (9 hotbar + 27 main) with unified item stacks.
//!
//! Spec 05 Section 4: Hotbar always visible, main inventory opens with E.
//! Items can be blocks, tools, or other types via the unified Item enum.

use crate::block::BlockId;
use crate::item::ItemStack;

/// Map a single inventory [`Item`](crate::item::Item) to a wire
/// [`ItemRef`](crate::protocol::ItemRef) for the per-tick player-state
/// broadcast and the client→server `InputPacket` held-item field. Tools
/// collapse to their material tier (0 wood, 1 stone, 2 iron, 3 diamond,
/// 4 satori); materials carry their bincode discriminant index as the id.
/// Plans/Armour have no avatar-held wire form yet, so they resolve to `Empty`.
///
/// Shared by the client (`game_loop.rs`, populating its own InputPacket) and
/// the server (`server.rs`, resolving local players' held item) so both ends
/// agree on the encoding.
pub fn item_to_ref(item: &crate::item::Item) -> crate::protocol::ItemRef {
    use crate::crafting::ToolMaterial;
    use crate::item::Item;
    use crate::protocol::ItemRef;
    match item {
        Item::Block(id) => ItemRef::Block(*id),
        Item::Tool(t) => {
            let tier: u16 = match t.material {
                ToolMaterial::Wood => 0,
                ToolMaterial::Stone => 1,
                ToolMaterial::Iron => 2,
                ToolMaterial::Diamond => 3,
                ToolMaterial::Satori => 4,
            };
            ItemRef::Tool(tier)
        }
        Item::Material(id) => ItemRef::Material(*id as u16),
        // No held-avatar wire form for these yet — render as empty-handed.
        Item::Plan(_) | Item::Armour(_) => ItemRef::Empty,
    }
}

/// Decode a wire `(item_kind, item_id)` pair back to an inventory
/// [`Item`](crate::item::Item) — the inverse of [`item_to_ref`] for the kinds
/// that survive the encoding (death-drops phase 2b, `InventoryGrantPacket`).
///
/// Returns `None` for anything that can't be reconstructed faithfully:
/// unknown/hostile ids from the network (the registry's silent AIR fallback
/// must never mint an item), `TOOL` refs (the wire collapses tools to a bare
/// material tier — kind and durability are lost), and `EMPTY`/unknown
/// discriminators. The server's pickup pass applies the same eligibility rule
/// on the encode side, so a `None` here means a tampered or newer-version peer.
pub fn item_from_ref(
    kind: u8,
    id: u16,
    registry: &crate::block::BlockRegistry,
) -> Option<crate::item::Item> {
    use crate::item::Item;
    use crate::protocol::item_kind;
    match kind {
        item_kind::BLOCK if registry.is_known(id) => Some(Item::Block(id)),
        item_kind::MATERIAL => crate::item::MaterialId::try_from(id).ok().map(Item::Material),
        _ => None,
    }
}

// ─── Full-fidelity item wire (death-drops phase 3, protocol v61) ───
//
// The `(item_kind, item_id)` pair above is lossy: a tool collapses to its
// material tier and an armour piece to `Empty`, so the server used to refuse
// to grant either (minting a wrong-kind, full-durability item would be worse
// than leaving it on the floor). `protocol::WireItem` carries the missing
// per-instance state; these two functions are the ONLY place the u8 wire
// bytes and the gameplay enums meet, and both directions are an explicit
// `match` — never an `as`-cast, so appending a `ToolType` or `ArmourMaterial`
// variant can't silently shift the wire meaning of an existing byte.
//
// Spec: `docs/foundations/2026-07-12-full-fidelity-item-wire.md`.

/// Wire byte for a [`ToolType`](crate::crafting::ToolType). Append-only.
fn tool_type_to_wire(t: crate::crafting::ToolType) -> u8 {
    use crate::crafting::ToolType as T;
    match t {
        T::Pickaxe => 0,
        T::Axe => 1,
        T::Sword => 2,
        T::Shovel => 3,
        T::Bow => 4,
        T::Hoe => 5,
        T::FlintAndSteel => 6,
        T::Shears => 7,
        T::FishingRod => 8,
        T::Slingshot => 9,
        T::Eraser => 10,
        T::DraftingStamp => 11,
    }
}

/// Inverse of [`tool_type_to_wire`]. Unrecognised bytes (a newer peer) yield
/// `None`, which refuses the whole item rather than guessing.
fn tool_type_from_wire(b: u8) -> Option<crate::crafting::ToolType> {
    use crate::crafting::ToolType as T;
    Some(match b {
        0 => T::Pickaxe,
        1 => T::Axe,
        2 => T::Sword,
        3 => T::Shovel,
        4 => T::Bow,
        5 => T::Hoe,
        6 => T::FlintAndSteel,
        7 => T::Shears,
        8 => T::FishingRod,
        9 => T::Slingshot,
        10 => T::Eraser,
        11 => T::DraftingStamp,
        _ => return None,
    })
}

/// Wire byte for a [`ToolMaterial`](crate::crafting::ToolMaterial). Matches
/// the tier ordering `item_to_ref` already puts on the wire, deliberately —
/// one less number for a reader to hold.
fn tool_material_to_wire(m: crate::crafting::ToolMaterial) -> u8 {
    use crate::crafting::ToolMaterial as M;
    match m {
        M::Wood => 0,
        M::Stone => 1,
        M::Iron => 2,
        M::Diamond => 3,
        M::Satori => 4,
    }
}

/// Inverse of [`tool_material_to_wire`].
fn tool_material_from_wire(b: u8) -> Option<crate::crafting::ToolMaterial> {
    use crate::crafting::ToolMaterial as M;
    Some(match b {
        0 => M::Wood,
        1 => M::Stone,
        2 => M::Iron,
        3 => M::Diamond,
        4 => M::Satori,
        _ => return None,
    })
}

/// Wire byte for an [`ArmourSlot`](crate::armour::ArmourSlot). Append-only.
fn armour_slot_to_wire(s: crate::armour::ArmourSlot) -> u8 {
    use crate::armour::ArmourSlot as S;
    match s {
        S::Helmet => 0,
        S::Chestplate => 1,
        S::Leggings => 2,
        S::Boots => 3,
    }
}

/// Inverse of [`armour_slot_to_wire`].
fn armour_slot_from_wire(b: u8) -> Option<crate::armour::ArmourSlot> {
    use crate::armour::ArmourSlot as S;
    Some(match b {
        0 => S::Helmet,
        1 => S::Chestplate,
        2 => S::Leggings,
        3 => S::Boots,
        _ => return None,
    })
}

/// Wire byte for an [`ArmourMaterial`](crate::armour::ArmourMaterial).
/// Append-only.
fn armour_material_to_wire(m: crate::armour::ArmourMaterial) -> u8 {
    use crate::armour::ArmourMaterial as M;
    match m {
        M::Leather => 0,
        M::Iron => 1,
        M::Diamond => 2,
        M::Satori => 3,
        M::Chainmail => 4,
        M::Rubber => 5,
    }
}

/// Inverse of [`armour_material_to_wire`].
fn armour_material_from_wire(b: u8) -> Option<crate::armour::ArmourMaterial> {
    use crate::armour::ArmourMaterial as M;
    Some(match b {
        0 => M::Leather,
        1 => M::Iron,
        2 => M::Diamond,
        3 => M::Satori,
        4 => M::Chainmail,
        5 => M::Rubber,
        _ => return None,
    })
}

/// Encode an [`Item`](crate::item::Item) as a
/// [`WireItem`](crate::protocol::WireItem) — the full-fidelity companion to
/// [`item_to_ref`]'s lossy `(kind, id)` pair.
///
/// Blocks and materials return `WireItem::None`: the pair already carries
/// them losslessly, so there is nothing to add. `Item::Plan` also returns
/// `None` — plans stay floor-bound by design (`plan::PlanData` is far too
/// heavy for a per-tick broadcast); the append-only enum leaves room to add
/// them later without renumbering anything.
pub fn item_to_wire_full(item: &crate::item::Item) -> crate::protocol::WireItem {
    use crate::item::Item;
    use crate::protocol::WireItem;
    match item {
        Item::Tool(t) => WireItem::Tool {
            tool_type: tool_type_to_wire(t.tool_type),
            material: tool_material_to_wire(t.material),
            durability: t.durability,
        },
        Item::Armour(a) => WireItem::Armour {
            slot: armour_slot_to_wire(a.slot),
            material: armour_material_to_wire(a.material),
            durability: a.durability,
        },
        Item::Block(_) | Item::Material(_) | Item::Plan(_) => WireItem::None,
    }
}

/// Decode a [`WireItem`](crate::protocol::WireItem) back to an
/// [`Item`](crate::item::Item) — the inverse of [`item_to_wire_full`].
///
/// Returns `None` for `WireItem::None` (the caller falls back to the legacy
/// `(kind, id)` pair via [`item_from_ref`]) and for anything that fails
/// validation:
///
/// - an unrecognised tool type / material / armour slot / armour tier byte —
///   a newer or tampered peer; never guess at a substitute,
/// - a zero durability — that item is *broken*; granting it would hand the
///   player a dead tool that the local sim would never have produced.
///
/// A durability ABOVE the kind's maximum is clamped rather than refused: it
/// is the one malformed value with an unambiguous sane reading, and clamping
/// keeps a future durability-ladder retune from silently voiding drops in
/// flight. Grants are server → client only, so all of this is
/// defence-in-depth, not a trust boundary.
pub fn item_from_wire_full(w: &crate::protocol::WireItem) -> Option<crate::item::Item> {
    use crate::item::Item;
    use crate::protocol::WireItem;
    match *w {
        WireItem::None => None,
        WireItem::Tool { tool_type, material, durability } => {
            if durability == 0 {
                return None;
            }
            let mut tool = crate::crafting::Tool::new(
                tool_type_from_wire(tool_type)?,
                tool_material_from_wire(material)?,
            );
            tool.durability = durability.min(tool.max_durability());
            Some(Item::Tool(tool))
        }
        WireItem::Armour { slot, material, durability } => {
            if durability == 0 {
                return None;
            }
            let slot = armour_slot_from_wire(slot)?;
            let material = armour_material_from_wire(material)?;
            let mut piece = crate::armour::ArmourItem::new(slot, material);
            piece.durability = durability.min(crate::armour::max_durability(slot, material));
            Some(Item::Armour(piece))
        }
    }
}

/// C3b-1 — a stack on the wire at full fidelity
/// ([`crate::protocol::WireStack`]): the `(kind, id)` pair, the count, and
/// the tool/armour state ([`item_to_wire_full`]). A Plan, which has no wire
/// form, goes as the reserved `item_kind::PLAN` (id 0): a placeholder.
pub fn stack_to_wire(stack: &ItemStack) -> crate::protocol::WireStack {
    let (item_kind, item_id) = match &stack.item {
        crate::item::Item::Plan(_) => (crate::protocol::item_kind::PLAN, 0),
        item => item_to_ref(item).to_wire(),
    };
    crate::protocol::WireStack { item_kind, item_id, count: stack.count, full_item: item_to_wire_full(&stack.item) }
}

/// C3b-1 — the inverse of [`stack_to_wire`]: the full-fidelity payload wins,
/// then the pair ([`item_from_ref`]). A Plan placeholder decodes to
/// `plan::PlanData::placeholder` only where `allow_plan` (a container slot);
/// a zero count, or anything that doesn't decode faithfully, is `None`.
pub fn stack_from_wire(
    w: &crate::protocol::WireStack,
    registry: &crate::block::BlockRegistry,
    allow_plan: bool,
) -> Option<ItemStack> {
    if w.count == 0 {
        return None;
    }
    let item = match item_from_wire_full(&w.full_item) {
        Some(item) => item,
        None if w.item_kind == crate::protocol::item_kind::PLAN => {
            if !allow_plan {
                return None;
            }
            crate::item::Item::Plan(crate::plan::PlanData::placeholder())
        }
        None => item_from_ref(w.item_kind, w.item_id, registry)?,
    };
    Some(ItemStack { item, count: w.count })
}

/// Returned by [`Inventory::use_hotbar_tool`]. Carries the pre/post durability
/// percentage so callers can drive low-durability warnings + just-broke
/// toasts without re-reading the slot.
#[derive(Clone, Debug)]
pub struct ToolUseInfo {
    pub before_pct: f32,
    pub after_pct: f32,
    pub just_broke: bool,
    pub display_name: String,
}

/// `Clone` so callers can dry-run an all-or-nothing placement (craft
/// result, vendor withdraw) before committing.
#[derive(Clone)]
pub struct Inventory {
    /// Slots 0-8 = hotbar, 9-35 = main inventory.
    slots: [Option<ItemStack>; 36],
    /// Set by `toggle()` but never read — the live inventory-open state lives
    /// elsewhere (the UI panel's own visibility flag), not on `Inventory` itself.
    #[allow(dead_code)]
    pub open: bool,
    /// #45 — per-slot lock. A locked slot keeps its item + index through a sort
    /// and is skipped by quick-stack/auto-refill. Session-scoped for now (not
    /// serialised — persisting it pairs with the next save-format pass to avoid
    /// touching `save.rs` while Phase 4's save churn is in flight).
    locked: [bool; 36],
    /// #45 P4 — when a hotbar **stack** is exhausted by placing, pull the next
    /// matching stack from the bag into the emptied slot. Default on; synced from
    /// `GraphicsSettings.auto_refill` on world entry. Tools never stack, so this
    /// only restocks stackable items (blocks/materials).
    pub auto_refill: bool,
}

impl Inventory {
    pub fn new() -> Self {
        // Survival = empty inventory. Creative players get a starter set
        // populated in chunk_stream::initial_load when the world is created
        // or loaded.
        Self {
            slots: std::array::from_fn(|_| None),
            open: false,
            locked: [false; 36],
            auto_refill: true,
        }
    }

    /// #45 P4 — restock a just-emptied hotbar slot (0..9) with the next matching
    /// stack from the bag (main 9..36 first, then other hotbar slots), skipping
    /// locked sources. No-op when auto-refill is off, the slot isn't a hotbar
    /// slot, or it's already occupied. `item` is what was just used up.
    fn auto_refill_hotbar(&mut self, slot: usize, item: &crate::item::Item) {
        if !self.auto_refill || slot >= 9 || self.slots[slot].is_some() {
            return;
        }
        for i in (9..36).chain(0..9) {
            if i == slot || self.locked[i] {
                continue;
            }
            if self.slots[i].as_ref().is_some_and(|s| s.item.can_stack_with(item)) {
                self.slots[slot] = self.slots[i].take();
                return;
            }
        }
    }

    /// #45 — is slot `i` locked (excluded from sort/quick-stack/auto-refill)?
    pub fn is_locked(&self, i: usize) -> bool {
        i < 36 && self.locked[i]
    }

    /// #45 — toggle the lock on slot `i` (right-click in the inventory UI).
    pub fn toggle_lock(&mut self, i: usize) {
        if i < 36 {
            self.locked[i] = !self.locked[i];
        }
    }

    /// #45 P3 — indices of all locked slots, for persistence
    /// (`save::WorldSave.locked_slots`).
    pub fn locked_indices(&self) -> Vec<u32> {
        (0..36).filter(|&i| self.locked[i]).map(|i| i as u32).collect()
    }

    /// #45 P3 — restore the lock set from a persisted index list (clears
    /// existing locks first). Out-of-range indices are ignored.
    pub fn set_locked_from(&mut self, indices: &[u32]) {
        self.locked = [false; 36];
        for &i in indices {
            if (i as usize) < 36 {
                self.locked[i as usize] = true;
            }
        }
    }

    /// #45 — sort the half-open slot range `[start, end)` in place: merge partial
    /// stacks of the same item and order by `Item::sort_key`, keeping locked
    /// slots fixed. Used by the inventory Sort button (main region 9..36) and the
    /// container Sort button (its own range).
    pub fn sort_region(&mut self, start: usize, end: usize) {
        let end = end.min(36);
        if start >= end {
            return;
        }
        let region: Vec<Option<ItemStack>> = self.slots[start..end].to_vec();
        let locked: Vec<bool> = self.locked[start..end].to_vec();
        let sorted = sort_slots(&region, &locked);
        for (offset, stack) in sorted.into_iter().enumerate() {
            self.slots[start + offset] = stack;
        }
    }

    pub fn hotbar_slot(&self, index: usize) -> Option<&ItemStack> {
        if index < 9 { self.slots[index].as_ref() } else { None }
    }

    pub fn hotbar_slot_mut(&mut self, index: usize) -> Option<&mut ItemStack> {
        if index < 9 { self.slots[index].as_mut() } else { None }
    }

    pub fn slot(&self, index: usize) -> Option<&ItemStack> {
        if index < 36 { self.slots[index].as_ref() } else { None }
    }

    pub fn set_slot(&mut self, index: usize, stack: Option<ItemStack>) {
        if index < 36 {
            self.slots[index] = stack;
        }
    }

    /// Take the entire stack out of a slot, leaving it empty. Returns
    /// the previous occupant. Used by the HP-2 chest dialog to lift
    /// stacks across the inventory/chest divide.
    pub fn take_slot(&mut self, index: usize) -> Option<ItemStack> {
        if index < 36 { self.slots[index].take() } else { None }
    }

    /// Empty every slot (0..36). Used by the scenario runner to wipe the
    /// inventory before provisioning a scenario kit (a scenario starts the
    /// player from the def's loadout, not their survival inventory).
    pub fn clear(&mut self) {
        for slot in self.slots.iter_mut() {
            *slot = None;
        }
    }

    /// Iterate over every slot 0..36 in order. `None` for empty slots.
    /// Used by Spec 24 capture/inspect/place flows that scan for plans
    /// or aggregate material counts.
    pub fn slots_iter(&self) -> impl Iterator<Item = Option<&ItemStack>> + '_ {
        (0..36).map(move |i| self.slots[i].as_ref())
    }

    /// A stable hashable KEY per item KIND currently held — one per
    /// block / material / tool-type+tier / armour-piece / plan, ignoring stack
    /// size and tool/armour durability (two stacks of stone share a key; a wooden
    /// and an iron pickaxe get different keys). The Scavenger accumulates these
    /// across the run so its variety score counts every kind ever held. The high
    /// byte tags the family so the families never collide.
    pub fn item_kind_keys(&self) -> std::collections::HashSet<u64> {
        use crate::item::Item;
        let mut keys = std::collections::HashSet::new();
        for stack in self.slots_iter().flatten() {
            let key: u64 = match &stack.item {
                Item::Block(b) => *b as u64,
                Item::Tool(t) => (1u64 << 56) | ((t.tool_type as u64) << 8) | (t.material as u64),
                Item::Material(m) => (2u64 << 56) | (*m as u64),
                Item::Plan(_) => 3u64 << 56,
                Item::Armour(a) => (4u64 << 56) | ((a.slot as u64) << 8) | (a.material as u64),
            };
            keys.insert(key);
        }
        keys
    }

    /// Number of DISTINCT item KINDS currently held (see [`Self::item_kind_keys`]).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn distinct_item_kinds(&self) -> usize {
        self.item_kind_keys().len()
    }

    /// Add a block to inventory. Returns false if it didn't fit. Live callers
    /// use `add_item` directly instead.
    #[allow(dead_code)]
    pub fn add_block(&mut self, block_id: BlockId) -> bool {
        self.add_item(ItemStack::new_block(block_id, 1)).is_none()
    }

    /// Add any item stack to inventory. Tries to merge with existing stacks
    /// first, then finds empty slots. Returns the **unplaced remainder** — an
    /// `ItemStack` carrying whatever count couldn't fit — or `None` if every
    /// unit was placed.
    ///
    /// Returning the remainder (rather than a `bool`) is the contract that lets
    /// callers recover *exactly* what didn't fit. A bool can't distinguish "all
    /// placed" from "some placed": the chest-withdraw path treated `false` as
    /// "nothing landed" and re-inserted the whole stack into the chest while
    /// part had already reached the inventory → item duplication. See
    /// `chest_ui::withdraw_chest_slot`. Most callers `let _ =` the result
    /// (best-effort add, drop on full); the chest/craft paths that recover
    /// items MUST re-insert only this remainder, never the original stack.
    pub fn add_item(&mut self, stack: ItemStack) -> Option<ItemStack> {
        let mut remaining = stack.count;
        // First try to stack with existing matching items.
        for slot in self.slots.iter_mut() {
            if remaining == 0 { break; }
            if let Some(existing) = slot
                && existing.item.can_stack_with(&stack.item) {
                    // saturating_sub: a slot can legitimately hold > max_stack
                    // after a legacy/hand-edited save; never underflow here.
                    let space = existing.item.max_stack().saturating_sub(existing.count);
                    let add = remaining.min(space);
                    existing.count += add;
                    remaining -= add;
                }
        }
        // Then place remainder in empty slots.
        while remaining > 0 {
            let per_slot = remaining.min(stack.item.max_stack());
            let placed = self.slots.iter_mut().find(|s| s.is_none());
            if let Some(slot) = placed {
                *slot = Some(ItemStack { item: stack.item.clone(), count: per_slot });
                remaining -= per_slot;
            } else {
                // Inventory full — hand back exactly what didn't fit.
                return Some(ItemStack { item: stack.item, count: remaining });
            }
        }
        None
    }

    /// Peek at the block ID in a hotbar slot without consuming it (for creative mode).
    pub fn hotbar_block_id(&self, slot: usize) -> Option<BlockId> {
        if slot >= 9 { return None; }
        self.slots[slot].as_ref()?.item.as_block()
    }

    /// Try to use the selected hotbar item for block placement.
    /// Returns the block ID if the selected item is a placeable block.
    /// Decrements the stack count and removes the slot if empty.
    pub fn take_block_from_hotbar(&mut self, slot: usize) -> Option<BlockId> {
        if slot >= 9 { return None; }
        let block_id = self.slots[slot].as_ref()?.item.as_block()?;
        let stack = self.slots[slot].as_mut()?;
        stack.count -= 1;
        if stack.count == 0 {
            let used = stack.item.clone();
            self.slots[slot] = None;
            self.auto_refill_hotbar(slot, &used);
        }
        Some(block_id)
    }

    /// Generalised placement-take: returns the block id to place if the
    /// selected hotbar item is either a placeable block OR a material
    /// that's flagged as placeable via [`crate::item::material_as_placeable_block`].
    /// Decrements one from the stack regardless of which path matched.
    ///
    /// Wave 29 (log seasoning) — Green/Seasoned/Kiln-Dried log materials
    /// place as OAK_LOG so the player can build with logs straight from
    /// a tree-fell without a 1:1 crafting conversion.
    pub fn take_placeable_from_hotbar(&mut self, slot: usize) -> Option<BlockId> {
        if slot >= 9 { return None; }
        let block_id = {
            let stack_ref = self.slots[slot].as_ref()?;
            match &stack_ref.item {
                crate::item::Item::Block(b) => *b,
                crate::item::Item::Material(m) => crate::item::material_as_placeable_block(*m)?,
                crate::item::Item::Tool(_) => return None,
                // Spec 24 — Plans aren't placeable like blocks; they
                // enter Ghost mode via the Inspect dialog instead.
                crate::item::Item::Plan(_) => return None,
                // Spec 28e — armour is equip-only, not placeable.
                crate::item::Item::Armour(_) => return None,
            }
        };
        let stack = self.slots[slot].as_mut()?;
        stack.count -= 1;
        if stack.count == 0 {
            let used = stack.item.clone();
            self.slots[slot] = None;
            self.auto_refill_hotbar(slot, &used);
        }
        Some(block_id)
    }

    /// Creative-mode peek: like [`Self::hotbar_block_id`] but also resolves
    /// placeable materials to their block id. Doesn't consume.
    pub fn hotbar_placeable_id(&self, slot: usize) -> Option<BlockId> {
        if slot >= 9 { return None; }
        let stack = self.slots[slot].as_ref()?;
        match &stack.item {
            crate::item::Item::Block(b) => Some(*b),
            crate::item::Item::Material(m) => crate::item::material_as_placeable_block(*m),
            crate::item::Item::Tool(_) => None,
            crate::item::Item::Plan(_) => None,
            crate::item::Item::Armour(_) => None,
        }
    }

    /// Get the attack damage of the item in the selected hotbar slot.
    pub fn hotbar_attack_damage(&self, slot: usize) -> f32 {
        self.hotbar_slot(slot)
            .map(|s| s.item.attack_damage())
            .unwrap_or(1.0)
    }

    /// Take exactly one of whatever sits in the given hotbar slot, decrementing
    /// the stack (or clearing it). Returns the single-item stack, or `None` if
    /// the slot is empty / out of range. Used by Q-drop (Spec 5 §2.4).
    ///
    /// Tools are dropped whole (one tool occupies a slot regardless of count),
    /// so this returns the full tool stack and clears the slot.
    pub fn take_one_from_hotbar(&mut self, slot: usize) -> Option<ItemStack> {
        if slot >= 9 { return None; }
        let stack = self.slots[slot].as_mut()?;
        match &stack.item {
            crate::item::Item::Tool(_) => self.slots[slot].take(),
            _ => {
                let dropped = ItemStack { item: stack.item.clone(), count: 1 };
                stack.count -= 1;
                if stack.count == 0 {
                    self.slots[slot] = None;
                }
                Some(dropped)
            }
        }
    }

    /// Decrement the count of the named hotbar-slot material by one, clearing
    /// the slot if it hits zero. Returns true if the slot held that material.
    /// Used by right-click "use item" actions like bonemeal-on-grass.
    pub fn consume_one_material(&mut self, slot: usize, expected: crate::item::MaterialId) -> bool {
        if slot >= 9 { return false; }
        let stack = match self.slots[slot].as_mut() {
            Some(s) => s,
            None => return false,
        };
        match &stack.item {
            crate::item::Item::Material(id) if *id == expected => {}
            _ => return false,
        }
        stack.count -= 1;
        if stack.count == 0 {
            self.slots[slot] = None;
        }
        true
    }

    /// #44 P2 — number of empty slots across the 36-slot inventory. Surfaced as
    /// a HUD badge so the player can see at a glance when they're nearly full.
    pub fn free_slot_count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_none()).count()
    }

    /// #44 P2 — total arrows carried (plain + Nostrich-fletched), for the HUD
    /// arrow badge that mirrors the vanilla bow read-out.
    pub fn arrow_count(&self) -> u32 {
        use crate::item::MaterialId;
        self.count_material(MaterialId::Arrow) as u32
            + self.count_material(MaterialId::NostrichArrow) as u32
    }

    /// Spec 35 — total count of a given material across all 36 slots.
    /// Used by the Repair Bench to quote how much repair material the
    /// player has.
    pub fn count_material(&self, material: crate::item::MaterialId) -> u16 {
        let mut total: u16 = 0;
        for s in self.slots.iter().flatten() {
            if let crate::item::Item::Material(id) = &s.item
                && *id == material {
                    total = total.saturating_add(s.count as u16);
                }
        }
        total
    }

    /// Spec 35 — remove up to `count` units of a material across all
    /// slots (lowest slot first). Returns the number actually removed
    /// (may be less than `count` if the player ran short — caller
    /// should have checked `count_material` first). Empties drained
    /// stacks.
    pub fn consume_material(&mut self, material: crate::item::MaterialId, count: u16) -> u16 {
        let mut remaining = count;
        for slot in self.slots.iter_mut() {
            if remaining == 0 {
                break;
            }
            let take = match slot.as_mut() {
                Some(s) => match &s.item {
                    crate::item::Item::Material(id) if *id == material => {
                        let take = (s.count as u16).min(remaining);
                        s.count -= take as u8;
                        take
                    }
                    _ => 0,
                },
                None => 0,
            };
            if take > 0 {
                remaining -= take;
                if slot.as_ref().map(|s| s.count == 0).unwrap_or(false) {
                    *slot = None;
                }
            }
        }
        count - remaining
    }

    /// If the selected hotbar slot is food, consume one + return
    /// (food_value, poison_ticks). Returns None if the slot is empty or the
    /// item isn't food. The caller applies the heal and (if non-zero)
    /// the poison side-effect.
    pub fn try_eat_hotbar(&mut self, slot: usize) -> Option<(f32, u32)> {
        if slot >= 9 { return None; }
        let stack = self.slots[slot].as_mut()?;
        let value = stack.item.food_value()?;
        let poison = stack.item.eat_poison_ticks();
        stack.count -= 1;
        if stack.count == 0 {
            self.slots[slot] = None;
        }
        Some((value, poison))
    }

    /// Use the tool in the selected hotbar slot (reduce durability).
    /// Removes the tool if it breaks. Returns `Some(ToolUseInfo)` describing
    /// the durability change when a tool actually got used; `None` if the
    /// slot was empty or held a non-tool. Callers can use the before/after
    /// percentages to drive low-durability warnings or break toasts.
    pub fn use_hotbar_tool(&mut self, slot: usize) -> Option<ToolUseInfo> {
        if slot >= 9 { return None; }
        self.use_tool_at(slot)
    }

    /// [`Self::use_hotbar_tool`] for any of the 36 slots: wear the tool in
    /// slot `index` (removed if it breaks). A joiner's swing the server
    /// confirms after the sword moved out of the hotbar still wears it
    /// (`joiner_actions::apply_outcome`, review D2b LOW-1).
    pub fn use_tool_at(&mut self, index: usize) -> Option<ToolUseInfo> {
        let slot = index;
        if slot >= 36 { return None; }
        let stack = self.slots[slot].as_mut()?;
        let tool = stack.item.as_tool_mut()?;
        let max = crate::crafting::Tool::new(tool.tool_type, tool.material).durability as f32;
        let before_pct = if max > 0.0 { tool.durability as f32 / max } else { 0.0 };
        tool.use_tool();
        let after_pct = if max > 0.0 { tool.durability as f32 / max } else { 0.0 };
        let just_broke = tool.is_broken();
        let display_name = format!("{:?} {:?}",
            tool.material, tool.tool_type);
        if just_broke {
            self.slots[slot] = None;
        }
        Some(ToolUseInfo { before_pct, after_pct, just_broke, display_name })
    }

    /// Get the display name of the item in a hotbar slot.
    pub fn hotbar_item_name(&self, slot: usize, registry: &crate::block::BlockRegistry) -> String {
        self.hotbar_slot(slot)
            .map(|s| s.item.name(registry))
            .unwrap_or_else(|| "Empty".to_string())
    }

    /// Get the display colour of the item in a hotbar slot. Live UI code
    /// (hud_ui.rs) computes `stack.item.color(registry)` inline instead of
    /// through this wrapper.
    #[allow(dead_code)]
    pub fn hotbar_item_color(&self, slot: usize, registry: &crate::block::BlockRegistry) -> [f32; 3] {
        self.hotbar_slot(slot)
            .map(|s| s.item.color(registry))
            .unwrap_or([0.1, 0.1, 0.1])
    }

    /// See the `open` field's note — nothing reads the state this flips.
    #[allow(dead_code)]
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// For save/load: get raw slot data. The live save path
    /// (`save::serialize_inventory`) iterates via `.slot(i)` instead.
    #[allow(dead_code)]
    pub fn raw_slots(&self) -> &[Option<ItemStack>; 36] {
        &self.slots
    }
}

/// #45 — pure deterministic sort over a slice of slots, honouring a parallel
/// `locked` mask. Locked positions keep their exact contents; all unlocked
/// stacks are gathered, partial stacks of the same item are merged (filling to
/// `max_stack`), the result is ordered by [`Item::sort_key`] (and count desc),
/// and the sorted stacks are re-laid into the unlocked positions in order. The
/// total count of every item is conserved — nothing is ever created or dropped.
///
/// `locked` is indexed in lockstep with `slots`; a shorter/absent mask treats
/// the missing tail as unlocked.
pub fn sort_slots(slots: &[Option<ItemStack>], locked: &[bool]) -> Vec<Option<ItemStack>> {
    let n = slots.len();
    let is_locked = |i: usize| locked.get(i).copied().unwrap_or(false);

    // Gather unlocked stacks, then merge same-item partials up to max_stack.
    let mut merged: Vec<ItemStack> = Vec::new();
    for (i, slot) in slots.iter().enumerate() {
        if is_locked(i) {
            continue;
        }
        let Some(stack) = slot else { continue };
        let max = stack.item.max_stack() as u32;
        let mut remaining = stack.count as u32;
        // Top up existing compatible stacks first.
        for m in merged.iter_mut() {
            if remaining == 0 {
                break;
            }
            if m.item.can_stack_with(&stack.item) {
                let space = max.saturating_sub(m.count as u32);
                let add = remaining.min(space);
                m.count = (m.count as u32 + add) as u8;
                remaining -= add;
            }
        }
        // Spill the rest into fresh stacks (max-sized chunks).
        while remaining > 0 {
            let take = remaining.min(max.max(1));
            merged.push(ItemStack { item: stack.item.clone(), count: take as u8 });
            remaining -= take;
        }
    }

    // Order: category/id key, then larger stacks first within an item.
    merged.sort_by(|a, b| {
        a.item
            .sort_key()
            .cmp(&b.item.sort_key())
            .then(b.count.cmp(&a.count))
    });

    // Re-lay: locked slots untouched; unlocked positions filled in sort order.
    let mut result: Vec<Option<ItemStack>> = slots.to_vec();
    let unlocked: Vec<usize> = (0..n).filter(|&i| !is_locked(i)).collect();
    for &i in &unlocked {
        result[i] = None;
    }
    for (stack, &pos) in merged.into_iter().zip(unlocked.iter()) {
        result[pos] = Some(stack);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Item, ItemStack, MaterialId};

    #[test]
    fn item_to_ref_maps_tool_tier() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        use crate::protocol::ItemRef;
        let iron = Item::Tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron));
        assert_eq!(item_to_ref(&iron), ItemRef::Tool(2));
        let wood = Item::Tool(Tool::new(ToolType::Axe, ToolMaterial::Wood));
        assert_eq!(item_to_ref(&wood), ItemRef::Tool(0));
        let stone = Item::Tool(Tool::new(ToolType::Sword, ToolMaterial::Stone));
        assert_eq!(item_to_ref(&stone), ItemRef::Tool(1));
        let diamond = Item::Tool(Tool::new(ToolType::Shovel, ToolMaterial::Diamond));
        assert_eq!(item_to_ref(&diamond), ItemRef::Tool(3));
        let satori = Item::Tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Satori));
        assert_eq!(item_to_ref(&satori), ItemRef::Tool(4));
    }

    #[test]
    fn item_to_ref_maps_block_and_material() {
        use crate::protocol::ItemRef;
        assert_eq!(item_to_ref(&Item::Block(7)), ItemRef::Block(7));
        // MaterialId::Stick is discriminant 0.
        assert_eq!(
            item_to_ref(&Item::Material(MaterialId::Stick)),
            ItemRef::Material(MaterialId::Stick as u16)
        );
    }

    #[test]
    fn item_from_ref_decodes_block_and_material() {
        let reg = crate::block::BlockRegistry::new();
        assert_eq!(
            item_from_ref(crate::protocol::item_kind::BLOCK, 7, &reg),
            Some(Item::Block(7))
        );
        assert_eq!(
            item_from_ref(
                crate::protocol::item_kind::MATERIAL,
                MaterialId::Leather as u16,
                &reg
            ),
            Some(Item::Material(MaterialId::Leather))
        );
    }

    #[test]
    fn item_from_ref_rejects_hostile_and_lossy_refs() {
        let reg = crate::block::BlockRegistry::new();
        // Unknown block id from the network must be rejected, not silently
        // decoded to the registry's AIR fallback.
        assert_eq!(item_from_ref(crate::protocol::item_kind::BLOCK, u16::MAX, &reg), None);
        // Out-of-range material discriminant.
        assert_eq!(
            item_from_ref(crate::protocol::item_kind::MATERIAL, u16::MAX, &reg),
            None
        );
        // Tools collapse to a tier on the wire (kind/durability lost) — no
        // faithful Item can be reconstructed, so the decode refuses.
        assert_eq!(item_from_ref(crate::protocol::item_kind::TOOL, 2, &reg), None);
        assert_eq!(item_from_ref(crate::protocol::item_kind::EMPTY, 0, &reg), None);
        // Unknown discriminator byte.
        assert_eq!(item_from_ref(99, 0, &reg), None);
    }

    #[test]
    fn try_eat_hotbar_consumes_one_food_returns_value() {
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_material(MaterialId::RawBeef, 3)));
        let result = inv.try_eat_hotbar(0);
        assert_eq!(result, Some((3.0, 0))); // RawBeef food_value, no poison
        assert_eq!(inv.slot(0).map(|s| s.count), Some(2));
    }

    #[test]
    fn try_eat_hotbar_clears_slot_at_zero() {
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_material(MaterialId::RawChicken, 1)));
        let _ = inv.try_eat_hotbar(0);
        assert!(inv.slot(0).is_none());
    }

    #[test]
    fn try_eat_hotbar_rejects_non_food() {
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Bone, 5)));
        let result = inv.try_eat_hotbar(0);
        assert_eq!(result, None);
        assert_eq!(inv.slot(0).map(|s| s.count), Some(5));
    }

    #[test]
    fn try_eat_hotbar_empty_slot_returns_none() {
        let mut inv = Inventory::new();
        let result = inv.try_eat_hotbar(0);
        assert_eq!(result, None);
    }

    #[test]
    fn try_eat_hotbar_invalid_slot_returns_none() {
        let mut inv = Inventory::new();
        inv.set_slot(5, Some(ItemStack::new_material(MaterialId::RawBeef, 1)));
        // Slot 9+ is main inventory, not hotbar.
        assert_eq!(inv.try_eat_hotbar(9), None);
        assert_eq!(inv.try_eat_hotbar(35), None);
    }

    #[test]
    fn take_one_from_hotbar_decrements_block_stack() {
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_block(1, 5)));
        let dropped = inv.take_one_from_hotbar(0).unwrap();
        assert_eq!(dropped.count, 1);
        assert!(matches!(dropped.item, Item::Block(1)));
        assert_eq!(inv.slot(0).map(|s| s.count), Some(4));
    }

    #[test]
    fn take_one_from_hotbar_clears_slot_at_zero() {
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_block(2, 1)));
        let _ = inv.take_one_from_hotbar(0);
        assert!(inv.slot(0).is_none());
    }

    #[test]
    fn take_one_from_hotbar_drops_whole_tool() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let mut inv = Inventory::new();
        let tool = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
        inv.set_slot(0, Some(ItemStack { item: Item::Tool(tool), count: 1 }));
        let dropped = inv.take_one_from_hotbar(0).unwrap();
        assert!(matches!(dropped.item, Item::Tool(_)));
        assert!(inv.slot(0).is_none());
    }

    #[test]
    fn take_one_from_hotbar_empty_slot_returns_none() {
        let mut inv = Inventory::new();
        assert!(inv.take_one_from_hotbar(0).is_none());
    }

    #[test]
    fn take_one_from_hotbar_rejects_non_hotbar_index() {
        let mut inv = Inventory::new();
        inv.set_slot(10, Some(ItemStack::new_block(1, 5)));
        assert!(inv.take_one_from_hotbar(10).is_none());
        assert_eq!(inv.slot(10).map(|s| s.count), Some(5));
    }

    #[test]
    fn add_item_returns_none_when_everything_placed() {
        let mut inv = Inventory::new();
        assert!(inv.add_item(ItemStack::new_block(crate::block::STONE, 10)).is_none());
        assert_eq!(inv.slot(0).map(|s| s.count), Some(10));
    }

    #[test]
    fn add_item_returns_whole_stack_when_inventory_full() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let mut inv = Inventory::new();
        // 36 tools (max_stack 1) occupy every slot — nothing can be placed.
        for i in 0..36 {
            inv.set_slot(i, Some(ItemStack::new_tool(
                Tool::new(ToolType::Pickaxe, ToolMaterial::Iron))));
        }
        let remainder = inv.add_item(ItemStack::new_block(crate::block::STONE, 5));
        assert_eq!(remainder.map(|s| s.count), Some(5),
            "nothing fit → the whole stack comes back as remainder");
    }

    #[test]
    fn add_item_returns_only_the_unplaced_remainder_on_partial_fit() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let mut inv = Inventory::new();
        // 35 tool slots + one empty slot. STONE max_stack is 64.
        for i in 0..35 {
            inv.set_slot(i, Some(ItemStack::new_tool(
                Tool::new(ToolType::Pickaxe, ToolMaterial::Iron))));
        }
        // 100 stone: 64 land in the empty slot, 36 are unplaced.
        let remainder = inv.add_item(ItemStack::new_block(crate::block::STONE, 100));
        assert_eq!(remainder.map(|s| s.count), Some(36));
    }

    #[test]
    fn free_slot_count_tracks_emptiness() {
        // #44 P2 — the HUD free-slot badge reads this. Empty = 36; each
        // distinct occupied slot drops it by one.
        let mut inv = Inventory::new();
        assert_eq!(inv.free_slot_count(), 36);
        inv.set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 1)));
        assert_eq!(inv.free_slot_count(), 35);
        inv.set_slot(35, Some(ItemStack::new_block(crate::block::STONE, 1)));
        assert_eq!(inv.free_slot_count(), 34);
        // Clearing a slot frees it again.
        inv.set_slot(0, None);
        assert_eq!(inv.free_slot_count(), 35);
    }

    #[test]
    fn arrow_count_sums_both_arrow_materials() {
        use crate::item::MaterialId;
        // #44 P2 — the HUD arrow badge counts plain + Nostrich arrows.
        let mut inv = Inventory::new();
        assert_eq!(inv.arrow_count(), 0);
        inv.add_item(ItemStack::new_material(MaterialId::Arrow, 10));
        inv.add_item(ItemStack::new_material(MaterialId::NostrichArrow, 5));
        assert_eq!(inv.arrow_count(), 15);
        // A non-arrow material doesn't inflate the count.
        inv.add_item(ItemStack::new_material(MaterialId::Flint, 7));
        assert_eq!(inv.arrow_count(), 15);
    }

    #[test]
    fn add_item_does_not_underflow_on_an_overfull_slot() {
        // A slot holding MORE than max_stack is reachable via the chest-cap
        // bug or a hand-edited/legacy save. The space calc must saturate, not
        // wrap/panic.
        let mut inv = Inventory::new();
        let mut overfull = ItemStack::new_block(crate::block::STONE, 64);
        overfull.count = 200; // > max_stack (64)
        inv.set_slot(0, Some(overfull));
        let remainder = inv.add_item(ItemStack::new_block(crate::block::STONE, 5));
        assert!(remainder.is_none(), "the 5 spill into a fresh slot without panicking");
        assert_eq!(inv.slot(1).map(|s| s.count), Some(5));
    }

    #[test]
    fn food_predicates_match_food_value() {
        assert!(Item::Material(MaterialId::RawBeef).is_food());
        assert!(!Item::Material(MaterialId::Bone).is_food());
        assert!(!Item::Material(MaterialId::Stick).is_food());
        assert_eq!(Item::Material(MaterialId::RawChicken).food_value(), Some(2.0));
    }

    // ── #45 Phase 1 — sort + locked slots ────────────────────────────

    /// Block id in a slot, if it holds a block (avoids match-ergonomics deref
    /// ambiguity in the assertions below).
    fn block_id(slot: &Option<ItemStack>) -> Option<crate::block::BlockId> {
        match slot {
            Some(ItemStack { item: Item::Block(b), .. }) => Some(*b),
            _ => None,
        }
    }

    /// Total count of a given block id across a slice (count-conservation helper).
    fn total_block(slots: &[Option<ItemStack>], id: crate::block::BlockId) -> u32 {
        slots
            .iter()
            .filter(|s| block_id(s) == Some(id))
            .map(|s| s.as_ref().unwrap().count as u32)
            .sum()
    }

    #[test]
    fn sort_slots_merges_partial_stacks() {
        // Two partial stone stacks (30 + 40) merge to a full 64 + a 6, packed
        // to the front; total is conserved.
        let slots = vec![
            Some(ItemStack::new_block(crate::block::STONE, 30)),
            None,
            Some(ItemStack::new_block(crate::block::STONE, 40)),
            None,
        ];
        let locked = vec![false; 4];
        let out = sort_slots(&slots, &locked);
        assert_eq!(total_block(&out, crate::block::STONE), 70, "count conserved");
        assert_eq!(out[0].as_ref().map(|s| s.count), Some(64), "first stack filled to max");
        assert_eq!(out[1].as_ref().map(|s| s.count), Some(6), "remainder in next slot");
        assert!(out[2].is_none() && out[3].is_none(), "merged → trailing slots empty");
    }

    #[test]
    fn sort_slots_orders_blocks_before_materials() {
        // Category order: blocks (0) sort ahead of materials (1).
        let slots = vec![
            Some(ItemStack::new_material(MaterialId::Stick, 5)),
            Some(ItemStack::new_block(crate::block::DIRT, 5)),
        ];
        let out = sort_slots(&slots, &vec![false; 2]);
        assert!(matches!(out[0].as_ref().unwrap().item, Item::Block(_)), "block first");
        assert!(matches!(out[1].as_ref().unwrap().item, Item::Material(_)), "material second");
    }

    #[test]
    fn sort_slots_respects_locked_positions() {
        // A locked slot keeps its exact item and index; only unlocked slots are
        // gathered/sorted around it.
        let slots = vec![
            Some(ItemStack::new_material(MaterialId::Stick, 3)), // unlocked
            Some(ItemStack::new_block(crate::block::DIRT, 7)),   // LOCKED — stays put
            Some(ItemStack::new_block(crate::block::STONE, 9)),  // unlocked
        ];
        let mut locked = vec![false; 3];
        locked[1] = true;
        let out = sort_slots(&slots, &locked);
        // Slot 1 is untouched.
        assert_eq!(block_id(&out[1]), Some(crate::block::DIRT));
        assert_eq!(out[1].as_ref().unwrap().count, 7);
        // The two unlocked items fill positions 0 and 2 in sort order (block, then material).
        assert_eq!(block_id(&out[0]), Some(crate::block::STONE));
        assert!(matches!(out[2].as_ref().unwrap().item, Item::Material(_)));
    }

    #[test]
    fn sort_slots_conserves_every_item_count() {
        let slots = vec![
            Some(ItemStack::new_block(crate::block::STONE, 50)),
            Some(ItemStack::new_block(crate::block::DIRT, 20)),
            Some(ItemStack::new_block(crate::block::STONE, 50)),
            None,
            Some(ItemStack::new_block(crate::block::DIRT, 44)),
        ];
        let out = sort_slots(&slots, &vec![false; 5]);
        assert_eq!(total_block(&out, crate::block::STONE), 100);
        assert_eq!(total_block(&out, crate::block::DIRT), 64);
    }

    #[test]
    fn auto_refill_restocks_emptied_hotbar_block_from_bag() {
        // Hotbar slot 0 has the last stone; the bag (slot 9) has a full stack.
        // Placing the last block auto-pulls the bag stack into the hotbar.
        let mut inv = Inventory::new();
        inv.auto_refill = true;
        inv.set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 1)));
        inv.set_slot(9, Some(ItemStack::new_block(crate::block::STONE, 64)));
        let placed = inv.take_block_from_hotbar(0);
        assert_eq!(placed, Some(crate::block::STONE));
        assert_eq!(inv.slot(0).map(|s| s.count), Some(64), "hotbar restocked from bag");
        assert!(inv.slot(9).is_none(), "bag stack moved up");
    }

    #[test]
    fn auto_refill_off_leaves_slot_empty() {
        let mut inv = Inventory::new();
        inv.auto_refill = false;
        inv.set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 1)));
        inv.set_slot(9, Some(ItemStack::new_block(crate::block::STONE, 64)));
        inv.take_block_from_hotbar(0);
        assert!(inv.slot(0).is_none(), "no refill when disabled");
        assert_eq!(inv.slot(9).map(|s| s.count), Some(64), "bag untouched");
    }

    #[test]
    fn auto_refill_skips_locked_source() {
        let mut inv = Inventory::new();
        inv.auto_refill = true;
        inv.set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 1)));
        inv.set_slot(9, Some(ItemStack::new_block(crate::block::STONE, 64)));
        inv.toggle_lock(9); // protect the bag stack
        inv.take_block_from_hotbar(0);
        assert!(inv.slot(0).is_none(), "locked source not pulled");
        assert_eq!(inv.slot(9).map(|s| s.count), Some(64), "locked stack stays");
    }

    #[test]
    fn inventory_lock_toggle_and_sort_region() {
        let mut inv = Inventory::new();
        assert!(!inv.is_locked(9));
        inv.toggle_lock(9);
        assert!(inv.is_locked(9));
        inv.toggle_lock(9);
        assert!(!inv.is_locked(9));
        // Scatter stone across the main region and sort it.
        inv.set_slot(11, Some(ItemStack::new_block(crate::block::STONE, 30)));
        inv.set_slot(20, Some(ItemStack::new_block(crate::block::STONE, 30)));
        inv.sort_region(9, 36);
        // 60 stone merged into one slot at the region front (slot 9).
        assert_eq!(inv.slot(9).map(|s| s.count), Some(60));
        assert!(inv.slot(11).is_none() && inv.slot(20).is_none());
    }

    #[test]
    fn distinct_item_kinds_counts_kinds_not_quantity() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        let mut inv = Inventory::new();
        assert_eq!(inv.distinct_item_kinds(), 0, "empty bag holds no kinds");
        inv.set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 64)));
        inv.set_slot(1, Some(ItemStack::new_block(crate::block::STONE, 64)));
        assert_eq!(inv.distinct_item_kinds(), 1, "two stacks of stone = one kind");
        inv.set_slot(2, Some(ItemStack::new_block(crate::block::DIRT, 64)));
        assert_eq!(inv.distinct_item_kinds(), 2, "stone + dirt = two kinds");
        inv.set_slot(3, Some(ItemStack::new_material(MaterialId::Coal, 8)));
        assert_eq!(inv.distinct_item_kinds(), 3, "+ a material = three kinds");
        // Two pickaxes of different TIERS are two kinds.
        let mut wood = Tool::new(ToolType::Pickaxe, ToolMaterial::Wood);
        inv.set_slot(4, Some(ItemStack::new_tool(wood.clone())));
        inv.set_slot(5, Some(ItemStack::new_tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron))));
        assert_eq!(inv.distinct_item_kinds(), 5, "wood + iron pickaxe = two kinds");
        // Same pickaxe tier at different durability is STILL one kind.
        wood.durability = wood.durability.saturating_sub(1);
        inv.set_slot(6, Some(ItemStack::new_tool(wood)));
        assert_eq!(inv.distinct_item_kinds(), 5, "durability doesn't change the kind");
    }

    // ─── Full-fidelity item wire (death-drops phase 3, v61) ───

    #[test]
    fn wire_full_round_trips_every_tool_type_and_material() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        use crate::item::Item;
        let types = [
            ToolType::Pickaxe,
            ToolType::Axe,
            ToolType::Sword,
            ToolType::Shovel,
            ToolType::Bow,
            ToolType::Hoe,
            ToolType::FlintAndSteel,
            ToolType::Shears,
            ToolType::FishingRod,
            ToolType::Slingshot,
            ToolType::Eraser,
            ToolType::DraftingStamp,
        ];
        let mats = [
            ToolMaterial::Wood,
            ToolMaterial::Stone,
            ToolMaterial::Iron,
            ToolMaterial::Diamond,
            ToolMaterial::Satori,
        ];
        for tt in types {
            for m in mats {
                let mut tool = Tool::new(tt, m);
                // Worn to just under half — the whole point of the payload.
                tool.durability = tool.max_durability() / 2 + 1;
                let wire = super::item_to_wire_full(&Item::Tool(tool));
                let back = super::item_from_wire_full(&wire).expect("tool decodes");
                assert_eq!(back, Item::Tool(tool), "{tt:?}/{m:?} survives the wire");
            }
        }
    }

    #[test]
    fn wire_full_round_trips_every_armour_slot_and_material() {
        use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot};
        use crate::item::Item;
        let slots = [
            ArmourSlot::Helmet,
            ArmourSlot::Chestplate,
            ArmourSlot::Leggings,
            ArmourSlot::Boots,
        ];
        let mats = [
            ArmourMaterial::Leather,
            ArmourMaterial::Iron,
            ArmourMaterial::Diamond,
            ArmourMaterial::Satori,
            ArmourMaterial::Chainmail,
            ArmourMaterial::Rubber,
        ];
        for slot in slots {
            for m in mats {
                let mut piece = ArmourItem::new(slot, m);
                piece.durability = piece.durability / 3 + 1;
                let wire = super::item_to_wire_full(&Item::Armour(piece));
                let back = super::item_from_wire_full(&wire).expect("armour decodes");
                assert_eq!(back, Item::Armour(piece), "{slot:?}/{m:?} survives the wire");
            }
        }
    }

    #[test]
    fn wire_full_is_none_for_blocks_materials_and_plans() {
        use crate::item::Item;
        use crate::protocol::WireItem;
        assert_eq!(
            super::item_to_wire_full(&Item::Block(crate::block::STONE)),
            WireItem::None,
            "the (kind, id) pair already carries a block losslessly"
        );
        assert_eq!(
            super::item_to_wire_full(&Item::Material(crate::item::MaterialId::Bone)),
            WireItem::None
        );
        assert_eq!(
            super::item_to_wire_full(&Item::Plan(crate::plan::PlanData::debug_3x3_stone())),
            WireItem::None,
            "plans stay floor-bound by design (heavy PlanData)"
        );
        assert!(super::item_from_wire_full(&WireItem::None).is_none());
    }

    #[test]
    fn wire_full_clamps_over_max_durability() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        use crate::item::Item;
        use crate::protocol::WireItem;
        let max = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron).max_durability();
        let decoded = super::item_from_wire_full(&WireItem::Tool {
            tool_type: 0,
            material: 2,
            durability: u16::MAX,
        })
        .expect("clamped, not refused");
        let Item::Tool(t) = decoded else { panic!("expected a tool") };
        assert_eq!(t.durability, max, "over-max durability clamps to the cap");

        let armour_max =
            crate::armour::max_durability(crate::armour::ArmourSlot::Boots, crate::armour::ArmourMaterial::Leather);
        let decoded = super::item_from_wire_full(&WireItem::Armour {
            slot: 3,
            material: 0,
            durability: u16::MAX,
        })
        .expect("clamped, not refused");
        let Item::Armour(a) = decoded else { panic!("expected armour") };
        assert_eq!(a.durability, armour_max);
    }

    #[test]
    fn wire_full_refuses_zero_durability_and_unknown_bytes() {
        use crate::protocol::WireItem;
        // A broken tool/piece must never be granted.
        assert!(super::item_from_wire_full(&WireItem::Tool {
            tool_type: 0,
            material: 2,
            durability: 0
        })
        .is_none());
        assert!(super::item_from_wire_full(&WireItem::Armour {
            slot: 0,
            material: 1,
            durability: 0
        })
        .is_none());
        // Unrecognised bytes from a newer/tampered peer — refuse, never guess.
        assert!(super::item_from_wire_full(&WireItem::Tool {
            tool_type: 200,
            material: 2,
            durability: 10
        })
        .is_none());
        assert!(super::item_from_wire_full(&WireItem::Tool {
            tool_type: 0,
            material: 200,
            durability: 10
        })
        .is_none());
        assert!(super::item_from_wire_full(&WireItem::Armour {
            slot: 200,
            material: 1,
            durability: 10
        })
        .is_none());
        assert!(super::item_from_wire_full(&WireItem::Armour {
            slot: 0,
            material: 200,
            durability: 10
        })
        .is_none());
    }
}
