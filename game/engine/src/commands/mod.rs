//! Engine command system — chat-driven slash commands.
//!
//! Plugin-shaped: registry + parser + dispatcher are game-agnostic. Built-in
//! commands (in `builtins::`) are voxel/Minecraft-style; another game can
//! register its own without touching this module.
//!
//! Spec: docs/foundations/2026-05-07-engine-commands.md

pub mod parser;
pub mod registry;
pub mod dispatch;
pub mod builtins;

// Short-path re-exports used by consumers (game_loop, chat_ui, main).
// Items not re-exported here are still reachable via their submodule path,
// e.g. `commands::parser::parse`.
pub use registry::{CommandRegistry, OpLevel};
pub use dispatch::{CommandContext, CommandResult, ChatLine, ChatLineKind, dispatch};
