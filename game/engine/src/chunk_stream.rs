//! Chunk streaming: incremental loading/unloading and initial bulk load.

use crate::chunk::CHUNK_SIZE;
use crate::mesh::build_chunk_meshes;
use super::{MAX_CHUNK_Y, STREAM_BUDGET};

impl super::GameState {
    /// Generate and mesh chunks around all players (union of needed columns)
    /// and, on a world this host lends its server (D1), around every joiner's
    /// server body. First call does a bulk initial load; subsequent calls
    /// stream incrementally.
    pub(crate) fn stream_chunks(&mut self) {
        // Live render distance (Spec 39 — was the `RENDER_DISTANCE` const).
        let rd = self.graphics.render_distance;

        // ── Part G: Union chunk streaming — needed columns across all players ──
        let player_cols: Vec<(i32, i32)> =
            self.players.iter().map(|slot| column_of(slot.player.pos)).collect();

        // Use player 0 for initial load centre
        let (pcx0, pcz0) = player_cols[0];

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

        // The streaming decision (which columns to load this frame, nearest
        // player 0 first, and which to unload) is the pure
        // `plan_stream_step_for`, shared with the dedicated server's streamer
        // (`server_stream.rs`). SELF-HEAL: a column that's marked loaded but
        // has no bedrock floor (a "void column", the floor-grid-holes bug) is
        // re-queued too — see `is_void_column`.
        //
        // D1 review fix 1 — on a world this host lends its server, every
        // joiner's server body anchors the streamer too, at the server's sim
        // distance (`client_stream_anchors`): this world is the only one the
        // server simulates them on, so the host walking off must not unload
        // the ground under them. A joiner's own column orders like player 0's,
        // so it is never stuck behind the host's far ring. Every streamed
        // column is meshed, joiner-only ones too: a column already marked
        // loaded is never meshed later, so skipping would leave holes when the
        // host walks over (the far-joiner meshing cost is a known follow-up).
        let joiner_cols = self
            .hosted_server
            .as_ref()
            .map(|hs| hs.lent_joiner_columns())
            .unwrap_or_default();
        let anchors = client_stream_anchors(&player_cols, rd, &joiner_cols);
        let mut nearest_to = vec![(pcx0, pcz0)];
        nearest_to.extend_from_slice(&joiner_cols);
        let world = &self.world;
        let step = plan_stream_step_for(
            &anchors,
            &nearest_to,
            STREAM_BUDGET,
            &self.loaded_columns,
            |cx, cz| is_void_column(world, cx, cz),
        );
        if step.healed > 0 {
            log::warn!(
                "stream_chunks self-heal: re-generating {} void column(s) \
                 (marked loaded but unfloored) near player 0",
                step.healed
            );
        }

        for &(cx, cz) in &step.load {
            // Spec 02 §7.5 — an evicted (edited / saved) column comes back
            // from the store; only a never-kept column is (re)generated. This
            // also covers the void self-heal: it never regenerates over an
            // evicted column. The rest mirrors the save-load path (light,
            // fluid/fire rescan, mesh).
            self.column_sims().stream_in(cx, cz);

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

        // Unload columns that are outside ALL anchors' radius (+
        // `UNLOAD_HYSTERESIS`) — the players' render distance, and a lent
        // world's joiners' sim distance. Planned before the loads above, which
        // are all inside an anchor's radius, so never in this set.
        for &(cx, cz) in &step.unload {
            // Reclaims the column's scattered wildlife and evicts / drops its
            // blocks (Spec 02 §7.5) — see `ColumnSims::stream_out`.
            self.column_sims().stream_out(cx, cz);
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

    /// The world-side state a column stream-in / stream-out touches, borrowed
    /// from this client (see [`ColumnSims`]).
    fn column_sims(&mut self) -> ColumnSims<'_> {
        ColumnSims {
            world: &mut self.world,
            loaded: &mut self.loaded_columns,
            registry: &self.registry,
            biome_gen: &self.biome_gen,
            water: &mut self.water,
            lava: &mut self.lava,
            fire: &mut self.fire,
            ecs: &mut self.ecs,
            tick: self.tick_counter,
        }
    }

    /// Blocking world load — `begin_load` then drain the whole queue. Kept as
    /// the `stream_chunks` fallback; the live path drives `begin_load` +
    /// `step_load` incrementally from the `GameMode::Loading` state. Params are
    /// unused (the load centre is derived from the restored/placed player);
    /// retained so the call-site signature is unchanged.
    pub(crate) fn initial_load(&mut self, _pcx: i32, _pcz: i32) {
        if !self.begin_load() {
            return; // refused: back in the lobby with the notice
        }
        while !self.load_queue.is_empty() {
            self.step_load(usize::MAX);
        }
    }

    /// One-shot world-load setup: read the save (or mark fresh), restore players,
    /// place the spawn, apply any spawn-pref override, rebuild overrides, then
    /// build `load_queue` (every column within render distance ∪ saved columns,
    /// nearest-first). The heavy per-column gen+light+mesh is drained by
    /// `step_load`, so the loading screen animates instead of freezing.
    ///
    /// Returns `false` when the world is on disk but failed to load: it is NOT
    /// replaced by a fresh world — the player is back in the lobby with "This
    /// world couldn't be opened: <why>. Nothing was changed.", the world was never
    /// marked live, and nothing was written (Spec 02 §8.4, `world_open`).
    pub(crate) fn begin_load(&mut self) -> bool {
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
        // Try loading a saved world first (the autosave first, for crash
        // recovery, falling back to the last manual save if it fails).
        // A joined session never reads a local save: an old "remote_game"
        // folder (written by builds before 2026-09-28) would otherwise restore
        // another server's inventory, position and chunks into this one.
        let load_result = if !self.persists_locally() {
            None
        } else {
            match crate::world_open::open_world(
                &self.world_name,
                &mut self.world,
                crate::world_open::AutosavePolicy::Prefer,
            ) {
                Ok(crate::world_open::OpenedWorld::New) => None,
                Ok(crate::world_open::OpenedWorld::Loaded { save, chunks, from }) => {
                    // An autosave the world opened from is KEPT until a save lands
                    // (`world_exit::should_clear_autosave`, `pause_save`): with a
                    // damaged world.dat it is the only good copy, and deleting it
                    // here lost the world to a crash before the next save
                    // (review 2026-10-06). The next autosave overwrites it anyway.
                    // Until a save lands, "Quit without saving" keeps it too
                    // (`world_exit::SessionSaves`).
                    self.session_saves =
                        crate::world_exit::SessionSaves::opened(from.opened_from_autosave());
                    if let Some(note) = from.player_note() {
                        self.toast = Some((
                            note,
                            web_time::Instant::now() + std::time::Duration::from_secs(10),
                        ));
                    }
                    Some((*save, chunks))
                }
                Err(why) => {
                    // Never a fresh world over a world that is there but failed
                    // to load: nothing is generated, the world is never marked
                    // live, nothing is written.
                    log::error!("world '{}' refused: {why}", self.world_name);
                    self.leave_world_with_notice(
                        crate::world_exit::SaveChoice::Abandon,
                        crate::save_format::unopenable_message(&why),
                    );
                    return false;
                }
            }
        };
        // A world that opened (or is genuinely new) gets its Proof-of-Play secret
        // persisted now — deferred from the lobby so a refused world is never
        // written (`GameState::apply_world_seed`) — or, when the open had to
        // rebuild a torn meta, the session adopts the secret it persisted.
        #[cfg(not(target_arch = "wasm32"))]
        if self.persists_locally() {
            self.persist_pop_secret_if_missing();
        }

        // Save path = resuming an existing world; the `else if` below is a
        // fresh world (no save / load failed AND nothing preloaded), which is
        // where the one-time kits + spawn placement live so they never re-apply
        // on re-entry (2026-06-16 playtest: Axolittle "loads of blocks you
        // havnt gotten" + "spawn in flying" on every join). A WASM resume takes
        // NEITHER branch: the sync load always fails there, but the game_loop
        // poll branch already restored world + players (`world_preloaded`), so
        // running FRESH would clobber the restored position + kit slots.
        let load_ok = load_result.is_some();
        if let Some((save_data, _chunk_count)) = load_result {
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
            // ── Fresh world: nothing saved here yet, or a joined session. A world
            // that is saved here but failed to load never reaches this branch
            // (refused above). ──
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
            let spawn = world_spawn_point(&self.world, &self.biome_gen);
            self.players[0].player.pos = spawn;
            self.players[0].player.velocity = glam::Vec3::ZERO;
            self.players[0].player.reset_fall();
            log::info!("Fresh world spawn at {spawn}.");

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
            // The server's body starts exactly there (Spec 04 §5.3): no local
            // spawn preference may move ours away from it.
            self.pending_spawn_pref = crate::spawn_pref::SpawnPref::Default;
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
        true
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

// ── Column-streaming policy (shared: client + dedicated server) ─────────────
//
// One decision, two callers: the client's `GameState::stream_chunks` (anchors =
// its local players, radius = render distance, `STREAM_BUDGET` per frame) and
// the dedicated server's `GameServer::stream_columns` (`server_stream.rs`;
// anchors = every connected player + the world spawn, radius = `--sim-distance`,
// `SERVER_STREAM_BUDGET` per tick). Spec 01 §4.1.2.

/// Extra columns (Chebyshev) a loaded column may sit beyond the streaming
/// radius before it unloads, so a player pacing along a column border doesn't
/// thrash load/unload.
pub(crate) const UNLOAD_HYSTERESIS: i32 = 2;

/// The column `(cx, cz)` holding world position `pos`.
pub(crate) fn column_of(pos: glam::Vec3) -> (i32, i32) {
    let cs = CHUNK_SIZE as i32;
    (
        (pos.x.floor() as i32).div_euclid(cs),
        (pos.z.floor() as i32).div_euclid(cs),
    )
}

/// Is this loaded column a "void column" — normal terrain with no bedrock floor
/// (the floor-grid-holes bug)? Normal terrain worlds floor every column with
/// bedrock at y=0 (`biome_block_at` is unconditional there); flat/Workshop
/// worlds floor at other Ys, so y=0 bedrock isn't their "is this generated?"
/// signal and they never report void. A streamer re-queues a void column: the
/// plain `!loaded` guard alone never revisits it, so a generate/bookkeeping
/// divergence (e.g. a column marked loaded because a save held only a stray
/// chunk of it) would stay a permanent hole. Re-running `generate_column` is
/// idempotent (it skips already-filled chunks).
pub(crate) fn is_void_column(world: &crate::world::World, cx: i32, cz: i32) -> bool {
    let cs = CHUNK_SIZE as i32;
    !world.is_workshop
        && !world.has_flat_floor()
        && world.get_block(cx * cs + 8, 0, cz * cs + 8) != crate::block::BEDROCK
}

/// One streaming step's decision (see [`plan_stream_step`]).
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct StreamStep {
    /// Columns to (re)load now, nearest first, at most `budget` of them.
    pub load: Vec<(i32, i32)>,
    /// How many columns wanted loading before the budget cut (`>= load.len()`).
    pub pending: usize,
    /// How many of those were loaded-but-void columns being re-generated.
    pub healed: usize,
    /// Loaded columns outside every anchor's `radius + UNLOAD_HYSTERESIS`.
    pub unload: Vec<(i32, i32)>,
}

/// A streaming anchor: a column, and how far round it (Chebyshev, in
/// columns) is kept loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StreamAnchor {
    pub col: (i32, i32),
    pub radius: i32,
}

/// How far round each joiner's server body a LENDING host's client streamer
/// keeps its world loaded (D1 review fix 1): the dedicated server's default
/// sim distance, so a joiner on a lent world is simulated over the same ground
/// a dedicated server would give them — physics, reach, mob AI and spawning
/// round them all need loaded columns, and the server's own copy that used to
/// hold them is gone.
pub(crate) const LENT_JOINER_SIM_DISTANCE: i32 = crate::server_stream::DEFAULT_SIM_DISTANCE;

/// The anchors a client's streamer keeps loaded round: every local player's
/// column at the render distance and — on a world it lends its server (D1;
/// `joiner_cols` is empty otherwise, see `HostedServer::lent_joiner_columns`)
/// — every joiner's server-body column at [`LENT_JOINER_SIM_DISTANCE`]. The
/// lent world IS the server's, so a column only the host's players were
/// keeping would otherwise unload under a joiner the moment the host walked
/// away: the body falls through, its edits are refused as Unloaded.
pub(crate) fn client_stream_anchors(
    local_cols: &[(i32, i32)],
    render_distance: i32,
    joiner_cols: &[(i32, i32)],
) -> Vec<StreamAnchor> {
    local_cols
        .iter()
        .map(|&col| StreamAnchor { col, radius: render_distance })
        .chain(
            joiner_cols
                .iter()
                .map(|&col| StreamAnchor { col, radius: LENT_JOINER_SIM_DISTANCE }),
        )
        .collect()
}

/// [`plan_stream_step_for`] with one `radius` for every anchor (the dedicated
/// server's streamer).
// Reached only from native hosting / the dedicated server.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn plan_stream_step(
    anchors: &[(i32, i32)],
    nearest_to: &[(i32, i32)],
    radius: i32,
    budget: usize,
    loaded: &ahash::AHashSet<(i32, i32)>,
    needs_reload: impl FnMut(i32, i32) -> bool,
) -> StreamStep {
    let anchors: Vec<StreamAnchor> =
        anchors.iter().map(|&col| StreamAnchor { col, radius }).collect();
    plan_stream_step_for(&anchors, nearest_to, budget, loaded, needs_reload)
}

/// The column-streaming decision, pure.
///
/// Needed = every column within its anchor's `radius` (Chebyshev) of any
/// `anchors` column. Wanted = needed columns not in `loaded`, plus loaded ones
/// `needs_reload` flags (the void self-heal). Wanted columns are ordered by
/// squared distance to the NEAREST `nearest_to` column, ties broken by
/// `(cx, cz)` so the order is deterministic, and the first `budget` are
/// returned in `load`. `unload` is every loaded column beyond EVERY anchor's
/// own `radius + UNLOAD_HYSTERESIS` ([`columns_outside_anchors`]).
///
/// `nearest_to` is separate from `anchors` on purpose: the client orders by
/// player 0 (and, lending, its joiners — see `stream_chunks`), so its
/// split-screen players load after player 0's nearer columns, as before; the
/// server orders by every anchor, so each player's own column (distance 0)
/// always loads first.
pub(crate) fn plan_stream_step_for(
    anchors: &[StreamAnchor],
    nearest_to: &[(i32, i32)],
    budget: usize,
    loaded: &ahash::AHashSet<(i32, i32)>,
    mut needs_reload: impl FnMut(i32, i32) -> bool,
) -> StreamStep {
    let mut needed = ahash::AHashSet::new();
    for &StreamAnchor { col: (ax, az), radius } in anchors {
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                needed.insert((ax + dx, az + dz));
            }
        }
    }
    let mut wanted: Vec<(i64, i32, i32)> = Vec::new();
    let mut healed = 0usize;
    for &(cx, cz) in &needed {
        let is_loaded = loaded.contains(&(cx, cz));
        let void = is_loaded && needs_reload(cx, cz);
        if is_loaded && !void {
            continue;
        }
        healed += usize::from(void);
        // i64: a far teleport puts columns ~10^5 apart, whose square overflows i32.
        let d = nearest_to
            .iter()
            .map(|&(px, pz)| {
                let (dx, dz) = (i64::from(cx - px), i64::from(cz - pz));
                dx * dx + dz * dz
            })
            .min()
            .unwrap_or(0);
        wanted.push((d, cx, cz));
    }
    let pending = wanted.len();
    wanted.sort_unstable();
    wanted.truncate(budget);
    StreamStep {
        load: wanted.into_iter().map(|(_, cx, cz)| (cx, cz)).collect(),
        pending,
        healed,
        unload: columns_outside_anchors(loaded, anchors, UNLOAD_HYSTERESIS),
    }
}

/// The world-side state a column stream-in / stream-out touches, borrowed
/// from whichever side streams it (the client's `GameState` or the dedicated
/// `GameServer`), so the per-column steps can't drift between them (the
/// `block_machines::MachineCtx` pattern). Rendering stays with the caller.
pub(crate) struct ColumnSims<'a> {
    pub world: &'a mut crate::world::World,
    pub loaded: &'a mut ahash::AHashSet<(i32, i32)>,
    pub registry: &'a crate::block::BlockRegistry,
    pub biome_gen: &'a crate::biome::BiomeGenerator,
    pub water: &'a mut crate::water::WaterSystem,
    pub lava: &'a mut crate::lava::LavaSystem,
    pub fire: &'a mut crate::fire::FireSystem,
    pub ecs: &'a mut hecs::World,
    /// The caller's monotonic tick counter (fire timestamps).
    pub tick: u64,
}

