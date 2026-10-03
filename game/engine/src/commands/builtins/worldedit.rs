//! `/we` (#7) — WorldEdit-style region editing. The command parser strips a
//! single `/`, so WorldEdit's `//set` syntax isn't usable; this is one `/we`
//! command group instead:
//!   - `pos1` / `pos2`        — set a selection corner at your feet
//!   - `set <block>`          — fill the selection
//!   - `replace <from> <to>`  — swap one block for another in the selection
//!   - `walls <block>`        — the four side faces of the selection
//!   - `copy` / `paste`       — clipboard the selection / stamp it at your feet
//!   - `stack <n> [x|y|z]`    — repeat the selection n times along an axis
//!   - `size` / `clear`       — report volume / clear the selection
//!
//! Creative-power editing: `OpLevel::Op` + a cheat (flags the world ledger).
//! The mutation ops live in `crate::worldedit`; this just parses + routes.

use crate::block::BlockId;
use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};
use crate::worldedit;

pub struct WorldEditCommand;

fn resolve_block(name: &str) -> Result<BlockId, String> {
    let stack = crate::commands::builtins::give::resolve_item(name, 1)?;
    match stack.item {
        crate::item::Item::Block(id) => Ok(id),
        _ => Err(format!("'{name}' is not a placeable block")),
    }
}

fn foot_block(ctx: &CommandContext) -> Option<[i32; 3]> {
    ctx.players.get(ctx.player_idx).map(|s| {
        let p = s.player.pos;
        [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32]
    })
}

/// Both selection corners, or an error message naming what's missing.
fn selection(ctx: &CommandContext) -> Result<([i32; 3], [i32; 3]), String> {
    let we = &ctx.players[ctx.player_idx].worldedit;
    match (we.pos1, we.pos2) {
        (Some(a), Some(b)) => Ok((a, b)),
        _ => Err("set both corners first (/we pos1, /we pos2)".to_string()),
    }
}

