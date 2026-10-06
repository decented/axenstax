//! Chunk streaming: incremental loading/unloading and initial bulk load.

use crate::chunk::CHUNK_SIZE;
use crate::mesh::build_chunk_meshes;
use super::{MAX_CHUNK_Y, STREAM_BUDGET};

impl super::GameState {
    /// Generate and mesh chunks around all players (union of needed columns).
    /// First call does a bulk initial load; subsequent calls stream incrementally.
    pub(crate) fn stream_chunks(&mut self) {
        let cs = CHUNK_SIZE as i32;
        // Live render distance (Spec 39 — was the `RENDER_DISTANCE` const).
        let rd = self.graphics.render_distance;

        // ── Part G: Union chunk streaming — needed columns across all players ──
        let mut needed_columns = ahash::AHashSet::new();
        for slot in &self.players {
            let pcx = (slot.player.pos.x.floor() as i32).div_euclid(cs);
            let pcz = (slot.player.pos.z.floor() as i32).div_euclid(cs);
            for dx in -rd..=rd {
                for dz in -rd..=rd {
                    needed_columns.insert((pcx + dx, pcz + dz));
                }
            }
        }

        // Use player 0 for initial load centre
        let pcx0 = (self.players[0].player.pos.x.floor() as i32).div_euclid(cs);
        let pcz0 = (self.players[0].player.pos.z.floor() as i32).div_euclid(cs);

        if self.loaded_columns.is_empty() {
            // Fallback path only: the normal world entry runs through the
            // `GameMode::Loading` state, which drives `begin_load` + `step_load`
            // incrementally (and applies any spawn-pref override itself), so by
            // the time Playing calls this `loaded_columns` is already populated
            // and we skip to streaming below. This blocking `initial_load`
            // (= begin_load + drain) guards any path that reaches Playing with
            // no columns loaded.
            self.initial_load(pcx0, pcz0);
            return;
        }

        // Normal terrain worlds floor every column with bedrock at y=0
        // (`biome_block_at` is unconditional there); flat/Workshop floor at
        // other Ys, so y=0 bedrock isn't their "is this generated?" signal.
        let normal_terrain = !self.world.is_workshop && !self.world.has_flat_floor();

        // Find unloaded columns within the union set, sorted closest to player 0.
        // SELF-HEAL: also re-queue a column that's marked loaded but has NO bedrock
        // floor — a "void column" (the floor-grid-holes bug) the player would drop
        // through. The plain `!loaded` guard alone never revisits such a column, so
        // a generate/bookkeeping divergence becomes a permanent hole. Re-running
        // generate_column is idempotent (it skips already-filled chunks).
        let mut to_load: Vec<(i32, i32, i32)> = Vec::new();
        let mut healed = 0u32;
        for &(cx, cz) in &needed_columns {
            let loaded = self.loaded_columns.contains(&(cx, cz));
            let void = loaded
                && normal_terrain
                && self.world.get_block(cx * cs + 8, 0, cz * cs + 8) != crate::block::BEDROCK;
            if !loaded || void {
                if void {
                    healed += 1;
                }
                let dx = cx - pcx0;
                let dz = cz - pcz0;
                to_load.push((dx * dx + dz * dz, cx, cz));
            }
        }
        if healed > 0 {
            log::warn!(
                "stream_chunks self-heal: re-generating {healed} void column(s) \
                 (marked loaded but unfloored) near player 0"
            );
        }

        if !to_load.is_empty() {
            to_load.sort_by_key(|&(d, _, _)| d);

            for &(_, cx, cz) in to_load.iter().take(STREAM_BUDGET) {
                // Spec 02 §7.5 — an evicted (edited / saved) column comes back
                // from the store; only a never-kept column is (re)generated. This
                // also covers the void self-heal: it never regenerates over an
                // evicted column. The rest mirrors the save-load path (light,
                // fluid/fire rescan, mesh).
                load_column_blocks(&mut self.world, cx, cz, &self.biome_gen, true);
                                crate::lighting::run_initial_pass_for_column(&mut self.world, cx, cz, &self.registry);
                self.water.register_column_sources(cx, cz, &self.world);
                self.lava.register_column_sources(cx, cz, &self.world);
                self.fire.register_column_fires(cx, cz, &self.world, self.tick_counter);
                crate::entity::scatter_mobs_in_column(
                    &mut self.ecs, cx, cz, &self.world, &self.biome_gen,
                );
                self.loaded_columns.insert((cx, cz));

                // Mesh new chunks in this column
                for cy in 0..=MAX_CHUNK_Y {
                    if self.world.has_chunk(cx, cy, cz) {
                        let meshes = build_chunk_meshes(cx, cy, cz, &self.world, &self.registry);
                        self.renderer.upload_chunk((cx, cy, cz), &meshes);
                    }
                }

                // Re-mesh neighbouring columns' boundary chunks (so hidden faces cull properly)
                for &(ndx, ndz) in &[(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let nx = cx + ndx;
                    let nz = cz + ndz;
                    if self.loaded_columns.contains(&(nx, nz)) {
                        for cy in 0..=MAX_CHUNK_Y {
                            if self.world.has_chunk(nx, cy, nz) {
                                let meshes =
                                    build_chunk_meshes(nx, cy, nz, &self.world, &self.registry);
                                self.renderer.upload_chunk((nx, cy, nz), &meshes);
                            }
                        }
                    }
                }
            }
        }

        // Unload columns that are outside ALL players' render distance
        let unload_dist = rd + 2;
        let player_cols: Vec<(i32, i32)> = self
            .players
            .iter()
            .map(|slot| {
                (
                    (slot.player.pos.x.floor() as i32).div_euclid(cs),
                    (slot.player.pos.z.floor() as i32).div_euclid(cs),
                )
            })
            .collect();
        let to_remove = columns_to_unload(&self.loaded_columns, &player_cols, unload_dist);

        for (cx, cz) in to_remove {
            self.loaded_columns.remove(&(cx, cz));
            // Reclaim the column's scattered wildlife so re-entry re-scatters a
            // fresh set rather than piling onto the old (engine audit B). Tamed
            // pets / villagers / golems aren't `Scattered`, so they survive.
            crate::entity::despawn_mobs_in_column(&mut self.ecs, cx, cz);
            // Spec 02 §7.5 — edited / saved columns are kept in the evicted
            // store (and still saved); pristine world-gen is dropped.
            unload_column_blocks(&mut self.world, cx, cz);
            for cy in 0..=MAX_CHUNK_Y {
                self.renderer.chunk_meshes.remove(&(cx, cy, cz));
                self.renderer.water_meshes.remove(&(cx, cy, cz));
                self.renderer.plant_meshes.remove(&(cx, cy, cz));
                self.renderer.decal_meshes.remove(&(cx, cy, cz));
                // Owner-inbox #18 — shed this chunk's per-type micro-model
                // instance buffers + far-LOD billboards too (else they leak +
                // render stale once flowers register).
                self.renderer.micro_meshes.remove(&(cx, cy, cz));
                self.renderer.micro_billboard_meshes.remove(&(cx, cy, cz));
            }
        }
    }

    /// Blocking world load — `begin_load` then drain the whole queue. Kept as
    /// the `stream_chunks` fallback; the live path drives `begin_load` +
    /// `step_load` incrementally from the `GameMode::Loading` state. Params are
    /// unused (the load centre is derived from the restored/placed player);
    /// retained so the call-site signature is unchanged.
    pub(crate) fn initial_load(&mut self, _pcx: i32, _pcz: i32) {
        self.begin_load();
        while !self.load_queue.is_empty() {
            self.step_load(usize::MAX);
        }
    }

    /// One-shot world-load setup: read the save (or mark fresh), restore players,
    /// place the spawn, apply any spawn-pref override, rebuild overrides, then
    /// build `load_queue` (every column within render distance ∪ saved columns,
    /// nearest-first). The heavy per-column gen+light+mesh is drained by
    /// `step_load`, so the loading screen animates instead of freezing.
    pub(crate) fn begin_load(&mut self) {
        // Owner-inbox #18 — upload one shared baked shell per registered
        // micro-model type before chunks stream in (idempotent + cheap; empty
        // until a block is bound in Phase C). Per-chunk instance buffers are then
        // populated by `upload_chunk`.
        self.renderer.sync_micro_models(&self.world.micro_registry);
        // Spec 40 persistence — restore the player's global wardrobe on world enter.
        // Native loads synchronously here; WASM kicks off the async Stash load (drained
        // by the main loop). `wardrobe_remember` gates whether the personal layer is
        // applied (read before the reapply that `seed`-equivalent code triggers).
        self.wardrobe_remember = crate::wardrobe_store::load_remember();
        self.wardrobe_user_acted = false;
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(set) = crate::wardrobe_store::load() {
                let base = crate::texture_gen::texture_count();
                self.world.player_wardrobe =
                    crate::override_registry::OverrideRegistry::from_set(set, base);
            }
        }
        #[cfg(target_arch = "wasm32")]
        self.kick_off_wardrobe_load();
        // Live render distance (Spec 39 — was the `RENDER_DISTANCE` const).
        let rd = self.graphics.render_distance;
        // One-shot resume intent from the WASM poll branch (see the field doc
        // on `GameState.world_preloaded`) — consumed here so it can never leak
        // into a later world entry.
        let world_preloaded = std::mem::take(&mut self.world_preloaded);
        // Try loading a saved world first (check autosave for crash recovery)
        // A joined session never reads a local save: an old "remote_game"
        // folder (written by builds before 2026-09-28) would otherwise restore
        // another server's inventory, position and chunks into this one.
        let load_result = if !self.persists_locally() {
            Err("joined session — the host's world, no local save".to_string())
        } else if crate::save::has_autosave(&self.world_name) {
            log::info!("Autosave found for '{}' — recovering from crash...", self.world_name);
            crate::save::load_autosave(&self.world_name, &mut self.world)
                .inspect(|_| crate::save::clear_autosave(&self.world_name))
        } else if crate::save::world_exists(&self.world_name) {
            log::info!("Loading saved world '{}'...", self.world_name);
            crate::save::load_world(&self.world_name, &mut self.world)
        } else {
            Err("No save found".to_string())
        };

        // Save path = resuming an existing world; the `else if` below is a
        // fresh world (no save / load failed AND nothing preloaded), which is
        // where the one-time kits + spawn placement live so they never re-apply
        // on re-entry (2026-06-16 playtest: Axolittle "loads of blocks you
        // havnt gotten" + "spawn in flying" on every join). A WASM resume takes
        // NEITHER branch: the sync load always fails there, but the game_loop
        // poll branch already restored world + players (`world_preloaded`), so
        // running FRESH would clobber the restored position + kit slots.
        let load_ok = load_result.is_ok();
        if let Ok((save_data, _chunk_count)) = load_result {
            // Build per-player restore list: new saves have a `players` Vec;
            // old saves have an empty Vec — fall back to legacy single-player fields.
            let player_saves: Vec<crate::save::PlayerSaveData> = if save_data.players.is_empty() {
                vec![crate::save::PlayerSaveData {
                    x: save_data.player_x,
                    y: save_data.player_y,
                    z: save_data.player_z,
                    yaw: 0.0,
                    pitch: 0.0,
                    health: save_data.player_health,
                    hotbar_slot: save_data.hotbar_slot,
                    inventory: save_data.inventory.clone(),
                    // Legacy single-player save: no spawn_pos field.
                    // Loader falls back to last position.
                    spawn_pos: None,
                    hunger: 20,
                    reputation: vec![],
                    tamed_pets: vec![],
                    armour_slots: [None, None, None, None],
                    kill_counter: vec![],
                    bounties_claimed: vec![],
                }]
            } else {
                save_data.players.clone()
            };

            let meta = crate::save::load_world_meta(&self.world_name);
            self.set_play_mode(crate::play_mode::PlayMode::from_meta_str(&meta.game_mode));
            self.is_commands_enabled = meta.commands_enabled;
            self.explosives_enabled = meta.explosives_enabled;
            self.fire_spread_enabled = meta.fire_spread_enabled;
            self.difficulty = meta.difficulty.clone();

            // Pre-allocate slots + GPU resources for every saved player before the
            // restore loop runs. Previously get_mut(i) silently returned None for
            // indices beyond the initial single PlayerSlot, so a 2-player save
            // would drop Player 2 on load. Cross-platform: WASM now supports
            // up to 4 local players via the web Gamepad backend (spec 15).
            {
                while self.players.len() < player_saves.len() {
                    let idx = self.players.len();
                    let p_save = &player_saves[idx];
                    let spawn = glam::Vec3::new(p_save.x, p_save.y, p_save.z);
                    let slot = crate::player_slot::PlayerSlot::new(idx, spawn, 0.5);
                    self.players.push(slot);

                    let gpu = self.renderer.create_player_resources();
                    self.renderer.player_gpu.push(gpu);
                }

                if self.players.len() > 1 {
                    self.screens = crate::screen::compute_screen_layout(
                        self.players.len(),
                        self.renderer.width,
                        self.renderer.height,
                    );
                    for screen in &self.screens {
                        let pidx = match &screen.content {
                            crate::screen::ScreenContent::LocalPlayer(sidx) => *sidx,
                        };
                        if let Some(p) = self.players.get_mut(pidx) {
                            p.camera.aspect = screen.viewport.aspect();
                        }
                    }
                }
            }

            // Restore each player from save
            for (i, p_save) in player_saves.iter().enumerate() {
                if let Some(slot) = self.players.get_mut(i) {
                    slot.player.pos = glam::Vec3::new(p_save.x, p_save.y, p_save.z);
                    slot.player.velocity = glam::Vec3::ZERO;
                    slot.player.reset_fall();
                    slot.camera.yaw = p_save.yaw;
                    slot.camera.pitch = p_save.pitch;
                    slot.combat.health = p_save.health;
                    slot.combat.hunger = p_save.hunger;
                    slot.hotbar_slot = p_save.hotbar_slot;
                    // Resume grounded — never auto-fly on load. Creative can still
                    // double-tap space to fly; flight capability is gated by the
                    // play mode in `Player::tick`, not by this initial state
                    // (2026-06-16 playtest: Axolittle "spawn in flying… you dont
                    // want that"). Survival/Adventure were already grounded.
                    slot.player.flying = false;
                    crate::save::restore_inventory(&mut slot.inventory, &p_save.inventory);
                    slot.armour_slots = crate::save::restore_armour_slots(&p_save.armour_slots);
                    // Spec 19 — reputation rebuild from the flattened save
                    // Vec. Legacy saves arrive with an empty Vec so the
                    // restore is a no-op (rep starts at Neutral with every
                    // village), preserving the prior load behaviour.
                    slot.reputation = crate::reputation::Reputation::default();
                    for &(vid, score) in &p_save.reputation {
                        slot.reputation.per_village.insert(vid, score);
                    }
                    // Spec 33 — restore kill_counter + bounties_claimed.
                    // Legacy saves arrive empty via #[serde(default)].
                    // kill_counter persistence means a player who killed
                    // 9 zombies and quits resumes at 9, not 0.
                    slot.kill_counter.clear();
                    for &(kind, n) in &p_save.kill_counter {
                        slot.kill_counter.insert(kind, n);
                    }
                    slot.bounties_claimed.clear();
                    for &(id, n) in &p_save.bounties_claimed {
                        slot.bounties_claimed.insert(id, n);
                    }
                    // Wave 20: bed-set spawn-point persistence. New saves
                    // carry it; legacy saves (None) fall back to the
                    // player's last position so they respawn near where
                    // they quit, not at world spawn.
                    slot.spawn_pos = match p_save.spawn_pos {
                        Some([sx, sy, sz]) => glam::Vec3::new(sx, sy, sz),
                        None => slot.player.pos,
                    };
                }
            }

            // Wave 3 (#45 P3) — restore player 0's locked inventory slots
            // (top-level field; split-screen players' locks aren't persisted yet).
            if let Some(p0) = self.players.get_mut(0) {
                p0.inventory.set_locked_from(&save_data.locked_slots);
            }

            // Mark loaded columns from existing chunks
            for (cx, _cy, cz) in self.world.chunk_positions() {
                self.loaded_columns.insert((cx, cz));
            }

            // Rail freight Phase 1 (Task 1.6) — re-spawn persisted carts into
            // the SAME ECS this client ticks (`cart::tick_carts` reads
            // `self.ecs`), so a saved in-flight cart resumes rolling on reload.
            // Carts are ECS entities, not block-entities, so they aren't part of
            // `apply_world_save_state` (which only touches the voxel `World`) —
            // they need the ECS handle, which is in scope here. Old saves have an
            // empty `carts` Vec via `#[serde(default)]`, so this is a no-op for
            // pre-rail worlds.
            for s in &save_data.carts {
                crate::cart::spawn_cart_from(&mut self.ecs, s.data.clone());
            }

            // Animals Wave 2 — re-spawn persisted TAMED pets (wolves,
            // nostriches) into this client's ECS, carrying their saved owner +
            // AI/tame state. NOT tagged `Scattered`, so they survive chunk
            // unload like a villager/golem would. Old saves have an empty
            // `saved_mobs` Vec via `#[serde(default)]`, so this is a no-op for
            // pre-Wave-2 worlds. (Untamed wildlife isn't persisted — it
            // re-scatters deterministically.)
            for pet in &save_data.saved_mobs {
                let kind = pet.data.mob_type();
                let pos = glam::Vec3::new(pet.x, pet.y, pet.z);
                let id = crate::entity::spawn_mob(&mut self.ecs, kind, pos);
                match &pet.data {
                    crate::save::SavedTamedPetData::Wolf(d) => {
                        // Belt-and-braces (Task 7): a save written before the
                        // save-time fix can still have a persisted
                        // AttackHostile/AttackRecentAttacker baked into its
                        // bytes, carrying a stale hecs entity id — sanitize
                        // on the way back in too.
                        let mut d = d.clone();
                        d.sanitize_attack_state_for_persistence();
                        let _ = self.ecs.insert_one(id, d);
                    }
                    crate::save::SavedTamedPetData::Nostrich(d) => {
                        let _ = self.ecs.insert_one(id, d.clone());
                    }
                    crate::save::SavedTamedPetData::Companion { data, .. } => {
                        // Legacy save (pre-1C): no command state on the wire —
                        // CompanionData restores with state = Follow (Default).
                        let _ = self.ecs.insert_one(id, data.clone());
                    }
                    crate::save::SavedTamedPetData::Companion2 { data, state, .. } => {
                        // 1C save: `state` is serde(skip) on CompanionData, so
                        // re-attach the command state carried alongside it.
                        let mut d = data.clone();
                        d.state = *state;
                        let _ = self.ecs.insert_one(id, d);
                    }
                    crate::save::SavedTamedPetData::Steed { data, .. } => {
                        // Legacy save (pre-Task 11): no cargo pack on the wire —
                        // spawn_mob attached a fresh wild HorseData; overwrite
                        // with the kept state (owner slot + AI state), packless.
                        let _ = self.ecs.insert_one(id, data.clone());
                    }
                    crate::save::SavedTamedPetData::Steed2 { data, pack, .. } => {
                        // Task 11 save: `pack` is serde(skip) on HorseData, so
                        // re-attach the cargo pack carried alongside it.
                        let mut d = data.clone();
                        d.pack = pack.clone();
                        let _ = self.ecs.insert_one(id, d);
                    }
                    crate::save::SavedTamedPetData::Authored { .. } => {
                        // #129 — re-tag as authored so it survives chunk unload
                        // (no `Scattered`) and is re-collected on the next save.
                        let _ = self.ecs.insert_one(id, crate::entity::Authored);
                    }
                }
            }

            // Heavy per-column work (lighting, water, gen-missing, meshing, mob
            // scatter) is deferred to `step_load`, driven by the queue built at
            // the end of `begin_load`. Spec 30: light isn't persisted, so the
            // queue re-runs the light pass per column before meshing it.
            log::info!(
                "Restored saved world '{}' ({} entities). Queuing chunk build.",
                self.world_name,
                self.ecs.len(),
            );
        } else if fresh_setup_wanted(load_ok, world_preloaded) {
            // ── Fresh world (no save found, or load failed). ──
            if let Err(ref e) = load_result
                && self.persists_locally()
                && (crate::save::world_exists(&self.world_name)
                    || crate::save::has_autosave(&self.world_name))
                {
                    log::warn!("Failed to load world: {e}. Generating new world.");
                }

            // Generate the small spawn-placement area (3×3 columns around origin,
            // covering find_surface_spawn's block-radius-6 spiral) so the player
            // can be placed on real ground. Lighting too, so the first mesh built
            // for these columns isn't dark. The rest of the render distance is
            // generated + meshed by `step_load` from the queue.
            for dx in -1..=1 {
                for dz in -1..=1 {
                    self.world.generate_column(dx, dz, &self.biome_gen);
                    crate::lighting::run_initial_pass_for_column(&mut self.world, dx, dz, &self.registry);
                    self.loaded_columns.insert((dx, dz));
                }
            }

            // Place the player on solid ground — never on tree canopy (#9) or in
            // water. Flat/Workshop worlds use the known fixed floor (avoids the
            // biome-height scan starting below y=79 and missing the flat floor).
            let (sx, spawn_y, sz) = if self.world.is_workshop || self.world.has_flat_floor() {
                (8i32, crate::workshop::WORKSHOP_FLOOR_Y + 1, 8i32)
            } else {
                find_surface_spawn(&self.world, &self.biome_gen, 0, 0)
            };
            self.players[0].player.pos = glam::Vec3::new(sx as f32 + 0.5, spawn_y as f32, sz as f32 + 0.5);
            self.players[0].player.velocity = glam::Vec3::ZERO;
            self.players[0].player.reset_fall();
            log::info!("Fresh world spawn at ({sx}, {spawn_y}, {sz}).");

            // New creative world starts with an EMPTY hotbar (owner 2026-07-02).
            // A creative builder picks their own blocks from the B search /
            // inventory, and the 36-slot inventory (hotbar included) persists
            // across save/load — so no starter kit is dumped on a fresh world.
            // The Workshop is the one exception: it needs its Bellows tool to
            // function, so it still receives that (brand-new world only — this
            // branch is never reached on re-entry, so it's never re-dumped).
            if self.is_creative && self.world.is_workshop {
                self.players[0].inventory.set_slot(
                    8,
                    Some(crate::item::ItemStack::new_material(crate::item::MaterialId::Bellows, 1)),
                );
            }

            // Test Lab starter kit — a tester should never be blocked grinding
            // before a mission. Generous building kit + the core tools + food +
            // the starter-hut Plan (so build-along / schematic missions are
            // immediately doable). Applies in Survival or Creative Test Labs.
            if self.world.world_type == "testlab" {
                use crate::block;
                use crate::crafting::{Tool, ToolMaterial, ToolType};
                use crate::item::{Item, ItemStack, MaterialId};
                let kit = [
                    block::OAK_PLANKS, block::OAK_LOG, block::STONE, block::COBBLESTONE,
                    block::DIRT, block::GLASS, block::SAND, block::CRAFTING_TABLE,
                    crate::rail::TRACK, block::CHEST,
                ];
                for (i, &id) in kit.iter().enumerate() {
                    self.players[0].inventory.set_slot(i, Some(ItemStack::new_block(id, 64)));
                }
                for tt in [ToolType::Pickaxe, ToolType::Axe, ToolType::Shovel, ToolType::Sword] {
                    let _ = self.players[0]
                        .inventory
                        .add_item(ItemStack::new_tool(Tool::new(tt, ToolMaterial::Iron)));
                }
                let _ = self.players[0]
                    .inventory
                    .add_item(ItemStack::new_material(MaterialId::Bread, 8));
                let _ = self.players[0].inventory.add_item(ItemStack {
                    item: Item::Plan(crate::satoshi::starter_hut_plan()),
                    count: 1,
                });
            }

            // Phase 6 — start every player in the remembered camera perspective.
            let default_mode = self.graphics.default_camera_mode;
            for slot in &mut self.players {
                slot.camera.mode = default_mode;
            }
        } else {
            // WASM preloaded resume: world + players were already restored by
            // the game_loop poll branch before `GameMode::Loading` was entered
            // — nothing to place, nothing to dump. The common tail below builds
            // the chunk queue around the RESTORED player position.
            log::info!(
                "Resuming preloaded world '{}' — skipping fresh-world setup.",
                self.world_name
            );
        }

        // ── Common tail (all paths) ─────────────────────────────────────────
        // Rebuild the derived render registry (official + wardrobe + override).
        self.reapply_overrides();

        let cs = CHUNK_SIZE as i32;
        // A joined session starts where the HOST placed us (JoinAccept spawn,
        // T2-9), not at this machine's own fresh-world spawn search.
        if let Some(spawn) = self.pending_join_spawn.take() {
            log::info!("Joined-world spawn from the host at {spawn:?}");
            self.place_player0_and_pregen(spawn);
        }

        // Apply a pending spawn-pref override (moved here from `stream_chunks`):
        // resolve the requested spawn, move the player, and pre-generate a small
        // area so they don't drop into void before the queue fills the rest in.
        if !matches!(self.pending_spawn_pref, crate::spawn_pref::SpawnPref::Default) {
            let saved = self.players[0].player.pos;
            let new_pos = crate::spawn_pref::resolve_spawn(
                self.pending_spawn_pref,
                self.biome_gen.seed,
                &self.biome_gen,
                saved,
            );
            if new_pos != saved {
                log::info!(
                    "Spawn override ({:?}): teleporting from {:?} to {:?}",
                    self.pending_spawn_pref, saved, new_pos,
                );
                self.place_player0_and_pregen(new_pos);
            }
            self.pending_spawn_pref = crate::spawn_pref::SpawnPref::Default;
        }

        // Build the load queue: every column within render distance around the
        // final player position ∪ all already-loaded (saved) columns, nearest
        // first. `step_load` drains it.
        let fpx = (self.players[0].player.pos.x.floor() as i32).div_euclid(cs);
        let fpz = (self.players[0].player.pos.z.floor() as i32).div_euclid(cs);
        let mut set: ahash::AHashSet<(i32, i32)> = self.loaded_columns.iter().copied().collect();
        for dx in -rd..=rd {
            for dz in -rd..=rd {
                set.insert((fpx + dx, fpz + dz));
            }
        }
        let mut cols: Vec<(i32, i32)> = set.into_iter().collect();
        cols.sort_by_key(|&(cx, cz)| {
            let dx = cx - fpx;
            let dz = cz - fpz;
            dx * dx + dz * dz
        });
        self.load_queue = cols.into();
    }

