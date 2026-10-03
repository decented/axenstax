//! `/keepinventory [on|off]` (#47) — toggle keep-inventory for the current
//! session's world. When on, death leaves the inventory intact and spawns no
//! grave; when off, death drops a recoverable grave (the Survival default).
//!
//! This sets the **live** `World.keep_inventory` flag (immediate effect). It is a
//! cheat (marks the world as cheated). Persisting the change into `WorldMeta`
//! across reloads is a follow-up — for now a reload restores the world's saved
//! value (blank-canvas worlds default it on at creation).

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

pub struct KeepInventoryCommand;

impl Command for KeepInventoryCommand {
    fn name(&self) -> &'static str { "keepinventory" }
    fn aliases(&self) -> &'static [&'static str] { &["ki"] }
    fn help(&self) -> &'static str {
        "Toggle keep-inventory: keep your items on death (no grave)"
    }
    fn usage(&self) -> &'static str { "/keepinventory [on|off] | /ki [on|off]" }
    fn min_op_level(&self) -> OpLevel { OpLevel::Op }
    fn is_cheat(&self) -> bool { true }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        let new_value = match args.first().map(|s| s.to_lowercase()) {
            Some(ref s) if s == "on" || s == "true" || s == "1" => true,
            Some(ref s) if s == "off" || s == "false" || s == "0" => false,
            // No argument → toggle the current state.
            None => !ctx.world.keep_inventory,
            Some(other) => {
                let msg = format!("usage: /keepinventory [on|off] (got '{other}')");
                ctx.error(msg.clone());
                return CommandResult::Error(msg);
            }
        };
        ctx.world.keep_inventory = new_value;
        ctx.success(format!(
            "Keep-inventory is now {}",
            if new_value { "ON — items stay on death" } else { "OFF — death drops a grave" }
        ));
        CommandResult::Success
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::CommandRegistry;
    use crate::play_mode::PlayMode;
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    fn run(world: &mut World, args: &[&str]) -> CommandResult {
        let cmd = KeepInventoryCommand;
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut mode = PlayMode::Survival;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::new(0.0, 70.0, 0.0), 0.5)];
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
    fn on_off_and_toggle_set_the_world_flag() {
        let mut world = World::new();
        assert!(!world.keep_inventory, "default off");
        run(&mut world, &["on"]);
        assert!(world.keep_inventory, "/keepinventory on");
        run(&mut world, &["off"]);
        assert!(!world.keep_inventory, "/keepinventory off");
        // No-arg toggles.
        run(&mut world, &[]);
        assert!(world.keep_inventory, "no-arg toggles on");
        run(&mut world, &[]);
        assert!(!world.keep_inventory, "no-arg toggles off again");
    }

    #[test]
    fn is_a_cheat() {
        assert!(KeepInventoryCommand.is_cheat());
    }
}
