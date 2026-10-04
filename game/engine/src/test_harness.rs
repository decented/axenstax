//! Headless test fixture. Wraps `GameServer` directly for synchronous,
//! deterministic tick control — bypasses the transport/thread plumbing of
//! `HostedServer`, which is self-clocked and not usable for fast tests.
//!
//! This is the Spec 3 Layer B foundation. Integration tests in
//! `test_integration/` build on it. When Spec 2 Phase 0 lands the in-process
//! transport pair, this harness will grow a variant that drives `HostedServer`
//! through packets; for now the direct-GameServer path is enough to cover
//! mob spawn, falling blocks, player physics, and AI assertions.
//!
//! Crate-only — `#[cfg(test)] mod test_harness;` in `main.rs`.

use glam::Vec3;

use crate::server::GameServer;

/// Construction options. `Default` yields a 1-player survival-normal world
/// with seed 42 and **no** initial chunk/mob scatter — fastest for unit tests.
/// Set `do_initial_load = true` for tests that need real terrain.
pub struct TestConfig {
    pub seed: u32,
    pub num_players: usize,
    pub difficulty: String,
    pub is_creative: bool,
    pub play_mode: crate::play_mode::PlayMode,
    pub world_name: String,
    /// When true, runs `GameServer::initial_load()` which generates terrain.
    /// Slow (seconds). Default false — most tests don't need real chunks.
    pub do_initial_load: bool,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            num_players: 1,
            difficulty: "normal".to_string(),
            is_creative: false,
            play_mode: crate::play_mode::PlayMode::Survival,
            world_name: "test-world".to_string(),
            do_initial_load: false,
        }
    }
}

/// The harness itself. Owns a `GameServer` and exposes helpers for driving
/// the sim from a test. A future extension will add packet-level plumbing
/// once Spec 1 Phase 3 wires the auth path through `HostedServer`.
pub struct TestHost {
    pub server: GameServer,
    /// Per-player camera perspective mode. Lives here (not on the server, which
    /// is render-agnostic) because `CameraMode` is a client-render concept — the
    /// harness stands in for the client. Drives [`TestHost::crosshair_target`]'s
    /// eye-anchored-aim invariant probe.
    camera_modes: Vec<crate::camera::CameraMode>,
    /// Per-player Phase-2 no-snap collision fraction `[0,1]` (1.0 = fully
    /// extended). Persisted across [`TestHost::update_camera_collision`] calls so
    /// a test can exercise clamp-in + ease-out exactly as the game loop does.
    collision_frac: Vec<f32>,
}

impl TestHost {
    /// Build a new headless host from `config`.
    pub fn start_with(config: TestConfig) -> Self {
        let num_players = config.num_players;
        // Integration tests were written against seed-42 terrain; keep that so
        // their world expectations stay stable (#8 threads a real seed only on
        // the production hosted-server path).
        let mut server = GameServer::new(num_players, config.world_name.clone(), 42);
        if config.do_initial_load {
            server.initial_load();
        }
        server.set_play_mode(config.play_mode);
        Self {
            server,
            camera_modes: vec![crate::camera::CameraMode::FirstPerson; num_players],
            collision_frac: vec![1.0; num_players],
        }
    }

    /// Advance the server by `n` ticks.
    pub fn tick(&mut self, n: u32) {
        for _ in 0..n {
            self.server.tick();
        }
    }

    /// Read-only access to the server ECS for assertions.
    pub fn ecs(&self) -> &hecs::World {
        &self.server.ecs
    }