    /// Move player 0 to `pos` (at rest) and pre-generate + light the 5×5
    /// columns around it, so they don't drop into void before the load queue
    /// fills the rest in. Shared by the joined-world spawn and the spawn-pref
    /// override in `begin_load`.
    fn place_player0_and_pregen(&mut self, pos: glam::Vec3) {
        let cs = CHUNK_SIZE as i32;
        self.players[0].player.pos = pos;
        self.players[0].player.velocity = glam::Vec3::ZERO;
        self.players[0].player.reset_fall();
        let np_cx = (pos.x.floor() as i32).div_euclid(cs);
        let np_cz = (pos.z.floor() as i32).div_euclid(cs);
        for dx in -2..=2 {
            for dz in -2..=2 {
                let cx = np_cx + dx;
                let cz = np_cz + dz;
                if !self.loaded_columns.contains(&(cx, cz)) {
                    load_column_blocks(&mut self.world, cx, cz, &self.biome_gen, true);
                    crate::lighting::run_initial_pass_for_column(&mut self.world, cx, cz, &self.registry);
                    self.loaded_columns.insert((cx, cz));
                }
            }
        }
    }

    /// Drain up to `budget` columns from `load_queue`: generate (if missing),
    /// light, register water, scatter mobs, then mesh the column + re-mesh its
    /// already-loaded neighbours' boundaries (matching `stream_chunks`). Returns
    /// the remaining queue length.
    pub(crate) fn step_load(&mut self, budget: usize) -> usize {
        let mut done = 0;
        while done < budget {
            let Some((cx, cz)) = self.load_queue.pop_front() else { break };
            // Spec 02 §7.5 — restore an evicted column first (it wins over any
            // world-gen spill a neighbour left); generate only if neither.
            load_column_blocks(&mut self.world, cx, cz, &self.biome_gen, false);
            crate::lighting::run_initial_pass_for_column(&mut self.world, cx, cz, &self.registry);
            self.water.register_column_sources(cx, cz, &self.world);
            self.lava.register_column_sources(cx, cz, &self.world);
            self.fire.register_column_fires(cx, cz, &self.world, self.tick_counter);
            crate::entity::scatter_mobs_in_column(&mut self.ecs, cx, cz, &self.world, &self.biome_gen);
            for cy in 0..=MAX_CHUNK_Y {
                if self.world.has_chunk(cx, cy, cz) {
                    let meshes = build_chunk_meshes(cx, cy, cz, &self.world, &self.registry);
                    self.renderer.upload_chunk((cx, cy, cz), &meshes);
                }
            }
            for &(nx, nz) in &[(cx - 1, cz), (cx + 1, cz), (cx, cz - 1), (cx, cz + 1)] {
                if self.loaded_columns.contains(&(nx, nz)) {
                    for cy in 0..=MAX_CHUNK_Y {
                        if self.world.has_chunk(nx, cy, nz) {
                            let meshes = build_chunk_meshes(nx, cy, nz, &self.world, &self.registry);
                            self.renderer.upload_chunk((nx, cy, nz), &meshes);
                        }
                    }
                }
            }
            self.loaded_columns.insert((cx, cz));
            done += 1;
        }
        self.load_queue.len()
    }

}

