//! Built-in commands. Game-specific (voxel/Minecraft-style); other games
//! register their own.

pub mod help;
pub mod time;
pub mod gamemode;
pub mod tp;
pub mod seed;
pub mod clear;
pub mod give;
pub mod spawnpoint;
pub mod heal;
pub mod kill;
pub mod killall;
pub mod spawn;
pub mod place;
pub mod clearitems;
pub mod biome;
pub mod iteminfo;
pub mod recipes;
// Spec 28d chunk 12 — debug command bundle.
pub mod mobs;
// #47 — keep-inventory toggle.
pub mod keepinventory;
pub mod biomes;
pub mod tools;
pub mod armour;
pub mod reach;
pub mod timestep;
// Spec 37 — market hub discovery.
pub mod market;
// Goal 1 — challenge scenario runner launcher.
pub mod scenario;
// Spec 40 — The Workshop authoring command surface.
pub mod workshop;
// Player→maker feedback (`/bug`, `/idea`) and maker replies (`/mailbox`). Native
// only: the browser build has no feedback channel (removed 2026-10-01), so these
// are not compiled there and are unknown commands on the web taster.
#[cfg(not(target_arch = "wasm32"))]
pub mod feedback;
#[cfg(not(target_arch = "wasm32"))]
pub mod mailbox;
// World chat §4.5 — attach chat to a room. Native only: no room plug and no
// chat at all on the web taster (spec §0/§6), and this file's own usage text
// names `/room invite` literally, which the web-bundle forbidden-symbol gate
// greps for.
#[cfg(not(target_arch = "wasm32"))]
pub mod room;
// Online play by contact — native only (no online play on the web taster).
#[cfg(not(target_arch = "wasm32"))]
pub mod online;
// Rail freight (Phase 1) — debug cart spawner.
pub mod spawncart;
// #6 — map waypoint management.
pub mod waypoint;
// #7 — WorldEdit-style region editing.
pub mod worldedit;
// #9 — blueprint build-guide.
pub mod buildguide;
// #10 — Minecraft schematic import.
pub mod importschem;
// Creator Gallery (Spec 2026-06-19 §9 Phase 1c) — author 2D art exhibits.
pub mod exhibit;
pub mod trial;

use super::registry::CommandRegistry;

pub fn register_all(registry: &mut CommandRegistry) {
    registry.register(Box::new(help::HelpCommand));
    registry.register(Box::new(time::TimeCommand));
    registry.register(Box::new(gamemode::GamemodeCommand));
    registry.register(Box::new(keepinventory::KeepInventoryCommand));
    registry.register(Box::new(waypoint::WaypointCommand));
    registry.register(Box::new(worldedit::WorldEditCommand));
    registry.register(Box::new(buildguide::BuildGuideCommand));
    registry.register(Box::new(importschem::ImportSchemCommand));
    registry.register(Box::new(exhibit::ExhibitCommand));
    registry.register(Box::new(tp::TpCommand));
    registry.register(Box::new(seed::SeedCommand));
    registry.register(Box::new(clear::ClearCommand));
    registry.register(Box::new(give::GiveCommand));
    registry.register(Box::new(spawnpoint::SpawnpointCommand));
    registry.register(Box::new(heal::HealCommand));
    registry.register(Box::new(kill::KillCommand));
    registry.register(Box::new(killall::KillallCommand));
    registry.register(Box::new(spawn::SpawnCommand));
    registry.register(Box::new(place::PlaceCommand));
    registry.register(Box::new(spawncart::SpawnCartCommand));
    registry.register(Box::new(clearitems::ClearItemsCommand));
    registry.register(Box::new(biome::BiomeCommand));
    registry.register(Box::new(iteminfo::ComplexityTierCommand));
    registry.register(Box::new(iteminfo::TradeValueCommand));
    registry.register(Box::new(recipes::RecipesCommand));
    // Chunk 12 — debug commands.
    registry.register(Box::new(mobs::MobsCommand));
    registry.register(Box::new(biomes::BiomesCommand));
    registry.register(Box::new(tools::ToolsCommand));
    registry.register(Box::new(armour::ArmourCommand));
    registry.register(Box::new(reach::ReachCommand));
    registry.register(Box::new(timestep::TimestepCommand));
    // Spec 37 — market hub discovery.
    registry.register(Box::new(market::MarketCommand));
    // Goal 1 — challenge scenario launcher.
    registry.register(Box::new(scenario::ScenarioCommand));
    // Trials — the Challenge Engine's first family (⚡ Race).
    registry.register(Box::new(trial::TrialCommand));
    // Spec 40 — The Workshop authoring command surface.
    registry.register(Box::new(workshop::WorkshopCommand));
    // Player→maker feedback — native only (see the `mod feedback` cfg above).
    #[cfg(not(target_arch = "wasm32"))]
    {
        registry.register(Box::new(feedback::BugCommand));
        registry.register(Box::new(feedback::IdeaCommand));
        registry.register(Box::new(mailbox::MailboxCommand));
        // Alpha-tester gate (2026-10-03): registered but HIDDEN until the
        // caller applies `native_mailbox::apply_gate` with the player's
        // settings. Fail-closed — a registry nobody gated shows no feedback.
        for name in crate::native_mailbox::FEEDBACK_COMMANDS {
            registry.set_hidden(name, true);
        }
    }
    // World chat §4.5 — native only (see the `mod room` cfg above).
    #[cfg(not(target_arch = "wasm32"))]
    registry.register(Box::new(room::RoomCommand));
    // Online play by contact — native only (see the `mod online` cfg above).
    #[cfg(not(target_arch = "wasm32"))]
    registry.register(Box::new(online::OnlineCommand));
}
