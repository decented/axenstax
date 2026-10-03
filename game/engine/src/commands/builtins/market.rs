use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

/// Spec 37 — `/market` returns a compass hint to the nearest Market
/// Hub (or "no markets found"). Read-only; not a cheat.
pub struct MarketCommand;

impl Command for MarketCommand {
    fn name(&self) -> &'static str {
        "market"
    }
    fn help(&self) -> &'static str {
        "Find the nearest market hub"
    }
    fn usage(&self) -> &'static str {
        "/market"
    }
    fn min_op_level(&self) -> OpLevel {
        OpLevel::None
    }
    fn is_cheat(&self) -> bool {
        false
    }
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> CommandResult {
        let from = ctx.players[ctx.player_idx].player.pos;
        match crate::market_hub::nearest_hub(&ctx.world.market_hubs, from) {
            Some((_hub, dist, dir)) => {
                ctx.success(format!(
                    "Nearest market: {} blocks {}",
                    dist.round() as i64,
                    dir.label(),
                ));
            }
            None => {
                ctx.success("No markets found. Place a Market Bell to start one.".to_string());
            }
        }
        CommandResult::Silent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::CommandRegistry;
    use crate::market_hub::{HubOwner, MarketHubData};
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    fn run(world: &mut World, player_pos: glam::Vec3) -> Vec<String> {
        let mut t = 0u32;
        let mut s = 1u32;
        let mut creative = false;
        let mut players = vec![PlayerSlot::new(0, player_pos, 0.5)];
        players[0].player.pos = player_pos;
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
            op_level: OpLevel::None,
            current_tick: 0,
            log: &mut log,
            registry: &reg,
            cheats_used_marker: &mut ch,
            ever_creative_marker: &mut ev,
            pure_survival_broken_marker: &mut ps,
        };
        let r = MarketCommand.execute(&mut ctx, &[]);
        assert_eq!(r, CommandResult::Silent);
        log.iter().map(|l| l.text.clone()).collect()
    }

    #[test]
    fn reports_no_markets_when_empty() {
        let mut world = World::new();
        let out = run(&mut world, glam::Vec3::ZERO);
        assert!(out.iter().any(|l| l.contains("No markets found")), "got {out:?}");
    }

    #[test]
    fn reports_direction_and_distance() {
        let mut world = World::new();
        world.market_hubs.push(MarketHubData::from_bell(
            HubOwner::LocalPlayer(0), 100, 64, 0,
        ));
        let out = run(&mut world, glam::Vec3::new(0.0, 64.0, 0.0));
        assert!(out.iter().any(|l| l.contains("east")), "expected east, got {out:?}");
        assert!(out.iter().any(|l| l.contains("100 blocks")), "expected 100 blocks, got {out:?}");
    }

    #[test]
    fn is_not_a_cheat() {
        assert!(!MarketCommand.is_cheat());
        assert_eq!(MarketCommand.min_op_level(), OpLevel::None);
    }
}
