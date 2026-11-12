//! Test-only helper: run one registered built-in command against a throwaway
//! world and read back what it said.

use super::builtins;
use super::dispatch::{CommandContext, CommandResult};
use super::registry::{CommandRegistry, OpLevel};
use crate::player_slot::PlayerSlot;
use crate::world::World;

/// A registry with every built-in this target registers.
pub(crate) fn full_registry() -> CommandRegistry {
    let mut reg = CommandRegistry::new();
    builtins::register_all(&mut reg);
    reg
}

/// Run `/name args...` at `op` in an empty survival world with one player.
/// Returns the result and the text of every line the command logged (plus
/// the message of an `Error` result).
pub(crate) fn run_named(name: &str, args: &[&str], op: OpLevel) -> (CommandResult, Vec<String>) {
    let reg = full_registry();
    let cmd = reg.lookup(name).unwrap_or_else(|| panic!("no command '{name}'"));
    let mut world = World::new();
    let mut t = 0u32;
    let mut s = 4u32;
    let mut creative = false;
    let mut players = vec![PlayerSlot::new(0, glam::Vec3::new(0.0, 70.0, 0.0), 0.5)];
    let mut log = Vec::new();
    let (mut ch, mut ev, mut ps) = (false, false, false);
    let mut ctx = CommandContext {
        world: &mut world,
        world_time: &mut t,
        world_time_step: &mut s,
        is_creative: &mut creative,
        play_mode: &mut crate::play_mode::PlayMode::Survival,
        seed: 42,
        world_name: "test",
        players: &mut players,
        player_idx: 0,
        op_level: op,
        current_tick: 0,
        log: &mut log,
        registry: &reg,
        cheats_used_marker: &mut ch,
        ever_creative_marker: &mut ev,
        pure_survival_broken_marker: &mut ps,
    };
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let result = cmd.execute(&mut ctx, &owned);
    let mut lines: Vec<String> = log.iter().map(|l| l.text.clone()).collect();
    // Some commands report a refusal only in the result they return.
    if let CommandResult::Error(m) = &result {
        lines.push(m.clone());
    }
    (result, lines)
}
