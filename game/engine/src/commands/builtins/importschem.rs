//! `/import <path>` (#10) — import a Minecraft `.schem` (Sponge) schematic from
//! a file into the plan registry, ready to use with `/buildguide <name>`. The
//! plan name is the file stem. Native-only (reads the filesystem); the in-page
//! file-picker for WASM is a follow-up.
//!
//! Builder aid (`OpLevel::None`, not a cheat) — it only adds a blueprint.

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

pub struct ImportSchemCommand;

impl Command for ImportSchemCommand {
    fn name(&self) -> &'static str {
        "import"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["importschem"]
    }
    fn help(&self) -> &'static str {
        "Import a Minecraft .schem into the blueprint registry"
    }
    fn usage(&self) -> &'static str {
        "/import <path-to.schem>"
    }
    fn min_op_level(&self) -> OpLevel {
        OpLevel::None
    }

    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        if args.is_empty() {
            let m = "usage: /import <path-to.schem>".to_string();
            ctx.error(m.clone());
            return CommandResult::Error(m);
        }
        let path = args.join(" ");

        #[cfg(target_arch = "wasm32")]
        {
            let _ = path;
            let m = "Schematic import is native-only for now.".to_string();
            ctx.error(m.clone());
            CommandResult::Error(m)
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    let m = format!("Can't read '{path}': {e}");
                    ctx.error(m.clone());
                    return CommandResult::Error(m);
                }
            };
            let name = std::path::Path::new(&path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "imported".to_string());
            match crate::schematic::parse_schem(&bytes, name.clone()) {
                Ok(plan) => {
                    let cells = plan.cells.len();
                    ctx.world.plan_registry.add(plan);
                    ctx.success(format!(
                        "Imported '{name}' ({cells} blocks). Use /buildguide {name}"
                    ));
                    CommandResult::Success
                }
                Err(e) => {
                    let m = format!("Import failed: {e}");
                    ctx.error(m.clone());
                    CommandResult::Error(m)
                }
            }
        }
    }
}