/// Columns in `loaded` that are more than `unload_dist` chunks (Chebyshev,
/// per axis) from EVERY player column — the ones `stream_chunks` unloads.
pub(crate) fn columns_to_unload(
    loaded: &ahash::AHashSet<(i32, i32)>,
    player_cols: &[(i32, i32)],
    unload_dist: i32,
) -> Vec<(i32, i32)> {
    loaded
        .iter()
        .filter(|&&(cx, cz)| {
            player_cols.iter().all(|&(pcx, pcz)| {
                (cx - pcx).abs() > unload_dist || (cz - pcz).abs() > unload_dist
            })
        })
        .copied()
        .collect()
}

/// World side of unloading a column (Spec 02 §7.5): edited / saved columns go
/// to the evicted store, pristine ones are dropped. Renderer-free so the
/// stream wiring is unit-tested.
pub(crate) fn unload_column_blocks(world: &mut crate::world::World, cx: i32, cz: i32) {
    world.evict_column(cx, cz);
}

/// World side of (re)loading a column (Spec 02 §7.5): restore an evicted
/// column, else generate. `regen_present = true` is the `stream_chunks` /
/// repair behaviour (`generate_column` runs even over present chunks — it's
/// idempotent and heals void columns); `false` is `step_load`'s (a column that
/// already has chunks, e.g. from disk, is left alone). A restored column is
/// never generated over: that would refill dug-out all-air chunks. Returns
/// true if the column was restored.
pub(crate) fn load_column_blocks(
    world: &mut crate::world::World,
    cx: i32,
    cz: i32,
    biome_gen: &crate::biome::BiomeGenerator,
    regen_present: bool,
) -> bool {
    if world.restore_column(cx, cz) {
        return true;
    }
    if regen_present || !(0..=MAX_CHUNK_Y).any(|cy| world.has_chunk(cx, cy, cz)) {
        world.generate_column(cx, cz, biome_gen);
    }
    false
}

