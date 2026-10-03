//! `/ws` — The Workshop authoring command (Spec 40).
//!
//! BRIDGE: this command surface is a **testable, functional control surface** for
//! the Workshop redesign loop — place an asset, paint it, pin it, and watch every
//! instance reskin — while the **tactile bellows + 16×16 paint-grid UI** (the
//! kid-facing UX the owner designed) is built and feel-tuned with Axolittle. The
//! loop it drives is the real pipeline (`workshop::commit_project_override` →
//! `OverrideRegistry` → the mesher/entity seams), so the command and the future
//! tactile UI commit through the exact same path. Replace the command's role as the
//! *primary* interface when the bellows/grid UI lands; keep it as a power-user /
//! test affordance.
//!
//! Subcommands:
//! - `/ws place <asset>` — add a project for an existing block (`flower`, `poppy`,
//!   `stone`, …) or mob (`mob:cow`). Echoes the new project id.
//! - `/ws paint [id] <r> <g> <b>` — set a solid reskin colour (0–255) on the
//!   project (defaults to the most-recent). A block gets all 6 faces; a mob gets
//!   every part.
//! - `/ws pump [id]` / `/ws deflate [id]` — step the working inflation.
//! - `/ws pin [id]` — commit the reskin: every instance of that asset updates.
//! - `/ws list` — list the Workshop's projects.
//! - `/ws revert` — revert all your reskins/reshapes back to stock art. (NOT the
//!   same as the Lobby "Reset Workshop" button, which wipes the room's blocks.)

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};
use crate::override_registry::AuthoredFaces;
use crate::workshop::{commit_project_override, WorkshopMode, WorkshopPaint, WorkshopTarget};

pub struct WorkshopCommand;

/// The signed-in player's pubkey (lowercase hex) on WASM, empty on native.
/// Stored hex-only in the engine; npub encoding is a JS display-boundary concern.
fn session_pubkey_hex() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        crate::save::WASM_PUBKEY.with(|p| p.borrow().clone()).unwrap_or_default()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        String::new()
    }
}

/// Resolve a friendly asset name to a Workshop target. Blocks the player is most
/// likely to want to redesign first; extend freely. `mob:<name>` selects a mob.
fn target_by_name(key: &str) -> Option<WorkshopTarget> {
    if let Some(mob) = key.strip_prefix("mob:") {
        return mob_by_name(mob).map(WorkshopTarget::Mob);
    }
    let id = match key {
        "flower" | "cornflower" => crate::block::CORNFLOWER,
        "poppy" => crate::block::FIELD_POPPY,
        "buttercup" => crate::block::BUTTERCUP,
        "stone" => crate::block::STONE,
        "dirt" => crate::block::DIRT,
        "grass" => crate::block::GRASS,
        "sand" => crate::block::SAND,
        "log" | "wood" => crate::block::OAK_LOG,
        _ => return None,
    };
    Some(WorkshopTarget::Block(id))
}

fn mob_by_name(key: &str) -> Option<crate::mob::MobType> {
    use crate::mob::MobType;
    match key {
        "cow" => Some(MobType::Cow),
        "pig" => Some(MobType::Pig),
        "sheep" => Some(MobType::Sheep),
        "chicken" => Some(MobType::Chicken),
        _ => None,
    }
}

/// Accept an npub (bech32) OR raw 64-hex; return canonical lowercase hex, else None.
fn to_hex_pubkey(s: &str) -> Option<String> {
    let t = s.trim();
    if let Some(hex) = crate::open_stash::normalize_pubkey(t) {
        return Some(hex);
    } // 64-hex
    crate::npub::npub_to_hex(t) // npub -> hex (wasm; None on native)
}

/// Parse an optional leading project-id arg, returning (id_opt, remaining_args).
fn split_id(args: &[String]) -> (Option<u32>, &[String]) {
    if let Some(first) = args.first()
        && let Ok(id) = first.parse::<u32>() {
            return (Some(id), &args[1..]);
        }
    (None, args)
}