impl ColumnSims<'_> {
    /// Stream a column in: [`Self::load_terrain`], then scatter its wildlife.
    pub(crate) fn stream_in(&mut self, cx: i32, cz: i32) {
        self.load_terrain(cx, cz);
        crate::entity::scatter_mobs_in_column(self.ecs, cx, cz, self.world, self.biome_gen);
    }

    /// The terrain half of [`Self::stream_in`]: restore the column from the
    /// evicted store, else generate it (Spec 02 §7.5 — `load_column_blocks`
    /// with `regen_present`, which also heals a void column); run the light
    /// pass (Spec 30: light isn't persisted, and mob spawning + crop growth
    /// read it); register its water, lava and fire; mark it loaded. A LAN /
    /// online host's server loads the columns round its joiners with this
    /// alone (`GameServer::ensure_column_loaded`): it scatters no wildlife on
    /// the server, as its `initial_load` region has none either.
    pub(crate) fn load_terrain(&mut self, cx: i32, cz: i32) {
        load_column_blocks(self.world, cx, cz, self.biome_gen, true);
        crate::lighting::run_initial_pass_for_column(self.world, cx, cz, self.registry);
        self.water.register_column_sources(cx, cz, self.world);
        self.lava.register_column_sources(cx, cz, self.world);
        self.fire.register_column_fires(cx, cz, self.world, self.tick);
        self.loaded.insert((cx, cz));
    }

    /// Stream a column out: unmark it, reclaim its scattered wildlife so
    /// re-entry re-scatters a fresh set rather than piling onto the old
    /// (engine audit B — tamed pets / villagers / golems aren't `Scattered`, so
    /// they survive, frozen) and its night-spawned hostiles (never saved),
    /// forget its water and lava sources (stream-in re-registers them), and
    /// evict its blocks: edited / saved columns go to the evicted store (and
    /// are still written by every save path); pristine world-gen is dropped
    /// (Spec 02 §7.5).
    pub(crate) fn stream_out(&mut self, cx: i32, cz: i32) {
        self.loaded.remove(&(cx, cz));
        crate::entity::despawn_mobs_in_column(self.ecs, cx, cz);
        self.water.forget_column(cx, cz);
        self.lava.forget_column(cx, cz);
        unload_column_blocks(self.world, cx, cz);
    }
}

