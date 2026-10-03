//! `/biomes` — list every biome known to the engine + its surface
//! block + tree species pool. Plural sibling of `/biome` (which spot-
//! checks the biome at a coord). Read-only; not a cheat.

use crate::biome::{Biome, biome_properties};
use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

const ALL: &[Biome] = &[
    Biome::Plains, Biome::Forest, Biome::BirchForest, Biome::Taiga,
    Biome::Jungle, Biome::Savanna, Biome::Desert, Biome::SnowyTundra,
    Biome::Mountains, Biome::Ocean,
];

pub struct BiomesCommand;

impl Command for BiomesCommand {
    fn name(&self) -> &'static str { "biomes" }
    fn help(&self) -> &'static str {
        "List every biome the engine knows + its surface + tree pool"
    }
    fn usage(&self) -> &'static str { "/biomes" }
    fn min_op_level(&self) -> OpLevel { OpLevel::None }
    fn is_cheat(&self) -> bool { false }
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> CommandResult {
        ctx.success(format!("{} biomes:", ALL.len()));
        for b in ALL {
            let p = biome_properties(*b);
            let species: Vec<&str> = p.tree_species.iter().map(|s| species_name(*s)).collect();
            ctx.success(format!(
                "  {:?} — surface={}, trees={:?}, density={:.1}",
                b, block_name(p.surface_block), species, p.tree_density,
            ));
        }
        CommandResult::Silent
    }
}

fn species_name(s: crate::block::WoodSpecies) -> &'static str {
    use crate::block::WoodSpecies as W;
    match s {
        W::Oak => "Oak",
        W::Birch => "Birch",
        W::Spruce => "Spruce",
        W::Jungle => "Jungle",
        W::Acacia => "Acacia",
        W::DarkOak => "DarkOak",
        W::Rubber => "Rubber",
    }
}

fn block_name(id: crate::block::BlockId) -> &'static str {
    use crate::block;
    match id {
        x if x == block::GRASS => "grass",
        x if x == block::SAND => "sand",
        x if x == block::SNOW => "snow",
        x if x == block::DIRT => "dirt",
        x if x == block::STONE => "stone",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::CommandRegistry;
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    fn run() -> (CommandResult, Vec<String>) {
        let cmd = BiomesCommand;
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::new(0.0, 70.0, 0.0), 0.5)];
        let mut log = Vec::new();
        let mut ch = false;
        let mut ev = false;
        let mut ps = false;
        let reg = CommandRegistry::new();
        let mut ctx = CommandContext {
            world: &mut world, world_time: &mut t, world_time_step: &mut s,
            is_creative: &mut creative, play_mode: &mut { use crate::play_mode::PlayMode; PlayMode::Survival }, seed: 42, world_name: "test",
            players: &mut players, player_idx: 0, op_level: OpLevel::Op,
            current_tick: 0, log: &mut log, registry: &reg,
            cheats_used_marker: &mut ch, ever_creative_marker: &mut ev,
            pure_survival_broken_marker: &mut ps,
        };
        let r = cmd.execute(&mut ctx, &[]);
        let texts: Vec<String> = log.iter().map(|l| l.text.clone()).collect();
        (r, texts)
    }

    #[test]
    fn lists_each_launch_biome() {
        let (_, texts) = run();
        for name in ["Plains", "Forest", "Taiga", "Jungle", "Desert", "SnowyTundra"] {
            assert!(texts.iter().any(|t| t.contains(name)), "missing {name}");
        }
    }
}