/// Resolve the target project id: explicit, else the most-recently-added.
fn resolve_id(ctx: &CommandContext, explicit: Option<u32>) -> Option<u32> {
    explicit.or_else(|| ctx.world.workshop.iter().last().map(|p| p.id))
}

impl Command for WorkshopCommand {
    fn name(&self) -> &'static str {
        "ws"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["workshop"]
    }
    fn help(&self) -> &'static str {
        "The Workshop — redesign an existing asset's look (place/paint/pin)"
    }
    fn usage(&self) -> &'static str {
        "/ws place <flower|poppy|stone|mob:cow|…> | edit <asset> (paint-grid) | paint [id] <r> <g> <b> | pump|deflate [id] | play [on|off] | pin [id] | reshape <block> | gallery | list | revert"
    }
    fn min_op_level(&self) -> OpLevel {
        OpLevel::Op
    }

    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        let sub = match args.first() {
            Some(s) => s.to_lowercase(),
            None => {
                ctx.error("usage: /ws place|paint|pump|deflate|pin|list|revert");
                return CommandResult::Error("missing subcommand".to_string());
            }
        };
        let rest = &args[1..];

        // Any non-publish subcommand cancels a staged publish — the confirm must
        // immediately follow its prompt (the `publish` arm manages its own pending).
        if sub != "publish" {
            ctx.world.beacon_publish_pending = None;
        }

        match sub.as_str() {
            "place" => {
                let Some(name) = rest.first().map(|s| s.to_lowercase()) else {
                    ctx.error("usage: /ws place <asset>");
                    return CommandResult::Error("missing asset".to_string());
                };
                let Some(target) = target_by_name(&name) else {
                    ctx.error(format!("unknown asset '{name}' (try flower, poppy, stone, mob:cow)"));
                    return CommandResult::Error("unknown asset".to_string());
                };
                // Place at the player's feet so a future render shows it there.
                let origin = ctx
                    .players
                    .get(ctx.player_idx)
                    .map(|s| [s.player.pos.x as i32, s.player.pos.y as i32, s.player.pos.z as i32])
                    .unwrap_or([0, 0, 0]);
                let id = ctx.world.workshop.add(target, WorkshopMode::Reskin, origin);
                *ctx.cheats_used_marker = true;
                ctx.success(format!("Placed Workshop project #{id} ({name}). /ws paint {id} <r> <g> <b>, then /ws pin {id}."));
                CommandResult::Success
            }
            "edit" => {
                // Open the 16×16 paint-grid panel for an existing asset.
                // `/ws edit <block>` or `/ws edit mob:<name> [part]`.
                let Some(name) = rest.first().map(|s| s.to_lowercase()) else {
                    ctx.error("usage: /ws edit <block|mob:cow [part]>");
                    return CommandResult::Error("missing asset".to_string());
                };
                let target = match target_by_name(&name) {
                    Some(WorkshopTarget::Block(id)) => {
                        crate::workshop_painter::PaintTarget::Block(id)
                    }
                    Some(WorkshopTarget::Mob(mob)) => {
                        let part: u8 = rest.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                        let parts = crate::entity_model::mob_model(mob).len() as u8;
                        if part >= parts.max(1) {
                            ctx.error(format!("{name} has parts 0..{}", parts.saturating_sub(1)));
                            return CommandResult::Error("bad part".to_string());
                        }
                        crate::workshop_painter::PaintTarget::MobPart { mob, part }
                    }
                    // `target_by_name` never yields Avatar — the avatar is painted
                    // in-world via the Bellows blow-up, not the /ws painter.
                    Some(WorkshopTarget::Avatar) => {
                        ctx.error("paint your avatar in the Workshop (aim the Bellows at the mannequin), not with /ws edit");
                        return CommandResult::Error("avatar not a /ws asset".to_string());
                    }
                    None => {
                        ctx.error(format!("unknown asset '{name}'"));
                        return CommandResult::Error("unknown asset".to_string());
                    }
                };
                *ctx.cheats_used_marker = true;
                ctx.success(format!("Opening the painter for {name} — paint the faces, then Pin."));
                CommandResult::OpenWorkshopPainter { target }
            }
            "paint" => {
                // Disambiguate by arg count (an id and r/g/b are all numbers):
                // 3 args = r g b (most-recent project); 4 args = id r g b.
                let (id_opt, rgb): (Option<u32>, &[String]) = if rest.len() >= 4 {
                    (rest[0].parse::<u32>().ok(), &rest[1..])
                } else {
                    (None, rest)
                };
                let Some(id) = resolve_id(ctx, id_opt) else {
                    ctx.error("no project to paint — /ws place <asset> first");
                    return CommandResult::Error("no project".to_string());
                };
                if rgb.len() < 3 {
                    ctx.error("usage: /ws paint [id] <r> <g> <b>  (0–255)");
                    return CommandResult::Error("need r g b".to_string());
                }
                let parse = |s: &String| s.parse::<u8>().ok();
                let (Some(r), Some(g), Some(b)) = (parse(&rgb[0]), parse(&rgb[1]), parse(&rgb[2])) else {
                    ctx.error("r g b must be 0–255");
                    return CommandResult::Error("bad rgb".to_string());
                };
                let faces = AuthoredFaces::solid([r, g, b, 255]);
                // Resolve the paint shape from the project's target BEFORE the
                // mutable borrow (mob part count needs the model).
                let paint = match ctx.world.workshop.get(id).map(|p| p.target.clone()) {
                    Some(WorkshopTarget::Block(_)) => WorkshopPaint::Block(faces),
                    Some(WorkshopTarget::Mob(mob)) => {
                        let parts = crate::entity_model::mob_model(mob).len();
                        WorkshopPaint::Mob((0..parts as u8).map(|i| (i, faces.clone())).collect())
                    }
                    // The avatar project is a transient Bellows blow-up, never a
                    // /ws-paintable project.
                    Some(WorkshopTarget::Avatar) => {
                        ctx.error("the avatar mannequin is painted in-world, not with /ws paint");
                        return CommandResult::Error("avatar not paintable via /ws".to_string());
                    }
                    None => {
                        ctx.error(format!("no project #{id}"));
                        return CommandResult::Error("no such project".to_string());
                    }
                };
                if let Some(p) = ctx.world.workshop.get_mut(id) {
                    p.paint = paint;
                }
                *ctx.cheats_used_marker = true;
                ctx.success(format!("Painted project #{id} ({r},{g},{b}). /ws pin {id} to apply everywhere."));
                CommandResult::Success
            }
            "pump" | "deflate" => {
                let (id_opt, _) = split_id(rest);
                let Some(id) = resolve_id(ctx, id_opt) else {
                    ctx.error("no project — /ws place <asset> first");
                    return CommandResult::Error("no project".to_string());
                };
                let inflate = sub == "pump";
                if let Some(p) = ctx.world.workshop.get_mut(id) {
                    let changed = if inflate { p.pump() } else { p.deflate() };
                    let lvl = p.inflation;
                    if changed {
                        ctx.success(format!("Project #{id} inflation → {lvl}"));
                    } else {
                        ctx.info(format!("Project #{id} unchanged (inflation {lvl})"));
                    }
                    CommandResult::Success
                } else {
                    ctx.error(format!("no project #{id}"));
                    CommandResult::Error("no such project".to_string())
                }
            }
            "pin" => {
                let (id_opt, _) = split_id(rest);
                let Some(id) = resolve_id(ctx, id_opt) else {
                    ctx.error("no project to pin — /ws place <asset> first");
                    return CommandResult::Error("no project".to_string());
                };
                let base = crate::texture_gen::texture_count();
                // Snapshot the project, commit its paint into the override registry.
                let Some(project) = ctx.world.workshop.get(id).cloned() else {
                    ctx.error(format!("no project #{id}"));
                    return CommandResult::Error("no such project".to_string());
                };
                // Spec 40 blow-up Phase 4 — a locked, in-world-edited working copy
                // bakes through the new pin path (paint→AuthoredFaces / shape→coloured
                // micro-model); the game loop runs it via `commit_locked_balloon`. The
                // old `commit_project_override` path below stays for the power-user
                // `/ws place`+`/ws paint` flow that has no edit buffer.
                let locked_edit = project.edit.is_some()
                    && matches!(project.blow_up, Some(b) if b.phase == crate::workshop::BlowUpPhase::Locked);
                if locked_edit {
                    *ctx.cheats_used_marker = true;
                    ctx.success(format!("Pinned project #{id} — every instance of that block wears your design."));
                    return CommandResult::CommitLockedBalloon(id);
                }
                let wrote = commit_project_override(&project, &mut ctx.world.player_wardrobe, base);
                if !wrote {
                    ctx.error(format!("project #{id} has no paint yet — /ws paint {id} <r> <g> <b> first"));
                    return CommandResult::Error("nothing to pin".to_string());
                }
                if let Some(p) = ctx.world.workshop.get_mut(id) {
                    p.pin();
                }
                *ctx.cheats_used_marker = true;
                ctx.success(format!("Pinned project #{id} — every instance of that asset is reskinned."));
                CommandResult::ApplyWorkshopOverrides
            }
            "play" => {
                // Toggle (or set) the mannequin walk-cycle preview.
                let on = match rest.first().map(|s| s.to_lowercase()).as_deref() {
                    Some("on") | Some("true") | Some("1") => true,
                    Some("off") | Some("false") | Some("0") => false,
                    _ => true, // bare `/ws play` turns it on
                };
                *ctx.cheats_used_marker = true;
                ctx.success(if on {
                    "Mannequins now animate (walk preview)."
                } else {
                    "Mannequins stand still."
                });
                CommandResult::SetWorkshopPlay(on)
            }
            "list" => {
                if ctx.world.workshop.is_empty() {
                    ctx.info("No Workshop projects yet. /ws place <asset> to start.");
                    return CommandResult::Success;
                }
                // Collect lines first (immutable borrow), then echo.
                let lines: Vec<String> = ctx
                    .world
                    .workshop
                    .iter()
                    .map(|p| {
                        let painted = !matches!(p.paint, WorkshopPaint::None);
                        format!(
                            "#{} {:?} inflation={} {}{}",
                            p.id,
                            p.target,
                            p.inflation,
                            if painted { "painted " } else { "" },
                            if p.is_parked() { "(parked)" } else { "(pinned)" }
                        )
                    })
                    .collect();
                for l in lines {
                    ctx.info(l);
                }
                CommandResult::Success
            }
            "revert" => {
                ctx.world.player_wardrobe = crate::override_registry::OverrideRegistry::new();
                *ctx.cheats_used_marker = true;
                ctx.success("Reverted all reskins — assets back to stock art.");
                CommandResult::ApplyWorkshopOverrides
            }
            "reshape" => {
                // Spec 40 Phase F (Mode B) — rebuild an existing BLOCK's shape: build
                // the new form (≤16³) at your feet, then `/ws reshape <asset>` captures
                // it and makes every instance of that block render the 3D micro-model.
                // (Mob reshape is #19 — parked.)
                let Some(name) = rest.first().map(|s| s.to_lowercase()) else {
                    ctx.error("usage: /ws reshape <block-asset>  (build the form at your feet first)");
                    return CommandResult::Error("missing asset".to_string());
                };
                let block_id = match target_by_name(&name) {
                    Some(WorkshopTarget::Block(id)) => id,
                    Some(WorkshopTarget::Mob(_)) => {
                        ctx.error("reshape is blocks-only in v1 (mob reshape = #19, parked)");
                        return CommandResult::Error("mob reshape unsupported".to_string());
                    }
                    // The avatar is Reskin-only (paint-only); reshape is geometry-
                    // meaningless for a fixed humanoid skin.
                    Some(WorkshopTarget::Avatar) => {
                        ctx.error("the avatar can't be reshaped — it's paint-only");
                        return CommandResult::Error("avatar reshape unsupported".to_string());
                    }
                    None => {
                        ctx.error(format!("unknown asset '{name}'"));
                        return CommandResult::Error("unknown asset".to_string());
                    }
                };
                // Capture the RESHAPE_BOX³ cube at the player's feet.
                let feet = ctx
                    .players
                    .get(ctx.player_idx)
                    .map(|s| {
                        [
                            s.player.pos.x.floor() as i32,
                            s.player.pos.y.floor() as i32,
                            s.player.pos.z.floor() as i32,
                        ]
                    })
                    .unwrap_or([0, 0, 0]);
                let Some(plan) =
                    crate::workshop::capture_box_as_plan(ctx.world, feet, crate::workshop::RESHAPE_BOX)
                else {
                    ctx.error("nothing to capture — build the new form at your feet first");
                    return CommandResult::Error("empty capture".to_string());
                };
                *ctx.cheats_used_marker = true;
                ctx.success(format!(
                    "Reshaped {name}: every instance now renders your {}-cell build.",
                    plan.cells.len()
                ));
                CommandResult::ApplyWorkshopReshape { block_id, plan }
            }
            "publish" => {
                // Explicit two-step public-content gate (CONSUMING.md §7): publishing is
                // public, under the player's name, in the clear, effectively permanent —
                // distinct from the private Stash save, never auto-fired.
                if rest.first().map(String::as_str) == Some("confirm") {
                    let Some(name) = ctx.world.beacon_publish_pending.take() else {
                        ctx.error("nothing staged to share — /ws publish <name> first");
                        return CommandResult::Error("no pending publish".to_string());
                    };
                    let mut set = ctx.world.player_wardrobe.set().clone();
                    if set.is_empty() {
                        ctx.error("nothing to publish yet — pin a redesign first");
                        return CommandResult::Error("empty override set".to_string());
                    }
                    if set.author_npub.is_empty() {
                        let hex = session_pubkey_hex();
                        if !hex.is_empty() {
                            set.author_npub = hex;
                        }
                    }
                    if set.version == 0 {
                        set.version = crate::override_registry::OVERRIDE_SET_VERSION;
                    }
                    let bytes = match set.to_blob_bytes() {
                        Ok(b) => b,
                        Err(e) => {
                            ctx.error(format!("could not serialise: {e}"));
                            return CommandResult::Error(e);
                        }
                    };
                    ctx.success(format!("Sharing “{name}” with everyone…"));
                    return CommandResult::PublishOverrideSet { name, bytes };
                }
                // First step: stage + warn. Refuse an empty set up front (no pending stored).
                if ctx.world.player_wardrobe.is_empty() {
                    ctx.error("nothing to publish yet — pin a redesign first (/ws pin)");
                    return CommandResult::Error("empty override set".to_string());
                }
                let name = if rest.is_empty() { "My redesign".to_string() } else { rest.join(" ") };
                ctx.world.beacon_publish_pending = Some(name.clone());
                ctx.success(format!(
                    "Share “{name}” with EVERYONE? Anyone can see your name and download it, and it can’t be unshared. Type /ws publish confirm to share."
                ));
                CommandResult::Success
            }
            "follow" | "unfollow" => {
                let Some(arg) = rest.first() else {
                    ctx.error("usage: /ws follow <npub>");
                    return CommandResult::Error("missing npub".to_string());
                };
                let Some(hex) = to_hex_pubkey(arg) else {
                    ctx.error("not a valid npub or pubkey");
                    return CommandResult::Error("bad pubkey".to_string());
                };
                let follow = sub == "follow";
                ctx.success(if follow {
                    format!("Following {arg} — their shared redesigns will show in /ws browse.")
                } else {
                    format!("Unfollowed {arg}.")
                });
                CommandResult::BeaconFollow { pubkey_hex: hex, follow }
            }
            "following" => {
                ctx.info("Fetching who you follow…");
                CommandResult::BeaconFollowing
            }
            "browse" => {
                let pubkey_hex = match rest.first() {
                    None => None,
                    Some(a) => match to_hex_pubkey(a) {
                        Some(h) => Some(h),
                        None => {
                            ctx.error("not a valid npub or pubkey");
                            return CommandResult::Error("bad pubkey".to_string());
                        }
                    },
                };
                ctx.info("Browsing published redesigns…");
                CommandResult::BeaconBrowse { pubkey_hex }
            }
            "adopt" => {
                let Some(id_str) = rest.first() else {
                    ctx.error("usage: /ws adopt <number from /ws browse>");
                    return CommandResult::Error("missing id".to_string());
                };
                let Ok(idx) = id_str.parse::<usize>() else {
                    ctx.error("adopt takes a number from /ws browse");
                    return CommandResult::Error("bad id".to_string());
                };
                let Some(entry) = ctx.world.beacon_browse_cache.get(idx) else {
                    ctx.error("nothing to adopt at that number — /ws browse first");
                    return CommandResult::Error("no browse cache".to_string());
                };
                let blob_hash = entry.blob_hash.clone();
                ctx.success(format!("Adopting redesign #{idx} ({})…", entry.label));
                CommandResult::BeaconAdopt { blob_hash }
            }
            "gallery" => {
                // Workshop Phase 5 Task 6 — open the Wardrobe panel. Workshop-only:
                // the wardrobe lists this world's authored designs, which only exist
                // in the Workshop world.
                if !ctx.world.is_workshop {
                    ctx.error("the Wardrobe is a Workshop tool — open it inside the Workshop");
                    return CommandResult::Error("not in workshop".to_string());
                }
                *ctx.cheats_used_marker = true;
                ctx.success("Opening the Wardrobe — your design gallery.");
                CommandResult::OpenWardrobe
            }
            other => {
                ctx.error(format!("unknown /ws subcommand '{other}'"));
                CommandResult::Error("unknown subcommand".to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    /// Drive the command against a fresh World, returning (result, world) so tests
    /// can assert on the workshop + override registry state.
    fn run_on(world: &mut World, args: &[&str]) -> CommandResult {
        let cmd = WorkshopCommand;
        let mut t = 0u32;
        let mut s = 1u32;
        let mut creative = true;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::new(2.0, 80.0, 3.0), 0.5)];
        let mut log = Vec::new();
        let (mut ch, mut ev, mut ps) = (false, false, false);
        let reg = CommandRegistry::new();
        let mut ctx = CommandContext {
            world,
            world_time: &mut t,
            world_time_step: &mut s,
            is_creative: &mut creative, play_mode: &mut { use crate::play_mode::PlayMode; PlayMode::Survival },
            seed: 42,
            world_name: "test",
            players: &mut players,
            player_idx: 0,
            op_level: OpLevel::Op,
            current_tick: 0,
            log: &mut log,
            registry: &reg,
            cheats_used_marker: &mut ch,
            ever_creative_marker: &mut ev,
            pure_survival_broken_marker: &mut ps,
        };
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        cmd.execute(&mut ctx, &owned)
    }

    #[test]
    fn place_paint_pin_reskins_a_block_globally() {
        let mut world = World::new();
        // Place a flower project.
        assert!(matches!(run_on(&mut world, &["place", "flower"]), CommandResult::Success));
        assert_eq!(world.workshop.len(), 1);
        let id = world.workshop.iter().next().unwrap().id;

        // No override before pin.
        assert!(world.player_wardrobe.block_face_layer(crate::block::CORNFLOWER, 0).is_none());

        // Paint (defaults to the most-recent project) then pin.
        assert!(matches!(run_on(&mut world, &["paint", "200", "40", "40"]), CommandResult::Success));
        let pin = run_on(&mut world, &["pin", &id.to_string()]);
        assert!(matches!(pin, CommandResult::ApplyWorkshopOverrides), "pin triggers a render apply");

        // The flower is now globally reskinned (in the player wardrobe) + the project is pinned.
        let base = crate::texture_gen::texture_count();
        for face in 0..6u8 {
            assert_eq!(world.player_wardrobe.block_face_layer(crate::block::CORNFLOWER, face), Some(base));
        }
        assert!(!world.workshop.get(id).unwrap().is_parked(), "pinned");
    }

    #[test]
    fn pin_without_paint_is_rejected() {
        let mut world = World::new();
        run_on(&mut world, &["place", "stone"]);
        let r = run_on(&mut world, &["pin"]);
        assert!(matches!(r, CommandResult::Error(_)));
        assert!(world.player_wardrobe.is_empty(), "nothing committed");
    }

    #[test]
    fn unknown_asset_and_subcommand_error() {
        let mut world = World::new();
        assert!(matches!(run_on(&mut world, &["place", "banana"]), CommandResult::Error(_)));
        assert!(matches!(run_on(&mut world, &["wobble"]), CommandResult::Error(_)));
        assert!(world.workshop.is_empty());
    }

    #[test]
    fn revert_clears_overrides() {
        let mut world = World::new();
        run_on(&mut world, &["place", "flower"]);
        run_on(&mut world, &["paint", "10", "20", "30"]);
        run_on(&mut world, &["pin"]);
        assert!(!world.player_wardrobe.is_empty());
        let r = run_on(&mut world, &["revert"]);
        assert!(matches!(r, CommandResult::ApplyWorkshopOverrides));
        assert!(world.player_wardrobe.is_empty(), "revert reverts to stock art");
    }

    #[test]
    fn reshape_captures_a_build_into_a_shape_override_result() {
        // Spec 40 Phase F — build a form at the player's feet, then `/ws reshape
        // <block>` returns an ApplyWorkshopReshape carrying the captured plan for
        // that block id (the game loop bakes + registers it).
        let mut world = World::new();
        // Player feet are (2, 80, 3) in the test harness; build there.
        world.set_block(2, 80, 3, crate::block::STONE);
        world.set_block(3, 80, 3, crate::block::STONE);
        world.set_block(2, 81, 3, crate::block::DIRT);

        let r = run_on(&mut world, &["reshape", "flower"]);
        match r {
            CommandResult::ApplyWorkshopReshape { block_id, plan } => {
                assert_eq!(block_id, crate::block::CORNFLOWER, "reshape targets the named block");
                assert_eq!(plan.cells.len(), 3, "captured the 3-block build");
            }
            other => panic!("expected ApplyWorkshopReshape, got {other:?}"),
        }
    }

    #[test]
    fn reshape_with_nothing_built_errors() {
        let mut world = World::new();
        let r = run_on(&mut world, &["reshape", "flower"]);
        assert!(matches!(r, CommandResult::Error(_)), "empty capture is rejected");
    }

    #[test]
    fn reshape_rejects_mobs() {
        let mut world = World::new();
        world.set_block(2, 80, 3, crate::block::STONE);
        let r = run_on(&mut world, &["reshape", "mob:cow"]);
        assert!(matches!(r, CommandResult::Error(_)), "mob reshape is parked (#19)");
    }

    #[test]
    fn publish_with_no_overrides_is_rejected() {
        let mut world = World::new();
        let r = run_on(&mut world, &["publish", "My Pack"]);
        assert!(matches!(r, CommandResult::Error(_)), "nothing to publish");
        assert!(world.beacon_publish_pending.is_none(), "no pending set when nothing to publish");
    }

    #[test]
    fn publish_requires_explicit_confirm_then_serialises() {
        let mut world = World::new();
        run_on(&mut world, &["place", "flower"]);
        run_on(&mut world, &["paint", "9", "9", "9"]);
        run_on(&mut world, &["pin"]);
        // First publish: a confirm prompt, NOT a publish side-effect.
        let first = run_on(&mut world, &["publish", "Flower Pack"]);
        assert!(matches!(first, CommandResult::Success), "first publish asks to confirm");
        assert_eq!(world.beacon_publish_pending.as_deref(), Some("Flower Pack"));
        // Confirm: now it serialises + returns the publish side-effect.
        match run_on(&mut world, &["publish", "confirm"]) {
            CommandResult::PublishOverrideSet { name, bytes } => {
                assert_eq!(name, "Flower Pack");
                let set = crate::override_registry::OverrideSet::from_blob_bytes(&bytes).unwrap();
                assert!(!set.block_designs.is_empty(), "the pinned reskin is in the blob");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(world.beacon_publish_pending.is_none(), "pending cleared after confirm");
    }

    #[test]
    fn confirm_without_pending_is_rejected() {
        let mut world = World::new();
        assert!(matches!(run_on(&mut world, &["publish", "confirm"]), CommandResult::Error(_)));
    }

    #[test]
    fn another_subcommand_clears_a_pending_publish() {
        let mut world = World::new();
        run_on(&mut world, &["place", "flower"]);
        run_on(&mut world, &["paint", "9", "9", "9"]);
        run_on(&mut world, &["pin"]);
        run_on(&mut world, &["publish", "Flower Pack"]);
        assert!(world.beacon_publish_pending.is_some());
        run_on(&mut world, &["list"]);   // any unrelated /ws subcommand
        assert!(world.beacon_publish_pending.is_none(), "an unrelated subcommand clears the pending publish");
    }

    #[test]
    fn follow_normalises_hex_and_rejects_junk() {
        let mut world = World::new();
        let hex = "a".repeat(64);
        match run_on(&mut world, &["follow", &hex]) {
            CommandResult::BeaconFollow { pubkey_hex, follow } => {
                assert_eq!(pubkey_hex, hex);
                assert!(follow);
            }
            other => panic!("expected BeaconFollow, got {other:?}"),
        }
        match run_on(&mut world, &["unfollow", &hex]) {
            CommandResult::BeaconFollow { follow, .. } => assert!(!follow),
            other => panic!("expected BeaconFollow(unfollow), got {other:?}"),
        }
        assert!(matches!(run_on(&mut world, &["follow", "not-a-key"]), CommandResult::Error(_)));
        assert!(matches!(run_on(&mut world, &["follow"]), CommandResult::Error(_)));
    }

    #[test]
    fn following_and_browse_route() {
        let mut world = World::new();
        assert!(matches!(run_on(&mut world, &["following"]), CommandResult::BeaconFollowing));
        assert!(matches!(run_on(&mut world, &["browse"]), CommandResult::BeaconBrowse { pubkey_hex: None }));
        let hex = "b".repeat(64);
        match run_on(&mut world, &["browse", &hex]) {
            CommandResult::BeaconBrowse { pubkey_hex: Some(p) } => assert_eq!(p, hex),
            other => panic!("expected BeaconBrowse(Some), got {other:?}"),
        }
        assert!(matches!(run_on(&mut world, &["browse", "nope"]), CommandResult::Error(_)));
    }

    #[test]
    fn adopt_requires_id_and_a_prior_browse() {
        let mut world = World::new();
        assert!(matches!(run_on(&mut world, &["adopt"]), CommandResult::Error(_))); // no id
        assert!(matches!(run_on(&mut world, &["adopt", "0"]), CommandResult::Error(_))); // empty cache
        assert!(matches!(run_on(&mut world, &["adopt", "x"]), CommandResult::Error(_))); // non-numeric
                                                                                         // With a cache row, adopt resolves the blob hash.
        world.beacon_browse_cache.push(crate::world::BrowseEntry {
            label: "L".into(),
            author_npub: "npub1x".into(),
            blob_hash: "deadbeef".into(),
            size: 1,
        });
        match run_on(&mut world, &["adopt", "0"]) {
            CommandResult::BeaconAdopt { blob_hash } => assert_eq!(blob_hash, "deadbeef"),
            other => panic!("expected BeaconAdopt, got {other:?}"),
        }
    }

    #[test]
    fn paint_and_pin_a_mob_sets_every_part() {
        let mut world = World::new();
        run_on(&mut world, &["place", "mob:cow"]);
        run_on(&mut world, &["paint", "9", "9", "9"]);
        assert!(matches!(run_on(&mut world, &["pin"]), CommandResult::ApplyWorkshopOverrides));
        // Every part of the cow model is now overridden in the player wardrobe.
        let parts = crate::entity_model::mob_model(crate::mob::MobType::Cow).len();
        for part in 0..parts as u8 {
            assert!(
                world.player_wardrobe.mob_part_faces(crate::mob::MobType::Cow, part, &[0; 6]).is_some(),
                "cow part {part} reskinned"
            );
        }
    }
}