/// Phase B1 review — does a server block change for block `(x, z)` land in a
/// column this client holds? A loaded column, or an evicted one (the write goes
/// through to the evicted store, so a later restore is current). Anywhere else
/// `World::set_block` conjures a stray chunk that this client's own generation
/// later skips, leaving a 16³ hole (and a LAN host's save keeps it).
pub(crate) fn remote_change_is_loaded(
    loaded: &ahash::AHashSet<(i32, i32)>,
    world: &crate::world::World,
    x: i32,
    z: i32,
) -> bool {
    let cs = CHUNK_SIZE as i32;
    loaded.contains(&(x.div_euclid(cs), z.div_euclid(cs))) || world.is_evicted_at(x, z)
}

/// Phase B1 review — keep a server block change for a column this client has
/// not loaded: generate the column, apply the change, evict it (Spec 02 §7.5:
/// an edited column is kept in the evicted store and written by every save; a
/// change that matches world-gen keeps nothing). For the LAN host, whose world
/// is the save of record: a joiner's edit near the spawn while the host is
/// away must not be lost. Joiners drop such changes instead (Spec 04 §4.1).
pub(crate) fn apply_remote_change_to_unloaded_column(
    world: &mut crate::world::World,
    biome_gen: &crate::biome::BiomeGenerator,
    bc: &crate::protocol::BlockChange,
) {
    let cs = CHUNK_SIZE as i32;
    let (cx, cz) = (bc.x.div_euclid(cs), bc.z.div_euclid(cs));
    load_column_blocks(world, cx, cz, biome_gen, true);
    world.apply_remote_block_change(bc);
    unload_column_blocks(world, cx, cz);
}

