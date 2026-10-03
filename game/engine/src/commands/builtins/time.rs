use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::Command;

/// Map a `/time set <keyword>` word to a world-time tick.
///
/// The engine clock (`camera.rs::compute_sun`) runs `0 = midnight,
/// 6000 = sunrise, 12000 = noon, 18000 = sunset` — i.e. offset +6000
/// from Minecraft's clock (where 0 = sunrise). The original constants
/// here were copied straight from Minecraft (`day = 1000`, `night =
/// 13000`), which under this engine's clock lands "day" near midnight
/// and "night" just after noon — the 2026-05-30 day↔night-swap
/// playtest bug. Values below are Minecraft semantics shifted +6000
/// (mod 24000) so the words mean what the player expects.
pub(crate) fn time_for_keyword(kw: &str) -> Option<u32> {
    Some(match kw {
        // Minecraft "day" (1000, bright morning) + 6000.
        "day" | "sunrise" | "dawn" => 7000,
        // Sun at its peak.
        "noon" => 12000,
        // Minecraft "night" (13000, after dusk) + 6000 → properly dark.
        "night" | "sunset" | "dusk" => 19000,
        // Darkest point.
        "midnight" => 0,
        _ => return None,
    })
}

pub struct TimeCommand;

impl Command for TimeCommand {
    fn name(&self) -> &'static str {
        "time"
    }
    fn help(&self) -> &'static str {
        "Get/set world time, or change day-length speed"
    }
    fn usage(&self) -> &'static str {
        "/time get | /time set <day|noon|night|midnight|0..23999> | /time speed <1..64>"
    }
    fn is_cheat(&self) -> bool {
        // get is informational; set / speed are cheats. We override on a
        // per-execution basis rather than per-command so /time get is honest.
        true
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        let sub = args.first().map(|s| s.to_lowercase());
        match sub.as_deref() {
            Some("get") | None => {
                // Read-only. We return Silent so dispatch.rs's
                // `is_cheat() && Success` gate never fires for `/time get`,
                // even though `is_cheat()` returns true at the command level
                // (cheat semantics are per-subcommand for /time).
                let total = *ctx.world_time;
                let hours = (total % 24000) * 24 / 24000;
                let minutes = ((total % 24000) * 24 % 24000) * 60 / 24000;
                ctx.success(format!(
                    "Time: {hours:02}:{minutes:02} (tick {total}), speed {}x",
                    *ctx.world_time_step
                ));
                CommandResult::Silent
            }
            Some("set") => {
                let target = match args.get(1) {
                    Some(s) => s,
                    None => {
                        let msg = "usage: /time set <day|noon|night|midnight|0..23999>".to_string();
                        ctx.error(msg.clone());
                        return CommandResult::Error(msg);
                    }
                };
                let lower = target.to_lowercase();
                let new_time = match time_for_keyword(&lower) {
                    Some(t) => t,
                    None => match lower.parse::<u32>() {
                        Ok(n) if n < 24000 => n,
                        Ok(_) => {
                            let msg = "tick must be in 0..24000".to_string();
                            ctx.error(msg.clone());
                            return CommandResult::Error(msg);
                        }
                        Err(_) => {
                            let msg = format!("can't parse '{lower}' as a time");
                            ctx.error(msg.clone());
                            return CommandResult::Error(msg);
                        }
                    },
                };
                *ctx.world_time = new_time;
                ctx.success(format!("Time set to {new_time}"));
                CommandResult::Success
            }
            Some("speed") => {
                let target = match args.get(1) {
                    Some(s) => s,
                    None => {
                        let msg = "usage: /time speed <1..64>".to_string();
                        ctx.error(msg.clone());
                        return CommandResult::Error(msg);
                    }
                };
                match target.parse::<u32>() {
                    Ok(n) if (1..=64).contains(&n) => {
                        *ctx.world_time_step = n;
                        // Day length in seconds: 24000 ticks of world_time per
                        // day ÷ (n per game tick × 20 game ticks/sec).
                        let secs = (24000 / (n * 20)).max(1);
                        ctx.success(format!(
                            "Time speed set to {n}x (day length ≈ {secs}s)"
                        ));
                        CommandResult::Success
                    }
                    Ok(_) => {
                        let msg = "speed must be 1..64".to_string();
                        ctx.error(msg.clone());
                        CommandResult::Error(msg)
                    }
                    Err(_) => {
                        let msg = format!("can't parse '{target}' as a number");
                        ctx.error(msg.clone());
                        CommandResult::Error(msg)
                    }
                }
            }
            Some(other) => {
                let msg = format!("unknown subcommand: {other}");
                ctx.error(msg.clone());
                CommandResult::Error(msg)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::player_slot::PlayerSlot;

    fn run(args: &[&str], world_time: &mut u32, step: &mut u32) -> (CommandResult, Vec<String>) {
        let cmd = TimeCommand;
        let mut world = World::new();
        let mut creative = false;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        let mut log = Vec::new();
        let mut ch = false;
        let mut ev = false;
        let mut ps = false;
        let reg = CommandRegistry::new();
        let mut ctx = CommandContext {
            world: &mut world,
            world_time,
            world_time_step: step,
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
        let r = cmd.execute(&mut ctx, &owned);
        let lines: Vec<String> = log.iter().map(|l| l.text.clone()).collect();
        (r, lines)
    }

    #[test]
    fn get_returns_silent() {
        let mut t = 6000;
        let mut s = 4;
        let (r, lines) = run(&["get"], &mut t, &mut s);
        assert_eq!(r, CommandResult::Silent);
        assert!(lines.iter().any(|l| l.contains("tick 6000")));
    }

    // Engine clock convention (camera.rs::compute_sun): 0 = midnight,
    // 6000 = sunrise, 12000 = noon, 18000 = sunset. The sun is above the
    // horizon (daylight) for ticks in 6000..=18000 and below it (dark)
    // otherwise. These tests assert *behaviour* (which half of the clock)
    // rather than a literal constant, so they can't silently re-encode a
    // swapped-constant bug the way the old `== 1000` / `== 13000`
    // assertions did (the 2026-05-30 day↔night-swap playtest bug).
    fn is_daylight(t: u32) -> bool {
        (6000..=18000).contains(&(t % 24000))
    }

    #[test]
    fn set_keyword_day_is_light() {
        let mut t = 0;
        let mut s = 4;
        let (r, _) = run(&["set", "day"], &mut t, &mut s);
        assert_eq!(r, CommandResult::Success);
        assert!(is_daylight(t), "/time set day should land in daylight, got {t}");
    }

    #[test]
    fn set_keyword_night_is_dark() {
        let mut t = 0;
        let mut s = 4;
        let (r, _) = run(&["set", "night"], &mut t, &mut s);
        assert_eq!(r, CommandResult::Success);
        assert!(!is_daylight(t), "/time set night should land in darkness, got {t}");
    }

    #[test]
    fn keyword_table_matches_clock_convention() {
        assert!(is_daylight(time_for_keyword("day").unwrap()));
        assert_eq!(time_for_keyword("noon"), Some(12000));
        assert!(!is_daylight(time_for_keyword("night").unwrap()));
        assert_eq!(time_for_keyword("midnight"), Some(0));
        assert_eq!(time_for_keyword("banana"), None);
    }

    #[test]
    fn set_numeric() {
        let mut t = 0;
        let mut s = 4;
        let (r, _) = run(&["set", "8500"], &mut t, &mut s);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(t, 8500);
    }

    #[test]
    fn set_out_of_range() {
        let mut t = 0;
        let mut s = 4;
        let (r, _) = run(&["set", "99999"], &mut t, &mut s);
        assert!(matches!(r, CommandResult::Error(_)));
        assert_eq!(t, 0);
    }

    #[test]
    fn set_garbage() {
        let mut t = 0;
        let mut s = 4;
        let (r, _) = run(&["set", "banana"], &mut t, &mut s);
        assert!(matches!(r, CommandResult::Error(_)));
    }

    #[test]
    fn speed_valid() {
        let mut t = 0;
        let mut s = 4;
        let (r, lines) = run(&["speed", "1"], &mut t, &mut s);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(s, 1);
        // 1× speed = 24000 / (1 × 20) = 1200 seconds = 20 minutes per day.
        // The displayed unit must match the value: previously we printed
        // seconds but labelled them "min", which would be off by 60×.
        assert!(
            lines.iter().any(|l| l.contains("1200s")),
            "expected '1200s' in success message, got {lines:?}"
        );
    }

    #[test]
    fn speed_4x_reports_5_min_day() {
        let mut t = 0;
        let mut s = 1;
        let (r, lines) = run(&["speed", "4"], &mut t, &mut s);
        assert_eq!(r, CommandResult::Success);
        // 24000 / (4 × 20) = 300s = the alpha 5-minute day.
        assert!(
            lines.iter().any(|l| l.contains("300s")),
            "expected '300s' in success message, got {lines:?}"
        );
    }

    #[test]
    fn speed_zero_rejected() {
        let mut t = 0;
        let mut s = 4;
        let (r, _) = run(&["speed", "0"], &mut t, &mut s);
        assert!(matches!(r, CommandResult::Error(_)));
        assert_eq!(s, 4);
    }

    #[test]
    fn speed_too_high_rejected() {
        let mut t = 0;
        let mut s = 4;
        let (r, _) = run(&["speed", "999"], &mut t, &mut s);
        assert!(matches!(r, CommandResult::Error(_)));
    }

    #[test]
    fn unknown_subcommand_rejected() {
        let mut t = 0;
        let mut s = 4;
        let (r, _) = run(&["banana"], &mut t, &mut s);
        assert!(matches!(r, CommandResult::Error(_)));
    }

    #[test]
    fn no_args_defaults_to_get() {
        let mut t = 9000;
        let mut s = 1;
        let (r, lines) = run(&[], &mut t, &mut s);
        assert_eq!(r, CommandResult::Silent);
        assert!(lines.iter().any(|l| l.contains("tick 9000")));
    }
}