impl Command for WorldEditCommand {
    fn name(&self) -> &'static str {
        "we"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["worldedit"]
    }
    fn help(&self) -> &'static str {
        "Region editing: pos1/pos2, set, replace, walls, copy/paste, stack"
    }
    fn usage(&self) -> &'static str {
        "/we pos1|pos2|set <b>|replace <a> <b>|walls <b>|copy|paste|stack <n> [x|y|z]|size|clear"
    }
    fn min_op_level(&self) -> OpLevel {
        OpLevel::Op
    }
    fn is_cheat(&self) -> bool {
        true
    }

    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        let err = |ctx: &mut CommandContext, m: String| -> CommandResult {
            ctx.error(m.clone());
            CommandResult::Error(m)
        };
        match args.first().map(|s| s.to_lowercase()).as_deref() {
            Some("pos1") | Some("pos2") => {
                let which = args[0].to_lowercase();
                let Some(p) = foot_block(ctx) else {
                    return err(ctx, "no player".to_string());
                };
                let we = &mut ctx.players[ctx.player_idx].worldedit;
                if which == "pos1" {
                    we.pos1 = Some(p);
                } else {
                    we.pos2 = Some(p);
                }
                ctx.success(format!("{which} set to ({}, {}, {})", p[0], p[1], p[2]));
                CommandResult::Success
            }
            Some("size") => {
                let (a, b) = match selection(ctx) {
                    Ok(v) => v,
                    Err(m) => return err(ctx, m),
                };
                let (min, max) = worldedit::bounds(a, b);
                ctx.success(format!("Selection: {} blocks", worldedit::volume(min, max)));
                CommandResult::Success
            }
            Some("clear") => {
                let we = &mut ctx.players[ctx.player_idx].worldedit;
                we.pos1 = None;
                we.pos2 = None;
                ctx.success("Selection cleared.".to_string());
                CommandResult::Success
            }
            Some("set") => {
                let Some(name) = args.get(1) else {
                    return err(ctx, "usage: /we set <block>".to_string());
                };
                let block = match resolve_block(name) {
                    Ok(b) => b,
                    Err(m) => return err(ctx, m),
                };
                let (a, b) = match selection(ctx) {
                    Ok(v) => v,
                    Err(m) => return err(ctx, m),
                };
                let (min, max) = worldedit::bounds(a, b);
                if worldedit::volume(min, max) > worldedit::MAX_REGION_VOLUME {
                    return err(ctx, "selection too large".to_string());
                }
                let mut changed = Vec::new();
                let n = worldedit::region_set(ctx.world, a, b, block, &mut changed);
                ctx.success(format!("Set {n} blocks."));
                CommandResult::RebuildRegion { min, max, changed }
            }
            Some("replace") => {
                let (Some(fname), Some(tname)) = (args.get(1), args.get(2)) else {
                    return err(ctx, "usage: /we replace <from> <to>".to_string());
                };
                let (from, to) = match (resolve_block(fname), resolve_block(tname)) {
                    (Ok(f), Ok(t)) => (f, t),
                    (Err(m), _) | (_, Err(m)) => return err(ctx, m),
                };
                let (a, b) = match selection(ctx) {
                    Ok(v) => v,
                    Err(m) => return err(ctx, m),
                };
                let (min, max) = worldedit::bounds(a, b);
                if worldedit::volume(min, max) > worldedit::MAX_REGION_VOLUME {
                    return err(ctx, "selection too large".to_string());
                }
                let mut changed = Vec::new();
                let n = worldedit::region_replace(ctx.world, a, b, from, to, &mut changed);
                ctx.success(format!("Replaced {n} blocks."));
                CommandResult::RebuildRegion { min, max, changed }
            }
            Some("walls") => {
                let Some(name) = args.get(1) else {
                    return err(ctx, "usage: /we walls <block>".to_string());
                };
                let block = match resolve_block(name) {
                    Ok(b) => b,
                    Err(m) => return err(ctx, m),
                };
                let (a, b) = match selection(ctx) {
                    Ok(v) => v,
                    Err(m) => return err(ctx, m),
                };
                let (min, max) = worldedit::bounds(a, b);
                if worldedit::volume(min, max) > worldedit::MAX_REGION_VOLUME {
                    return err(ctx, "selection too large".to_string());
                }
                let mut changed = Vec::new();
                let n = worldedit::region_walls(ctx.world, a, b, block, &mut changed);
                ctx.success(format!("Walled {n} blocks."));
                CommandResult::RebuildRegion { min, max, changed }
            }
            Some("copy") => {
                let (a, b) = match selection(ctx) {
                    Ok(v) => v,
                    Err(m) => return err(ctx, m),
                };
                let (min, max) = worldedit::bounds(a, b);
                if worldedit::volume(min, max) > worldedit::MAX_REGION_VOLUME {
                    return err(ctx, "selection too large".to_string());
                }
                let clip = worldedit::region_copy(ctx.world, a, b);
                let count = clip.blocks.len();
                ctx.players[ctx.player_idx].worldedit.clipboard = Some(clip);
                ctx.success(format!("Copied {count} blocks to clipboard."));
                CommandResult::Success
            }
            Some("paste") => {
                let Some(at) = foot_block(ctx) else {
                    return err(ctx, "no player".to_string());
                };
                let clip = match ctx.players[ctx.player_idx].worldedit.clipboard.clone() {
                    Some(c) => c,
                    None => return err(ctx, "clipboard empty (/we copy first)".to_string()),
                };
                let mut changed = Vec::new();
                let n = worldedit::clipboard_paste(ctx.world, &clip, at, &mut changed);
                let max = [
                    at[0] + clip.dims[0] - 1,
                    at[1] + clip.dims[1] - 1,
                    at[2] + clip.dims[2] - 1,
                ];
                ctx.success(format!("Pasted {n} blocks."));
                CommandResult::RebuildRegion { min: at, max, changed }
            }
            Some("stack") => {
                let count: i32 = match args.get(1).and_then(|s| s.parse().ok()) {
                    Some(n) if n > 0 && n <= 256 => n,
                    _ => return err(ctx, "usage: /we stack <1-256> [x|y|z]".to_string()),
                };
                let axis = match args.get(2).map(|s| s.to_lowercase()).as_deref() {
                    Some("y") => 1,
                    Some("z") => 2,
                    _ => 0,
                };
                let (a, b) = match selection(ctx) {
                    Ok(v) => v,
                    Err(m) => return err(ctx, m),
                };
                let (min0, max0) = worldedit::bounds(a, b);
                if worldedit::volume(min0, max0).saturating_mul(count as u64)
                    > worldedit::MAX_REGION_VOLUME
                {
                    return err(ctx, "stack result too large".to_string());
                }
                let mut changed = Vec::new();
                let (n, min, max) =
                    worldedit::region_stack(ctx.world, a, b, axis, count, &mut changed);
                ctx.success(format!("Stacked {n} blocks."));
                CommandResult::RebuildRegion { min, max, changed }
            }
            _ => err(
                ctx,
                "usage: /we pos1|pos2|set|replace|walls|copy|paste|stack|size|clear".to_string(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::play_mode::PlayMode;
    use crate::player_slot::PlayerSlot;
    use crate::world::World;
    use glam::Vec3;

    /// Run `/we <args>` against a world + a single player at `player_pos`, with
    /// the worldedit session pre-seeded via `setup`. Returns (result, world).
    fn run(
        world: &mut World,
        player_pos: Vec3,
        setup: impl FnOnce(&mut PlayerSlot),
        args: &[&str],
    ) -> CommandResult {
        let cmd = WorldEditCommand;
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = true;
        let mut mode = PlayMode::Creative;
        let mut players = vec![PlayerSlot::new(0, player_pos, 0.5)];
        setup(&mut players[0]);
        let mut log = Vec::new();
        let (mut ch, mut ev, mut ps) = (false, false, false);
        let reg = CommandRegistry::new();
        let mut ctx = CommandContext {
            world,
            world_time: &mut t,
            world_time_step: &mut s,
            is_creative: &mut creative,
            play_mode: &mut mode,
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
    fn pos_then_set_fills_and_returns_rebuild_region() {
        let mut world = World::new();
        let r = run(
            &mut world,
            Vec3::new(0.5, 4.0, 0.5),
            |slot| {
                slot.worldedit.pos1 = Some([0, 4, 0]);
                slot.worldedit.pos2 = Some([1, 4, 1]);
            },
            &["set", "stone"],
        );
        match r {
            CommandResult::RebuildRegion { min, max, changed } => {
                assert_eq!(min, [0, 4, 0]);
                assert_eq!(max, [1, 4, 1]);
                assert_eq!(changed.len(), 4, "every changed cell is reported for broadcast");
            }
            other => panic!("expected RebuildRegion, got {other:?}"),
        }
        assert_eq!(world.get_block(0, 4, 0), block::STONE);
        assert_eq!(world.get_block(1, 4, 1), block::STONE);
    }

    #[test]
    fn set_without_a_selection_errors() {
        let mut world = World::new();
        let r = run(&mut world, Vec3::new(0.5, 4.0, 0.5), |_| {}, &["set", "stone"]);
        assert!(matches!(r, CommandResult::Error(_)));
        assert_eq!(world.get_block(0, 4, 0), block::AIR);
    }

    #[test]
    fn set_rejects_a_non_block_name() {
        let mut world = World::new();
        let r = run(
            &mut world,
            Vec3::new(0.5, 4.0, 0.5),
            |slot| {
                slot.worldedit.pos1 = Some([0, 4, 0]);
                slot.worldedit.pos2 = Some([0, 4, 0]);
            },
            &["set", "definitely_not_a_block"],
        );
        assert!(matches!(r, CommandResult::Error(_)));
    }

    #[test]
    fn copy_then_paste_stamps_at_the_players_feet() {
        let mut world = World::new();
        world.set_block(0, 4, 0, block::STONE);
        // Copy the single stone cell.
        let r = run(
            &mut world,
            Vec3::new(0.5, 4.0, 0.5),
            |slot| {
                slot.worldedit.pos1 = Some([0, 4, 0]);
                slot.worldedit.pos2 = Some([0, 4, 0]);
            },
            &["copy"],
        );
        assert_eq!(r, CommandResult::Success);
        // The clipboard isn't visible across `run` calls (fresh players), so the
        // copy + paste are exercised separately; here assert copy succeeded by
        // re-running with a pre-seeded clipboard.
        let clip = worldedit::region_copy(&world, [0, 4, 0], [0, 4, 0]);
        let r2 = run(
            &mut world,
            Vec3::new(10.0, 4.0, 0.0),
            |slot| slot.worldedit.clipboard = Some(clip),
            &["paste"],
        );
        assert!(matches!(r2, CommandResult::RebuildRegion { .. }));
        assert_eq!(world.get_block(10, 4, 0), block::STONE);
    }
}