/// Columns in `loaded` that are more than their anchor's `radius + slack`
/// (Chebyshev, per axis) from EVERY anchor — the ones a streamer unloads.
pub(crate) fn columns_outside_anchors(
    loaded: &ahash::AHashSet<(i32, i32)>,
    anchors: &[StreamAnchor],
    slack: i32,
) -> Vec<(i32, i32)> {
    loaded
        .iter()
        .filter(|&&(cx, cz)| {
            anchors.iter().all(|&StreamAnchor { col: (ax, az), radius }| {
                let dist = radius + slack;
                (cx - ax).abs() > dist || (cz - az).abs() > dist
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

/// A world's spawn point: where a fresh world places its player, and where a
/// dedicated server starts a joiner (`GameServer::world_spawn`) — one
/// rule, so both agree. Flat/Workshop worlds use the known fixed floor (the
/// biome-height scan would start below y=79 and miss it); every other world
/// the nearest clear ground to the origin. The columns around the origin must
/// be generated first.
pub(crate) fn world_spawn_point(
    world: &crate::world::World,
    biome_gen: &crate::biome::BiomeGenerator,
) -> glam::Vec3 {
    let (sx, sy, sz) = if world.is_workshop || world.has_flat_floor() {
        (8, crate::workshop::WORKSHOP_FLOOR_Y + 1, 8)
    } else {
        find_surface_spawn(world, biome_gen, 0, 0)
    };
    glam::Vec3::new(sx as f32 + 0.5, sy as f32, sz as f32 + 0.5)
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
    /// (`columns_outside_anchors` → `unload_column_blocks`, then `load_column_blocks`
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
        let far = [StreamAnchor { col: (20, 0), radius: rd }];
        let gone = columns_outside_anchors(&loaded, &far, UNLOAD_HYSTERESIS);
        assert!(gone.contains(&(1, 1)));
        for &(cx, cz) in &gone {
            loaded.remove(&(cx, cz));
            unload_column_blocks(&mut world, cx, cz);
        }
        assert!(!world.has_chunk(1, 2, 1), "unloaded column leaves `chunks`");
        // Walk back: the stream load branch re-enters the column.
        let home = [StreamAnchor { col: (0, 0), radius: rd }];
        assert!(columns_outside_anchors(&loaded, &home, UNLOAD_HYSTERESIS).is_empty());
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

    fn set_of(cols: &[(i32, i32)]) -> ahash::AHashSet<(i32, i32)> {
        cols.iter().copied().collect()
    }

    /// Phase B1 — the shared streaming decision: nearest first, capped at the
    /// budget, deterministic on ties, `pending` counts the whole backlog.
    #[test]
    fn plan_loads_nearest_first_within_budget() {
        let loaded = set_of(&[(0, 0)]);
        let step = plan_stream_step(&[(0, 0)], &[(0, 0)], 1, 3, &loaded, |_, _| false);
        assert_eq!(step.pending, 8, "the 3x3 square minus the loaded centre");
        // The 4 edge neighbours (d²=1) beat the corners (d²=2); ties by (cx, cz).
        assert_eq!(step.load, vec![(-1, 0), (0, -1), (0, 1)]);
        assert_eq!(step.healed, 0);
        assert!(step.unload.is_empty());

        let all = plan_stream_step(&[(0, 0)], &[(0, 0)], 1, usize::MAX, &loaded, |_, _| false);
        assert_eq!(all.load.len(), 8);
        let ds: Vec<i32> = all.load.iter().map(|&(x, z)| x * x + z * z).collect();
        assert!(ds.windows(2).all(|w| w[0] <= w[1]), "nearest first: {ds:?}");
    }

    /// With every anchor as a distance reference, each anchor's own column
    /// (distance 0) loads before anything else — what keeps a server-simulated
    /// player standing on ground when several players need columns at once.
    #[test]
    fn plan_orders_by_the_nearest_reference_column() {
        let anchors = [(0, 0), (40, 0)];
        let step = plan_stream_step(&anchors, &anchors, 2, 2, &set_of(&[]), |_, _| false);
        assert_eq!(step.load, vec![(0, 0), (40, 0)], "both own columns first");
        // Ordering by player 0 alone (the client) puts the far anchor last.
        let p0 = plan_stream_step(&anchors, &[(0, 0)], 2, 50, &set_of(&[]), |_, _| false);
        assert_eq!(p0.pending, 50, "two disjoint 5x5 squares");
        assert_eq!(p0.load.last().map(|&(x, _)| x >= 38), Some(true));
    }

    /// The void self-heal: a loaded column the predicate flags is re-queued
    /// (and counted); an unflagged loaded one is left alone.
    #[test]
    fn plan_requeues_flagged_loaded_columns_as_healed() {
        let loaded = set_of(&[(0, 0), (1, 0)]);
        let step = plan_stream_step(&[(0, 0)], &[(0, 0)], 0, 4, &loaded, |cx, _| cx == 0);
        assert_eq!(step.load, vec![(0, 0)]);
        assert_eq!(step.healed, 1);
        assert_eq!(step.pending, 1);
    }

    /// Unload uses the radius plus `UNLOAD_HYSTERESIS`, against every anchor.
    #[test]
    fn plan_unloads_only_beyond_radius_plus_hysteresis_of_every_anchor() {
        let r = 3;
        let edge = r + UNLOAD_HYSTERESIS;
        let loaded = set_of(&[(edge, 0), (edge + 1, 0), (0, -(edge + 1)), (100, 100)]);
        let step = plan_stream_step(&[(0, 0), (100, 100)], &[(0, 0)], r, 0, &loaded, |_, _| false);
        let mut unload = step.unload.clone();
        unload.sort_unstable();
        assert_eq!(unload, vec![(0, -(edge + 1)), (edge + 1, 0)]);
        assert!(step.load.is_empty(), "a zero budget loads nothing");
    }

    /// D1 review fix 1 — each anchor keeps its OWN radius: a far anchor with a
    /// smaller radius loads and keeps only its own square, and a column inside
    /// any anchor's radius + hysteresis is never unloaded.
    #[test]
    fn plan_keeps_each_anchor_at_its_own_radius() {
        let anchors = [
            StreamAnchor { col: (0, 0), radius: 3 },
            StreamAnchor { col: (40, 0), radius: 1 },
        ];
        let all = set_of(&[]);
        let step = plan_stream_step_for(&anchors, &[(0, 0), (40, 0)], usize::MAX, &all, |_, _| false);
        assert_eq!(step.pending, 7 * 7 + 3 * 3, "a 7x7 and a 3x3 square");
        assert!(step.load.contains(&(41, 1)) && !step.load.contains(&(42, 0)));
        let small_edge = 1 + UNLOAD_HYSTERESIS;
        let loaded = set_of(&[(40 + small_edge, 0), (40 + small_edge + 1, 0), (3 + UNLOAD_HYSTERESIS, 0)]);
        let step = plan_stream_step_for(&anchors, &[(0, 0)], 0, &loaded, |_, _| false);
        assert_eq!(step.unload, vec![(40 + small_edge + 1, 0)]);
    }

    /// D1 review fix 1 — a LENDING host's streamer keeps a joiner's column
    /// however far the host walks: every joiner's server body is an anchor at
    /// the server's sim distance, beside the local players at the render
    /// distance. Without it (the pre-fix client, local players only) the
    /// joiner's column unloads under them.
    #[test]
    fn a_lending_hosts_stream_anchors_keep_a_far_joiners_column() {
        let rd = 4;
        let joiner = (0, 0);
        let host = (rd + UNLOAD_HYSTERESIS + 1, 0);
        let loaded = set_of(&[joiner, (1, 0), host]);
        // Negative control: the host's own anchors alone drop the joiner's column.
        let host_only = client_stream_anchors(&[host], rd, &[]);
        let step = plan_stream_step_for(&host_only, &[host], 0, &loaded, |_, _| false);
        assert!(step.unload.contains(&joiner), "pre-fix: the joiner's column unloads");

        let anchors = client_stream_anchors(&[host], rd, &[joiner]);
        assert!(anchors.contains(&StreamAnchor { col: host, radius: rd }));
        assert!(anchors.contains(&StreamAnchor { col: joiner, radius: LENT_JOINER_SIM_DISTANCE }));
        let step = plan_stream_step_for(&anchors, &[host, joiner], 0, &loaded, |_, _| false);
        assert!(step.unload.is_empty(), "nothing near the joiner unloads: {:?}", step.unload);
        // And a joiner standing in an unloaded column has it loaded first.
        let step = plan_stream_step_for(&anchors, &[host, joiner], 1, &set_of(&[host]), |_, _| false);
        assert_eq!(step.load, vec![joiner]);
    }

    /// `ColumnSims::stream_in` restores an evicted column (never generating
    /// over it), lights it and marks it loaded; `stream_out` evicts an edited
    /// column and drops a pristine one.
    #[test]
    fn column_sims_stream_in_and_out_round_trip_an_edited_column() {
        let bg = BiomeGenerator::new(42);
        let registry = crate::block::BlockRegistry::new();
        let mut world = crate::world::World::new();
        let mut loaded = ahash::AHashSet::new();
        let mut water = crate::water::WaterSystem::new();
        let mut lava = crate::lava::LavaSystem::new();
        let mut fire = crate::fire::FireSystem::new();
        let mut ecs = hecs::World::new();
        let mut sims = ColumnSims {
            world: &mut world,
            loaded: &mut loaded,
            registry: &registry,
            biome_gen: &bg,
            water: &mut water,
            lava: &mut lava,
            fire: &mut fire,
            ecs: &mut ecs,
            tick: 0,
        };
        sims.stream_in(0, 0);
        sims.stream_in(1, 0);
        assert!(sims.loaded.contains(&(0, 0)) && sims.loaded.contains(&(1, 0)));
        assert_eq!(sims.world.get_block(8, 0, 8), block::BEDROCK, "generated");
        assert!(!is_void_column(sims.world, 0, 0));
        // The light pass ran: the highest air cell inside a present chunk
        // (fresh chunks start dark) is open to the sky.
        let lit = (0..96).rev().find(|&y| {
            sims.world.has_chunk(0, y / 16, 0) && sims.world.get_block(3, y, 3) == block::AIR
        });
        let y = lit.expect("an air cell inside a generated chunk");
        assert_eq!(sims.world.sky_light_at(3, y, 3), 15, "light pass ran (y {y})");
        sims.world.place_player_block(3, 90, 3, block::GLASS);

        sims.stream_out(0, 0);
        sims.stream_out(1, 0);
        assert!(sims.loaded.is_empty());
        assert!(sims.world.is_column_evicted(0, 0), "edited column kept");
        assert!(!sims.world.is_column_evicted(1, 0), "pristine column dropped");
        assert!(!sims.world.has_chunk(1, 0, 0));

        sims.stream_in(0, 0);
        assert!(!sims.world.is_column_evicted(0, 0), "restored to the live chunks");
        assert_eq!(sims.world.get_block(3, 90, 3), block::GLASS, "the edit survives");
    }

    /// Owns everything a [`ColumnSims`] borrows, for the B1-review tests.
    struct SimsFixture {
        bg: BiomeGenerator,
        registry: crate::block::BlockRegistry,
        world: crate::world::World,
        loaded: ahash::AHashSet<(i32, i32)>,
        water: crate::water::WaterSystem,
        lava: crate::lava::LavaSystem,
        fire: crate::fire::FireSystem,
        ecs: hecs::World,
    }

    impl SimsFixture {
        fn new() -> Self {
            Self {
                bg: BiomeGenerator::new(42),
                registry: crate::block::BlockRegistry::new(),
                world: crate::world::World::new(),
                loaded: ahash::AHashSet::new(),
                water: crate::water::WaterSystem::new(),
                lava: crate::lava::LavaSystem::new(),
                fire: crate::fire::FireSystem::new(),
                ecs: hecs::World::new(),
            }
        }

        fn sims(&mut self) -> ColumnSims<'_> {
            ColumnSims {
                world: &mut self.world,
                loaded: &mut self.loaded,
                registry: &self.registry,
                biome_gen: &self.bg,
                water: &mut self.water,
                lava: &mut self.lava,
                fire: &mut self.fire,
                ecs: &mut self.ecs,
                tick: 0,
            }
        }
    }

    /// Phase B1 review (MED-HIGH) — a fluid on a loaded column's edge never
    /// flows into a column that was never loaded. `set_block` there conjured a
    /// chunk, `generate_column` skips a non-empty chunk, so when the column
    /// streamed in that slice had no bedrock or stone (and a save kept it).
    #[test]
    fn fluids_never_flow_into_a_never_loaded_column() {
        let mut fx = SimsFixture::new();
        fx.sims().stream_in(0, 0);
        // A lava and a water source on (0, 0)'s east edge, floored, beside
        // the never-loaded column (1, 0).
        let lava_at = (15, 6, 4);
        let water_at = (15, 6, 12);
        for (p, fluid) in [(lava_at, block::LAVA), (water_at, block::WATER)] {
            fx.world.set_block(p.0, p.1 - 1, p.2, block::STONE);
            fx.world.set_block(p.0, p.1, p.2, fluid);
        }
        fx.lava.add_source(lava_at.0, lava_at.1, lava_at.2);
        fx.water.add_source(water_at.0, water_at.1, water_at.2);
        for _ in 0..60 {
            fx.water.tick_spread(&mut fx.world);
            fx.lava.tick_spread(&mut fx.world);
        }
        // (Block light from cave lava may leave empty, light-only chunks
        // there; `generate_column` refills those. No block was written.)
        assert!(
            (0..=MAX_CHUNK_Y).all(|cy| fx.world.get_chunk(1, cy, 0).is_none_or(|c| c.is_empty())),
            "no block was written into the never-loaded column"
        );

        fx.sims().stream_in(1, 0);
        for x in 16..32 {
            for z in 0..16 {
                assert_eq!(fx.world.get_block(x, 0, z), block::BEDROCK, "bedrock at ({x}, 0, {z})");
            }
        }
    }

    /// Phase B1 review (LOW-MED) — streaming a column out forgets its water
    /// and lava sources (water registers every water block, so an ocean world
    /// grew the sets without bound); streaming it back in re-registers them.
    #[test]
    fn stream_out_forgets_a_columns_fluid_sources_and_stream_in_restores_them() {
        let mut fx = SimsFixture::new();
        fx.sims().stream_in(0, 0);
        fx.sims().stream_in(1, 0);
        let water_at = (3, 90, 3);
        let lava_at = (5, 90, 5);
        let kept_at = (20, 90, 3); // column (1, 0) stays loaded
        fx.world.place_player_block(water_at.0, water_at.1, water_at.2, block::WATER);
        fx.world.place_player_block(lava_at.0, lava_at.1, lava_at.2, block::LAVA);
        fx.world.place_player_block(kept_at.0, kept_at.1, kept_at.2, block::WATER);
        fx.water.add_source(water_at.0, water_at.1, water_at.2);
        fx.lava.add_source(lava_at.0, lava_at.1, lava_at.2);
        fx.water.add_source(kept_at.0, kept_at.1, kept_at.2);

        fx.sims().stream_out(0, 0);
        assert!(!fx.water.is_source(water_at.0, water_at.1, water_at.2), "water source forgotten");
        assert!(!fx.lava.is_source(lava_at.0, lava_at.1, lava_at.2), "lava source forgotten");
        assert!(fx.water.is_source(kept_at.0, kept_at.1, kept_at.2), "a loaded column's source stays");

        fx.sims().stream_in(0, 0);
        assert!(fx.water.is_source(water_at.0, water_at.1, water_at.2), "re-registered on stream-in");
        assert!(fx.lava.is_source(lava_at.0, lava_at.1, lava_at.2), "re-registered on stream-in");
    }

    /// Forgetting an unloaded column's sources must not drain the flow they
    /// feed across the border: a retract next door that walks into the
    /// unloaded column assumes the flow there is fed (it cannot see).
    #[test]
    fn a_retract_does_not_drain_flow_fed_from_an_unloaded_column() {
        let mut fx = SimsFixture::new();
        fx.sims().stream_in(0, 0);
        fx.sims().stream_in(1, 0);
        for x in 10..=17 {
            for z in 2..=7 {
                fx.world.place_player_block(x, 89, z, block::STONE);
            }
        }
        let fed_from = (16, 90, 4); // column (1, 0)
        let local = (14, 90, 6); // column (0, 0)
        for p in [fed_from, local] {
            fx.world.place_player_block(p.0, p.1, p.2, block::WATER);
            fx.water.add_source(p.0, p.1, p.2);
        }
        for _ in 0..80 {
            fx.water.tick_spread(&mut fx.world);
        }
        assert_eq!(fx.world.get_block(14, 90, 5), block::WATER, "fixture: the floor is flooded");

        fx.sims().stream_out(1, 0); // edited: evicted, its source forgotten
        fx.world.set_block(local.0, local.1, local.2, block::AIR);
        fx.water.remove_source(local.0, local.1, local.2);
        for _ in 0..10 {
            fx.water.tick_retract(&mut fx.world);
        }
        assert_eq!(
            fx.world.get_block(14, 90, 5),
            block::WATER,
            "flow within reach of the unloaded column's source is kept"
        );
    }

    /// Phase B1 review (LOW) — a night-spawned hostile in a column that
    /// streams out is despawned (it is never saved, and frozen there it held a
    /// slot of the hostile cap forever); a hideout's brigand (its hideout
    /// counts it) and a tamed pet stay, frozen until the column returns.
    #[test]
    fn stream_out_reclaims_night_spawns_but_keeps_hideout_brigands_and_pets() {
        use crate::entity::{spawn_mob, NightSpawn, Position};
        use crate::mob::MobType;
        let mut fx = SimsFixture::new();
        fx.sims().stream_in(0, 0);
        let at = glam::Vec3::new(8.5, 90.0, 8.5);
        let night = spawn_mob(&mut fx.ecs, MobType::Brigand, at);
        fx.ecs.insert_one(night, NightSpawn).unwrap();
        let guard = spawn_mob(&mut fx.ecs, MobType::Brigand, at);
        fx.ecs
            .insert_one(guard, crate::brigand::HomeHideout { anchor: [8, 80, 8] })
            .unwrap();
        let wolf = spawn_mob(&mut fx.ecs, MobType::Wolf, at);

        fx.sims().stream_out(0, 0);
        assert!(!fx.ecs.contains(night), "the night spawn is reclaimed");
        assert!(fx.ecs.contains(guard), "the hideout brigand stays");
        assert!(fx.ecs.contains(wolf), "a non-scattered mob stays");
        assert_eq!(fx.ecs.get::<&Position>(guard).unwrap().0, at, "untouched");
    }

    /// Phase B1 review (MED) — a client applies a server block change only to
    /// a column it holds: a loaded one, or an evicted one (the write goes
    /// through to the store). Anywhere else `set_block` would conjure a stray
    /// chunk that the client's own generation later skips (a 16³ hole).
    #[test]
    fn a_remote_change_lands_only_in_a_loaded_or_evicted_column() {
        let mut fx = SimsFixture::new();
        fx.sims().stream_in(0, 0);
        fx.sims().stream_in(2, 0);
        fx.world.place_player_block(40, 90, 4, block::GLASS);
        fx.sims().stream_out(2, 0); // edited: kept in the evicted store
        assert!(remote_change_is_loaded(&fx.loaded, &fx.world, 3, 4), "loaded");
        assert!(remote_change_is_loaded(&fx.loaded, &fx.world, 40, 4), "evicted: write-through");
        assert!(!remote_change_is_loaded(&fx.loaded, &fx.world, 20, 4), "never loaded");
        assert!(!remote_change_is_loaded(&fx.loaded, &fx.world, -1, 4), "never loaded (west)");
    }

    /// The LAN host's world is the save of record, so a server change for a
    /// column its client has not loaded (a joiner's edit near spawn while the
    /// host is away) is kept, not dropped: the column is generated, the change
    /// applied, and the column evicted. It streams back in whole — bedrock
    /// and the edit.
    #[test]
    fn a_hosts_remote_change_in_an_unloaded_column_is_kept_whole() {
        let mut fx = SimsFixture::new();
        let bc = crate::protocol::BlockChange::with_meta(20, 90, 4, block::GLASS, 0);
        apply_remote_change_to_unloaded_column(&mut fx.world, &fx.bg, &bc);
        assert!(fx.world.is_column_evicted(1, 0), "kept in the evicted store");
        assert!(!(0..=MAX_CHUNK_Y).any(|cy| fx.world.has_chunk(1, cy, 0)), "no stray live chunk");

        fx.sims().stream_in(1, 0);
        assert_eq!(fx.world.get_block(20, 90, 4), block::GLASS, "the change is there");
        for x in 16..32 {
            for z in 0..16 {
                assert_eq!(fx.world.get_block(x, 0, z), block::BEDROCK, "bedrock at ({x}, 0, {z})");
            }
        }

        // A change that matches world-gen keeps nothing.
        let mut fx = SimsFixture::new();
        let same = crate::protocol::BlockChange::with_meta(40, 0, 4, block::BEDROCK, 0);
        apply_remote_change_to_unloaded_column(&mut fx.world, &fx.bg, &same);
        assert!(!fx.world.is_column_evicted(2, 0), "a pristine column is dropped again");
        assert!(!(0..=MAX_CHUNK_Y).any(|cy| fx.world.has_chunk(2, cy, 0)));
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