    /// Set a block directly in the authoritative world (test fixture).
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, block: crate::block::BlockId) {
        self.server.world.set_block(x, y, z, block);
    }

    /// Read a block from the authoritative world.
    pub fn get_block(&self, x: i32, y: i32, z: i32) -> crate::block::BlockId {
        self.server.world.get_block(x, y, z)
    }

    /// Read-only access to the authoritative world (test fixture) — for tests
    /// that call free functions taking `&World` directly (e.g. rail neighbour
    /// reads) instead of going through a `TestHost` wrapper method.
    pub fn world(&self) -> &crate::world::World {
        &self.server.world
    }

    /// Simulate a *player* placing `block` at `(x,y,z)` — flags the voxel
    /// player-placed, mirroring the production placement path
    /// (`World::place_player_block`). Use this (not `set_block`) when a test
    /// needs the proof-of-play anti-farming gate to see the block as placed.
    pub fn place_block(&mut self, x: i32, y: i32, z: i32, block: crate::block::BlockId) {
        self.server.world.place_player_block(x, y, z, block);
    }

    /// Move player `i` to a specific position (test fixture — bypasses physics).
    pub fn teleport_player(&mut self, i: usize, pos: Vec3) {
        self.server.players[i].player.pos = pos;
        self.server.players[i].player.velocity = Vec3::ZERO;
    }

    /// Position of player `i`.
    pub fn player_pos(&self, i: usize) -> Vec3 {
        self.server.players[i].player.pos
    }

    /// Set the world's play mode on the wrapped server.
    pub fn set_play_mode(&mut self, mode: crate::play_mode::PlayMode) {
        self.server.set_play_mode(mode);
    }

    /// Simulate player 0 breaking the block at `(x,y,z)` with their currently
    /// held tool, mirroring the survival break path's proof-of-play work
    /// accrual via the SAME production functions (`can_harvest` + `block_work`
    /// + `World::add_work`). Returns the work added — `0` if the block isn't
    /// harvestable with the held tool (no work done). Sets the block to AIR.
    pub fn mine_block(&mut self, x: i32, y: i32, z: i32) -> u64 {
        let block = self.server.world.get_block(x, y, z);
        let held = self.server.players[0].hotbar_slot;
        let tool = self.server.players[0]
            .inventory
            .hotbar_slot(held)
            .and_then(|s| match &s.item {
                crate::item::Item::Tool(t) => Some(t.clone()),
                _ => None,
            });
        // Shared decision point with the production break path
        // (`game_loop.rs`): player-placed blocks earn no work (Spec 06 §2.2).
        let harvestable = crate::crafting::can_harvest(block, tool.as_ref());
        let was_placed = self.server.world.is_placed(x, y, z);
        let work = crate::crafting::break_work(block, harvestable, was_placed);
        self.server.world.set_block(x, y, z, crate::block::AIR);
        self.server.world.set_placed(x, y, z, false);
        self.server.world.add_work(work);
        work
    }

    /// Aim player `i`'s look (yaw/pitch in radians). Drives `Camera::forward()`,
    /// which — together with `eye_pos()` — is the origin of every gameplay ray.
    pub fn set_player_look(&mut self, i: usize, yaw: f32, pitch: f32) {
        self.server.players[i].yaw = yaw;
        self.server.players[i].pitch = pitch;
    }

    /// Set player `i`'s camera perspective mode. Third-person moves only the
    /// render origin — it must NOT move the crosshair target (see
    /// [`Self::crosshair_target`]).
    pub fn set_camera_mode(&mut self, i: usize, mode: crate::camera::CameraMode) {
        self.camera_modes[i] = mode;
    }

    /// The block cell player `i`'s crosshair is pointing at — the cell a
    /// mine/place/attack ray would strike. Mirrors the production block-targeting
    /// raycast (`game_loop.rs`: `cast_ray(eye_pos(), camera.forward(),
    /// REACH_DISTANCE, …)`) EXACTLY: it rays from the true eye along the look
    /// direction, NEVER the render eye. The player's `CameraMode` is loaded onto
    /// the camera so a future "improvement" that (wrongly) origins the aim at
    /// `camera.render_eye()` would shift the hit cell and fail the invariant
    /// test. Returns `None` if nothing solid is within reach.
    pub fn crosshair_target(&self, i: usize) -> Option<[i32; 3]> {
        let slot = &self.server.players[i];
        let eye = slot.player.eye_pos();
        // Reconstruct the client camera from the relayed look + stored mode, so
        // `forward()` is the exact same computation the game uses.
        let mut camera = crate::camera::Camera::new(eye, 1.0);
        camera.yaw = slot.yaw;
        camera.pitch = slot.pitch;
        camera.mode = self.camera_modes[i];
        crate::raycast::cast_ray(
            eye,
            camera.forward(),
            crate::REACH_DISTANCE,
            &self.server.world,
            &self.server.registry,
        )
        .map(|h| h.block_pos)
    }

    /// The true eye of player `i` (the body's eye — also the camera `position`
    /// and the origin of every gameplay ray). Reference point for measuring the
    /// third-person render-eye pull-back in collision tests.
    pub fn camera_eye(&self, i: usize) -> Vec3 {
        self.server.players[i].player.eye_pos()
    }

    /// Rebuild the client camera for slot `i` from the relayed pos/look + the
    /// stored mode and collision fraction (mirrors [`Self::crosshair_target`]'s
    /// reconstruction). The harness stands in for the client, which owns the
    /// camera; the server is render-agnostic.
    fn rebuild_camera(&self, i: usize) -> crate::camera::Camera {
        let slot = &self.server.players[i];
        let mut camera = crate::camera::Camera::new(slot.player.eye_pos(), 1.0);
        camera.yaw = slot.yaw;
        camera.pitch = slot.pitch;
        camera.mode = self.camera_modes[i];
        camera.collision_frac = self.collision_frac[i];
        camera
    }

    /// Phase 2 — advance the no-snap third-person camera-collision smoothing one
    /// step (`dt` seconds) against the live world, exactly as the game loop's
    /// fixed tick does (raycast → target fraction → smooth). Call repeatedly to
    /// exercise clamp-in and ease-out. **Render-only** — never moves the aim ray.
    pub fn update_camera_collision(&mut self, i: usize, dt: f32) {
        let mut camera = self.rebuild_camera(i);
        let target = crate::raycast::camera_collision_fraction(
            &camera,
            &self.server.world,
            &self.server.registry,
        );
        camera.update_collision(target, dt);
        self.collision_frac[i] = camera.collision_frac;
    }

    /// The current (collision-clamped) render-eye for slot `i` — the point the
    /// frame is viewed FROM. Equals the eye in first-person; pulled back (and
    /// clamped before walls) in third-person.
    pub fn camera_render_eye(&self, i: usize) -> Vec3 {
        self.rebuild_camera(i).render_eye()
    }

    // ── Rail freight (Phase 1) helpers ──────────────────────────────────────

    /// Insert a chest block-entity carrying `stacks` at `(x,y,z)`. A "depot" is
    /// just a chest adjacent to a track terminus — no dedicated block — so this
    /// is also how depots are set up. `stacks` fill the first slots in order.
    pub fn insert_chest_with(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        stacks: &[crate::item::ItemStack],
    ) {
        debug_assert!(
            stacks.len() <= crate::chest::CHEST_SLOTS,
            "insert_chest_with: {} stacks exceeds CHEST_SLOTS ({})",
            stacks.len(),
            crate::chest::CHEST_SLOTS
        );
        let mut chest = crate::chest::ChestData::new();
        for (i, s) in stacks.iter().enumerate() {
            chest.slots[i] = Some(s.clone());
        }
        self.server.world.insert_chest((x, y, z), chest);
    }

    /// Read-only view of the chest at `(x,y,z)`, if any.
    pub fn chest_at(&self, x: i32, y: i32, z: i32) -> Option<&crate::chest::ChestData> {
        self.server.world.chest_at((x, y, z))
    }

    /// Spawn a parked cart on track cell `(x,y,z)`. Returns the entity id.
    pub fn spawn_cart(&mut self, x: i32, y: i32, z: i32) -> hecs::Entity {
        crate::cart::spawn_cart(&mut self.server.ecs, (x, y, z))
    }

    /// Dispatch the parked cart on cell `(x,y,z)` heading in `look_dir`, running
    /// the production [`crate::cart::dispatch_cart`] path: seed direction + load
    /// freight from an adjacent depot chest + depart. Returns `Some(entity)` of
    /// the dispatched cart, or `None` if no parked cart was on the cell.
    pub fn dispatch_cart(&mut self, x: i32, y: i32, z: i32, look_dir: Vec3) -> Option<hecs::Entity> {
        crate::cart::dispatch_cart(
            &mut self.server.ecs,
            &mut self.server.world,
            (x, y, z),
            look_dir,
        )
    }

    /// Spawn a parked cart with a specific armour [`Hull`] tier on track cell
    /// `(x,y,z)` (CA4 tests need iron/diamond carts to compare breach time).
    /// Returns the entity id.
    pub fn spawn_cart_with_hull(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        hull: crate::cart::Hull,
    ) -> hecs::Entity {
        crate::cart::spawn_cart_with_hull(&mut self.server.ecs, (x, y, z), hull)
    }

    /// CA4 — lay `amount` break-work against cart `id`'s hull via the production
    /// [`crate::cart::apply_breach`] core (the SAME function the game-loop break
    /// interception calls). Returns `true` iff the cart broke this call (despawn
    /// + drops fired). Drives the breach-to-break test without the client-input
    /// wiring (which break-tick laid the work).
    pub fn breach_cart(&mut self, id: hecs::Entity, amount: f32) -> bool {
        crate::cart::apply_breach(&mut self.server.ecs, id, amount)
    }

    /// Whether a cart entity with id `id` is still present in the server ECS
    /// (a broken cart is despawned, so this returns false afterwards).
    pub fn cart_alive(&self, id: hecs::Entity) -> bool {
        self.server.ecs.get::<&crate::cart::CartData>(id).is_ok()
    }

    /// Count dropped `ItemEntity`s in the ECS whose stack is `material`,
    /// summing their counts — lets a test assert the cart item + each spilled
    /// cargo stack landed on the ground after a breach.
    pub fn dropped_material_count(&self, material: crate::item::MaterialId) -> u32 {
        self.server
            .ecs
            .query::<&crate::entity::ItemEntity>()
            .iter()
            .filter_map(|(_, ie)| match &ie.stack.item {
                crate::item::Item::Material(m) if *m == material => Some(u32::from(ie.stack.count)),
                _ => None,
            })
            .sum()
    }

    /// The `CartData` for entity `id` (test assertions on cell/speed/cargo).
    pub fn cart_data(&self, id: hecs::Entity) -> crate::cart::CartData {
        (*self.server.ecs.get::<&crate::cart::CartData>(id).unwrap()).clone()
    }

    /// Ticks remaining in the server's rain/storm windows, computed the exact
    /// same way `hosted_server.rs`'s `broadcast_state` computes the
    /// `rain_ticks_left`/`storm_ticks_left` it puts on every StateUpdate
    /// (`Weather::ticks_left(tick_counter)`). `TestHost` deliberately
    /// bypasses the transport/`HostedServer` plumbing (see the module doc),
    /// so this is how a P9 weather-sync test asserts what a real broadcast
    /// would have carried without standing up the full packet pipeline.
    pub fn weather_ticks_left(&self) -> (u32, u32) {
        self.server.weather.ticks_left(self.server.tick_counter)
    }

    // ── Survival-chain helpers (mine → smelt → craft) ───────────────────────

    /// Select the hotbar slot holding a `material` tool of `tool_type`. Returns
    /// `false` (and leaves the selection alone) if the player has no such tool —
    /// a test that crafted one asserts on this rather than swinging the wrong
    /// tier by accident.
    pub fn equip_tool(
        &mut self,
        tool_type: crate::crafting::ToolType,
        material: crate::crafting::ToolMaterial,
    ) -> bool {
        for i in 0..9 {
            let is_it = matches!(
                self.server.players[0].inventory.hotbar_slot(i).map(|s| &s.item),
                Some(crate::item::Item::Tool(t))
                    if t.tool_type == tool_type && t.material == material
            );
            if is_it {
                self.server.players[0].hotbar_slot = i;
                return true;
            }
        }
        false
    }

    /// How many of `slot`'s item player 0 is carrying (summed across stacks).
    pub fn carrying(&self, slot: crate::crafting::CraftSlot) -> u32 {
        self.server.players[0]
            .inventory
            .slots_iter()
            .flatten()
            .filter(|s| crate::crafting::CraftSlot::from_item(&s.item) == slot)
            .map(|s| u32::from(s.count))
            .sum()
    }

    /// Break the block at `(x,y,z)` in survival and pocket what it drops,
    /// running the production break path's decision points: the tier gate
    /// (`crafting::can_harvest` against the held tool), the drop table
    /// (`BlockRegistry::mine_drop`), and `Inventory::add_item`. Returns the
    /// drop, or `None` when the held tool is too soft to harvest it (the block
    /// still breaks — that IS the game's behaviour).
    pub fn mine_into_inventory(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<crate::item::ItemStack> {
        let block = self.server.world.get_block(x, y, z);
        let held = self.server.players[0].hotbar_slot;
        let tool = self.server.players[0]
            .inventory
            .hotbar_slot(held)
            .and_then(|s| match &s.item {
                crate::item::Item::Tool(t) => Some(t.clone()),
                _ => None,
            });
        let harvestable = crate::crafting::can_harvest(block, tool.as_ref());
        self.server.world.set_block(x, y, z, crate::block::AIR);
        self.server.world.set_placed(x, y, z, false);
        if !harvestable {
            return None;
        }
        let drop = self.server.registry.mine_drop(block);
        if drop.count == 0 {
            return None;
        }
        self.server.players[0].inventory.add_item(drop.clone());
        Some(drop)
    }

    /// Craft `grid` for player 0: every filled cell spends ONE matching item out
    /// of the inventory, the recipe is resolved by the production
    /// [`crate::crafting::match_recipe`], and the result goes back in. Returns
    /// `None` — spending nothing — if the grid matches no recipe or the player
    /// is short of an ingredient.
    pub fn craft(
        &mut self,
        grid: [[crate::crafting::CraftSlot; 3]; 3],
    ) -> Option<crate::item::ItemStack> {
        use crate::crafting::CraftSlot;
        let out = crate::crafting::match_recipe(&grid)?;
        let wanted: Vec<CraftSlot> = grid
            .iter()
            .flatten()
            .copied()
            .filter(|s| *s != CraftSlot::Empty)
            .collect();
        // Check the whole bill before spending any of it, so a failed craft
        // never eats half the ingredients.
        for w in &wanted {
            let need = wanted.iter().filter(|x| *x == w).count() as u32;
            if self.carrying(*w) < need {
                return None;
            }
        }
        for w in wanted {
            assert!(self.take_one(w), "ingredient vanished mid-craft: {w:?}");
        }
        self.server.players[0].inventory.add_item(out.clone());
        Some(out)
    }

    /// Spend one of `slot`'s item from player 0's inventory.
    fn take_one(&mut self, slot: crate::crafting::CraftSlot) -> bool {
        let inv = &mut self.server.players[0].inventory;
        for i in 0..36 {
            let matches = inv
                .slot(i)
                .is_some_and(|s| crate::crafting::CraftSlot::from_item(&s.item) == slot);
            if matches {
                let mut stack = inv.take_slot(i).expect("slot just matched");
                if stack.count > 1 {
                    stack.count -= 1;
                    inv.set_slot(i, Some(stack));
                }
                return true;
            }
        }
        false
    }

    /// Tap the rubber log at `(x,y,z)` with a bucket: convert it to
    /// `RUBBER_LOG_TAPPED` (production [`crate::rubber::apply_tap`], cooldown
    /// stamped off the monotonic tick) and pocket the 1 Rubber. Returns whether
    /// the log was tappable.
    pub fn tap_rubber_log(&mut self, x: i32, y: i32, z: i32) -> bool {
        let now = self.server.tick_counter;
        if !crate::rubber::apply_tap(&mut self.server.world, (x, y, z), now) {
            return false;
        }
        self.server.players[0]
            .inventory
            .add_item(crate::item::ItemStack::new_material(
                crate::item::MaterialId::Rubber,
                1,
            ));
        true
    }

    /// Place `block` out of player 0's own inventory — the stack is spent, so a
    /// test can only build what it actually crafted. Returns `false` (placing
    /// nothing) if they are not carrying one. Power blocks get their device via
    /// [`Self::place_power_block`].
    pub fn place_from_inventory(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        block: crate::block::BlockId,
        facing: crate::meta::Facing,
    ) -> bool {
        if !self.take_one(crate::crafting::CraftSlot::Block(block)) {
            return false;
        }
        if crate::block::is_power_block(block) {
            self.place_power_block(x, y, z, block, facing);
        } else {
            self.server.world.place_player_block(x, y, z, block);
        }
        true
    }

    /// Stand a loaded furnace at `(x,y,z)` — the block plus a `FurnaceData`
    /// holding `input` and `fuel`.
    pub fn load_furnace(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        input: crate::item::ItemStack,
        fuel: crate::item::ItemStack,
    ) {
        self.server.world.set_block(x, y, z, crate::block::FURNACE);
        self.server.world.insert_furnace(
            (x, y, z),
            crate::furnace::FurnaceData {
                input: Some(input),
                fuel: Some(fuel),
                ..Default::default()
            },
        );
    }

    /// Run `n` passes of the furnace smelt sweep over every furnace in the
    /// world (the production [`crate::furnace::tick_all`]). A `GameServer` only
    /// ticks furnaces itself when `simulates_block_machines` is set (the
    /// dedicated server, T1-3); `TestHost` leaves that flag off, so the
    /// harness stands in for the host client's sweep here. Don't combine this
    /// with the flag — that would smelt twice per pass.
    pub fn tick_furnaces(&mut self, n: u32) {
        for _ in 0..n {
            let _ = crate::furnace::tick_all(&mut self.server.world);
        }
    }

    /// Take everything out of the furnace at `(x,y,z)`'s output slot and put it
    /// in player 0's inventory (the "Take" click).
    pub fn take_furnace_output(&mut self, x: i32, y: i32, z: i32) -> Option<crate::item::ItemStack> {
        let out = self.server.world.furnace_at_mut((x, y, z))?.output.take()?;
        self.server.players[0].inventory.add_item(out.clone());
        Some(out)
    }

    // ── Spec 48 (Electricity) helpers ───────────────────────────────────────

    /// Place a power block the way a player does: the block, its metadata
    /// facing, the matching `PowerDevice` (cables carry none), and the dirty +
    /// neighbour-notify pair that seeds the network. Mirrors the client place
    /// arm in `game_loop.rs`, sharing its
    /// [`crate::power::device_kind_for_block`] table.
    pub fn place_power_block(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        block: crate::block::BlockId,
        facing: crate::meta::Facing,
    ) {
        debug_assert!(
            crate::block::is_power_block(block),
            "place_power_block on a non-power block ({block})"
        );
        let pos = (x, y, z);
        self.server.world.set_block(x, y, z, block);
        if let Some(kind) = crate::power::device_kind_for_block(block) {
            self.server
                .world
                .insert_power_device(pos, crate::power::PowerDeviceData::new(kind, facing));
            self.server.world.set_meta(pos, crate::meta::with_facing(0, facing));
        }
        self.server.world.mark_dirty(pos);
        self.server.world.notify_neighbours(pos);
    }

    /// Stamp the canonical facing byte on `(x,y,z)` — how a directional
    /// non-power block (a piston) records which way it points.
    pub fn set_meta_facing(&mut self, x: i32, y: i32, z: i32, facing: crate::meta::Facing) {
        let m = self.server.world.meta_at(x, y, z);
        self.server
            .world
            .set_meta((x, y, z), crate::meta::with_facing(m, facing));
    }

    /// Break a power block the way a player does: clear the cell, DROP the
    /// device entity (a forgotten one lives on as a ghost source), and nudge the
    /// neighbours so the run settles. Mirrors the `game_loop.rs` break arms.
    pub fn break_power_block(&mut self, x: i32, y: i32, z: i32) {
        let pos = (x, y, z);
        let prev = self.server.world.get_block(x, y, z);
        self.server.world.set_block(x, y, z, crate::block::AIR);
        if crate::block::is_power_block(prev) {
            self.server.world.block_entities.remove(&pos);
        }
        self.server.world.notify_neighbours(pos);
    }

    /// The power device at `(x,y,z)`, if any.
    pub fn power_device(&self, x: i32, y: i32, z: i32) -> Option<&crate::power::PowerDeviceData> {
        self.server.world.power_device_at((x, y, z))
    }

    /// Is the device at `(x,y,z)` latched on / turning / tripped? Panics if
    /// there is no device there — a test asserting "off" on an empty cell is
    /// asserting nothing.
    pub fn device_on(&self, x: i32, y: i32, z: i32) -> bool {
        self.power_device(x, y, z)
            .unwrap_or_else(|| panic!("no power device at ({x},{y},{z})"))
            .on
    }

    /// Right-click the toggle-class device at `(x,y,z)` — the production path,
    /// `power::interact_device`, and nothing reimplemented here. Panics if the
    /// cell holds no such device: a test "flipping" an empty cell asserts
    /// nothing. Returns the changes the interaction wants broadcast.
    fn interact(&mut self, x: i32, y: i32, z: i32) -> Vec<crate::protocol::BlockChange> {
        let now = self.server.tick_counter;
        crate::power::interact_device(&mut self.server.world, (x, y, z), now)
            .unwrap_or_else(|| panic!("no interactable power device at ({x},{y},{z})"))
            .changes
    }

    /// Flip the lever at `(x,y,z)` — the right-click path: latch, mirror the
    /// state into the meta bit the handle shape reads, re-seed the network.
    pub fn toggle_lever(&mut self, x: i32, y: i32, z: i32) {
        assert_eq!(
            self.power_device(x, y, z).map(|d| d.kind),
            Some(crate::power::PowerDeviceKind::Lever),
            "no lever at ({x},{y},{z})"
        );
        let _ = self.interact(x, y, z);
    }

    /// Push the button (or plunger) at `(x,y,z)`: drive it now and schedule the
    /// 10-tick auto-release, exactly as the right-click path does.
    pub fn press_button(&mut self, x: i32, y: i32, z: i32) {
        assert!(
            matches!(
                self.power_device(x, y, z).map(|d| d.kind),
                Some(
                    crate::power::PowerDeviceKind::Button
                        | crate::power::PowerDeviceKind::PlungerDetonator
                )
            ),
            "no button at ({x},{y},{z})"
        );
        let _ = self.interact(x, y, z);
    }

    /// Give the hand crank at `(x,y,z)` one turn — a full `CRANK_RUN_TICKS`
    /// window of charge.
    pub fn turn_crank(&mut self, x: i32, y: i32, z: i32) {
        assert_eq!(
            self.power_device(x, y, z).map(|d| d.kind),
            Some(crate::power::PowerDeviceKind::HandCrank),
            "no hand crank at ({x},{y},{z})"
        );
        let _ = self.interact(x, y, z);
    }

    /// Right-click `held` onto the Steam Generator at `(x,y,z)`. Returns whether
    /// the fuel slot took it (the production
    /// [`crate::power::try_load_generator_fuel`] refuses non-fuels).
    pub fn fuel_generator(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        held: &crate::item::ItemStack,
    ) -> bool {
        let pos = (x, y, z);
        let loaded = self
            .server
            .world
            .power_device_at_mut(pos)
            .is_some_and(|d| crate::power::try_load_generator_fuel(d, held));
        if loaded {
            self.server.world.mark_dirty(pos);
            self.server.world.notify_neighbours(pos);
        }
        loaded
    }

    /// Set the operation of the Logic Gate at `(x,y,z)` (the right-click cycle).
    pub fn set_gate_op(&mut self, x: i32, y: i32, z: i32, op: crate::power::GateOp) {
        let pos = (x, y, z);
        match self.server.world.power_device_at_mut(pos) {
            Some(d) if d.kind == crate::power::PowerDeviceKind::LogicGate => d.gate_op = op,
            _ => panic!("no logic gate at ({x},{y},{z})"),
        }
        self.server.world.mark_dirty(pos);
        self.server.world.notify_neighbours(pos);
    }

    /// Run one pass of the piston sweep (production
    /// [`crate::piston::tick_pistons`]) and return the cells it changed. Like
    /// [`Self::tick_furnaces`], this stands in for the host client's sweep —
    /// `GameServer::tick` only runs it with `simulates_block_machines` set (the
    /// dedicated server, T1-3), which `TestHost` leaves off.
    pub fn tick_pistons(&mut self) -> Vec<(i32, i32, i32)> {
        crate::piston::tick_pistons(&mut self.server.world)
    }

    /// Pour a water source at `(x,y,z)` — the block plus the registration the
    /// spread system needs, exactly as the bucket-placement path does.
    pub fn place_water_source(&mut self, x: i32, y: i32, z: i32) {
        self.server.world.set_block(x, y, z, crate::block::WATER);
        self.server.world.set_meta((x, y, z), 0);
        self.server.water.add_source(x, y, z);
    }

    /// Drain the source at `(x,y,z)` — clears the cell and queues the retraction
    /// that dries the run downstream.
    pub fn remove_water_source(&mut self, x: i32, y: i32, z: i32) {
        self.server.world.set_block(x, y, z, crate::block::AIR);
        self.server.water.remove_source(x, y, z);
    }

    /// Put the world under a thunderstorm for the next `ticks` ticks. The wind
    /// (`wind::sample`) reads the same window, so this is how a test blows a gale
    /// without waiting for the weather to roll one.
    pub fn force_storm(&mut self, ticks: u64) {
        let now = self.server.tick_counter;
        self.server.weather = crate::weather::Weather {
            rain_until: now + ticks,
            storm_until: now + ticks,
        };
    }

    /// The wind the NEXT tick will blow at height `y` — the same three inputs
    /// (`tick_counter + 1`, weather window, seed) `GameServer::tick` feeds
    /// `wind::sample`, so a test can assert on the breeze the mills are about to
    /// see rather than guessing.
    pub fn wind_next_tick(&self, y: i32) -> crate::wind::WindSample {
        crate::wind::sample(
            self.server.tick_counter + 1,
            self.server.weather,
            self.server.seed,
            y,
        )
    }
}