/// Should `begin_load` run the FRESH-world setup (spawn placement + the
/// one-time Workshop/Test-Lab kits)? Explicit entry intent, not an inference
/// from the synchronous load result: on WASM `save::load_world` is a stub that
/// ALWAYS fails, so a PWA resume — whose players/world the game_loop poll
/// branch has already restored from IndexedDB (`world_preloaded = true`) —
/// used to fall into FRESH and get its position reset + kit re-dumped on every
/// reload. Pure so the invariant is unit-tested.
/// Spec: docs/superpowers/specs/2026-06-22-begin-load-wasm-resume-position-fix-spec.md
pub(crate) fn fresh_setup_wanted(load_succeeded: bool, world_preloaded: bool) -> bool {
    !load_succeeded && !world_preloaded
}

/// Find a safe standing position near `(ox, oz)`: solid, walkable ground with
/// two clear cells above, never on tree canopy (leaves/logs) or in water.
/// Spirals outward from the origin column so a trunk or a lake sitting exactly
/// at the origin doesn't strand the player — we just step to the nearest clear
/// column. Returns `(x, feet_y, z)` in world coords. Owner inbox #9.
fn find_surface_spawn(
    world: &crate::world::World,
    biome_gen: &crate::biome::BiomeGenerator,
    ox: i32,
    oz: i32,
) -> (i32, i32, i32) {
    const MAX_RING: i32 = 6;
    for ring in 0..=MAX_RING {
        for dz in -ring..=ring {
            for dx in -ring..=ring {
                // Perimeter of this ring only (inner rings already tried).
                if dx.abs().max(dz.abs()) != ring {
                    continue;
                }
                let (x, z) = (ox + dx, oz + dz);
                if let Some(feet_y) = ground_with_headroom(world, biome_gen, x, z) {
                    return (x, feet_y, z);
                }
            }
        }
    }
    // Nothing clear within the spiral — drop in just above the generator's
    // terrain height at the origin and let physics settle.
    (ox, biome_gen.terrain_height(ox, oz).max(1) + 1, oz)
}

/// Scan a single column for solid ground with two air cells above. Returns the
/// feet Y (one above the ground block) or `None` if the column is water-topped
/// or roofed (e.g. a tree trunk directly above the ground).
fn ground_with_headroom(
    world: &crate::world::World,
    biome_gen: &crate::biome::BiomeGenerator,
    x: i32,
    z: i32,
) -> Option<i32> {
    use crate::block;
    // Start clear of any canopy, then walk down to the first solid ground.
    let mut y = biome_gen.terrain_height(x, z).max(0) + 24;
    while y > 1 {
        let b = world.get_block(x, y, z);
        let is_ground = b != block::AIR
            && b != block::WATER
            && !block::is_any_leaves(b)
            && !block::is_any_log_block(b);
        if is_ground {
            // Two strictly-air cells of head-room → a real standing spot.
            // Anything else above the ground (a trunk, a block) disqualifies
            // this column so we move to a neighbour instead of spawning inside it.
            if world.get_block(x, y + 1, z) == block::AIR
                && world.get_block(x, y + 2, z) == block::AIR
            {
                return Some(y + 1);
            }
            return None;
        }
        y -= 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biome::BiomeGenerator;
    use crate::block;

    /// Spec 02 §7.5 — drives the SAME world-side helpers `stream_chunks` calls
    /// (`columns_to_unload` → `unload_column_blocks`, then `load_column_blocks`
    /// on re-entry) for a player walking > rd+2 chunks away and back. Fails on
    /// the pre-fix wiring (drop on unload, regenerate on re-entry).
    #[test]
    fn stream_unload_and_reentry_keeps_an_edited_column() {
        let bg = BiomeGenerator::new(42);
        let mut world = crate::world::World::new();
        let mut loaded: ahash::AHashSet<(i32, i32)> = ahash::AHashSet::new();
        let rd = 2;
        // Player at column (0,0): load the render square.
        for cx in -rd..=rd {
            for cz in -rd..=rd {
                load_column_blocks(&mut world, cx, cz, &bg, true);
                loaded.insert((cx, cz));
            }
        }
        // Edit (1,1): place glass and dig a whole chunk (0..16 y) to air.
        world.place_player_block(20, 40, 20, block::GLASS);
        for x in 16..32 {
            for y in 16..32 {
                for z in 16..32 {
                    world.set_block(x, y, z, block::AIR);
                }
            }
        }
        // Walk > 200 blocks away: every column is out of range.
        let far = [(20, 0)];
        let gone = columns_to_unload(&loaded, &far, rd + 2);
        assert!(gone.contains(&(1, 1)));
        for &(cx, cz) in &gone {
            loaded.remove(&(cx, cz));
            unload_column_blocks(&mut world, cx, cz);
        }
        assert!(!world.has_chunk(1, 2, 1), "unloaded column leaves `chunks`");
        // Walk back: the stream load branch re-enters the column.
        let home = [(0, 0)];
        assert!(columns_to_unload(&loaded, &home, rd + 2).is_empty());
        for cx in -rd..=rd {
            for cz in -rd..=rd {
                if !loaded.contains(&(cx, cz)) {
                    load_column_blocks(&mut world, cx, cz, &bg, true);
                    loaded.insert((cx, cz));
                }
            }
        }
        assert_eq!(world.get_block(20, 40, 20), block::GLASS, "edit survives unload + re-entry");
        assert!(world.is_placed(20, 40, 20));
        assert!(
            (16..32).all(|y| world.get_block(20, y, 20) == block::AIR),
            "dug-out chunk must not be regenerated"
        );
    }

    /// `step_load`'s variant: a column already holding chunks (read from disk)
    /// is not generated over, but an evicted one is restored first.
    #[test]
    fn step_load_variant_restores_and_never_regens_present_columns() {
        let bg = BiomeGenerator::new(42);
        let mut world = crate::world::World::new();
        load_column_blocks(&mut world, 0, 0, &bg, false);
        world.set_block(3, 20, 3, block::AIR);
        world.set_block(3, 70, 3, block::GLASS);
        unload_column_blocks(&mut world, 0, 0);
        assert!(load_column_blocks(&mut world, 0, 0, &bg, false), "restored");
        assert_eq!(world.get_block(3, 70, 3), block::GLASS);
        assert_eq!(world.get_block(3, 20, 3), block::AIR);
    }

    #[test]
    fn fresh_setup_never_runs_on_a_preloaded_resume() {
        // begin_load's FRESH branch (spawn placement + one-time kits) must be
        // driven by explicit entry intent, not inferred from the synchronous
        // load result — on WASM `save::load_world` is a stub that ALWAYS fails,
        // so a PWA resume (players already restored by the game_loop poll
        // branch) fell into FRESH and had its position reset + Workshop/Test-Lab
        // kit re-dumped on every reload. Spec:
        // docs/superpowers/specs/2026-06-22-begin-load-wasm-resume-position-fix-spec.md
        //
        // Native new world: sync load fails, nothing preloaded → FRESH wanted.
        assert!(fresh_setup_wanted(false, false));
        // Native/WASM resume where the sync load succeeded → SAVE branch, no FRESH.
        assert!(!fresh_setup_wanted(true, false));
        // WASM resume: sync load fails but game_loop pre-restored → NO FRESH.
        assert!(!fresh_setup_wanted(false, true));
        // Degenerate (load succeeded AND preloaded) → still no FRESH.
        assert!(!fresh_setup_wanted(true, true));
    }

    #[test]
    fn spawn_avoids_tree_canopy_and_lands_on_ground() {
        // #9 — the old scan stopped at the first non-AIR walking down from
        // y=80, so on a tree column it landed on the leaves. find_surface_spawn
        // must skip leaves/logs/water, require air head-room, and step off the
        // origin column when a trunk sits there.
        let bg = BiomeGenerator::new(42);
        let mut world = crate::world::World::new();
        let base = bg.terrain_height(0, 0);
        // Grass platform around the origin (built relative to the generator's
        // own terrain height so the down-scan always reaches it).
        for x in -3..=3 {
            for z in -3..=3 {
                world.set_block(x, base, z, block::GRASS);
                world.set_block(x, base - 1, z, block::DIRT);
            }
        }
        // A tree at the exact origin column: trunk + canopy above the grass.
        world.set_block(0, base + 1, 0, block::OAK_LOG);
        world.set_block(0, base + 2, 0, block::OAK_LOG);
        world.set_block(0, base + 3, 0, block::OAK_LEAVES);
        world.set_block(0, base + 4, 0, block::OAK_LEAVES);

        let (sx, sy, sz) = find_surface_spawn(&world, &bg, 0, 0);
        // Feet rest on solid ground...
        let ground = world.get_block(sx, sy - 1, sz);
        assert!(
            ground == block::GRASS || ground == block::DIRT,
            "feet must rest on solid ground, got {ground} at ({sx},{sy},{sz})"
        );
        // ...the feet cell itself is clear (not inside a trunk or canopy)...
        assert_eq!(world.get_block(sx, sy, sz), block::AIR, "feet cell must be air");
        // ...and we stepped off the origin trunk column.
        assert!(!(sx == 0 && sz == 0), "must not spawn in the origin trunk column");
    }
}
