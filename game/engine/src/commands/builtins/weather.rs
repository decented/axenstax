//! `/weather <clear|rain|storm|thunder> [seconds]` — set the sky.
//!
//! The command only parses and refuses what it can see (the Workshop); the
//! weather itself is the authority's, so it hands the game loop a
//! [`CommandResult::SetWeather`] and the loop writes the window through
//! `GameState::write_weather`, the way `/time` reaches the clock through
//! `write_clock`. The game loop also refuses while a trial's `weather_lock`
//! holds the sky, which no command can see. A cheat, like `/time set`.

use crate::commands::complete::{matching, CompletionContext};
use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::Command;
use crate::weather::{Sky, WeatherRequest, MAX_COMMAND_SECONDS};

/// Every word `/weather` accepts, with its sky. `execute` and Tab completion
/// both read this, so a word cannot be accepted but not offered.
const SKIES: &[(&str, Sky)] = &[
    ("clear", Sky::Clear),
    ("rain", Sky::Rain),
    ("storm", Sky::Storm),
    // The Minecraft word for a storm.
    ("thunder", Sky::Storm),
];

const USAGE: &str = "/weather <clear|rain|storm|thunder> [seconds]";

/// Parse the words after `/weather`. Pure, so every form and error is a unit
/// test. The error is the line to show the player.
pub(crate) fn parse_request(args: &[String]) -> Result<WeatherRequest, String> {
    let usage = || format!("usage: {USAGE}");
    let Some(word) = args.first() else { return Err(usage()) };
    let lower = word.to_lowercase();
    let Some((_, sky)) = SKIES.iter().find(|(name, _)| *name == lower) else {
        return Err(format!("not a sky: '{word}'. {}", usage()));
    };
    if args.len() > 2 {
        return Err(usage());
    }
    let (seconds, capped_from) = match args.get(1) {
        None => (sky.default_seconds(), None),
        Some(s) => match s.parse::<i128>() {
            Ok(n) if n >= 1 && n <= i128::from(MAX_COMMAND_SECONDS) => (n as u32, None),
            Ok(n) if n > i128::from(MAX_COMMAND_SECONDS) => {
                (MAX_COMMAND_SECONDS as u32, Some(u64::try_from(n).unwrap_or(u64::MAX)))
            }
            Ok(_) => return Err(format!("seconds must be 1 to {MAX_COMMAND_SECONDS}. {}", usage())),
            // Digits too many for any integer: certainly more than an hour.
            Err(_) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => {
                (MAX_COMMAND_SECONDS as u32, Some(u64::MAX))
            }
            Err(_) => return Err(format!("'{s}' is not a number of seconds. {}", usage())),
        },
    };
    Ok(WeatherRequest { sky: *sky, seconds, capped_from })
}

pub struct WeatherCommand;

impl Command for WeatherCommand {
    fn name(&self) -> &'static str {
        "weather"
    }
    fn help(&self) -> &'static str {
        "Set the weather: clear, rain or storm, for a while"
    }
    fn usage(&self) -> &'static str {
        USAGE
    }
    fn complete(&self, args_before: &[&str], partial: &str, _ctx: &CompletionContext) -> Vec<String> {
        match args_before {
            [] => matching(SKIES.iter().map(|(name, _)| name), partial),
            _ => Vec::new(),
        }
    }
    // A cheat by default (the trait's default, as `/time`): the game loop's
    // refusal (a trial's weather lock) takes the mark back.
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        let request = match parse_request(args) {
            Ok(r) => r,
            Err(msg) => {
                ctx.error(msg.clone());
                return CommandResult::Error(msg);
            }
        };
        if let Some(why) = crate::weather::command_refusal(
            crate::weather::world_has_weather(ctx.world.is_workshop),
            None,
        ) {
            ctx.error(why);
            return CommandResult::Error(why.to_string());
        }
        CommandResult::SetWeather(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::OpLevel;
    use crate::commands::test_support::run_named;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn req(sky: Sky, seconds: u32, capped_from: Option<u64>) -> Result<WeatherRequest, String> {
        Ok(WeatherRequest { sky, seconds, capped_from })
    }

    #[test]
    fn every_form_parses_with_its_default_duration() {
        assert_eq!(parse_request(&args(&["clear"])), req(Sky::Clear, 1200, None), "clear: 20 minutes");
        assert_eq!(parse_request(&args(&["rain"])), req(Sky::Rain, 300, None), "rain: 5 minutes");
        assert_eq!(parse_request(&args(&["storm"])), req(Sky::Storm, 300, None), "storm: 5 minutes");
        assert_eq!(parse_request(&args(&["thunder"])), req(Sky::Storm, 300, None), "thunder is storm");
        assert_eq!(parse_request(&args(&["CLEAR"])), req(Sky::Clear, 1200, None), "case-insensitive");
        // 20 minutes and 5 minutes in ticks (20 per second).
        assert_eq!(1200 * 20, 24_000);
        assert_eq!(300 * 20, 6_000);
    }

    #[test]
    fn a_number_is_the_seconds() {
        assert_eq!(parse_request(&args(&["clear", "600"])), req(Sky::Clear, 600, None));
        assert_eq!(parse_request(&args(&["rain", "1"])), req(Sky::Rain, 1, None));
        assert_eq!(parse_request(&args(&["thunder", "3600"])), req(Sky::Storm, 3600, None));
    }

    #[test]
    fn more_than_an_hour_is_capped_and_says_so() {
        assert_eq!(parse_request(&args(&["rain", "3601"])), req(Sky::Rain, 3600, Some(3601)));
        assert_eq!(parse_request(&args(&["clear", "99999"])), req(Sky::Clear, 3600, Some(99_999)));
        let huge = "9".repeat(40);
        assert_eq!(parse_request(&args(&["clear", &huge])), req(Sky::Clear, 3600, Some(u64::MAX)));
        let said = crate::weather::command_reply(parse_request(&args(&["rain", "7200"])).unwrap());
        assert!(said.contains("capped from 7200"), "{said}");
    }

    #[test]
    fn zero_negative_and_non_numbers_are_errors_with_the_usage_line() {
        for bad in [&["clear", "0"][..], &["clear", "-5"], &["rain", "soon"], &["rain", "1.5"], &["rain", ""]] {
            let e = parse_request(&args(bad)).unwrap_err();
            assert!(e.contains(USAGE), "{bad:?}: {e}");
        }
        for bad in [&[][..], &["sunny"], &["clear", "10", "20"]] {
            let e = parse_request(&args(bad)).unwrap_err();
            assert!(e.contains(USAGE), "{bad:?}: {e}");
        }
    }

    /// Dispatch `line` through the registry at `op`, returning the result and
    /// whether the World Integrity Ledger was marked.
    fn dispatch_line(line: &str, op: OpLevel) -> (CommandResult, bool, Vec<String>) {
        let reg = crate::commands::test_support::full_registry();
        let mut world = crate::world::World::new();
        let (mut t, mut s, mut creative) = (0u32, 1u32, false);
        let mut mode = crate::play_mode::PlayMode::Survival;
        let mut players = vec![crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        let (mut log, mut c, mut e, mut p) = (Vec::new(), false, false, false);
        let mut ctx = CommandContext {
            world: &mut world,
            world_time: &mut t,
            world_time_step: &mut s,
            is_creative: &mut creative,
            play_mode: &mut mode,
            seed: 1,
            world_name: "w",
            players: &mut players,
            player_idx: 0,
            op_level: op,
            current_tick: 0,
            log: &mut log,
            registry: &reg,
            cheats_used_marker: &mut c,
            ever_creative_marker: &mut e,
            pure_survival_broken_marker: &mut p,
        };
        let r = crate::commands::dispatch(line, &mut ctx, &reg);
        let lines = log.iter().map(|l| l.text.clone()).collect();
        (r, c, lines)
    }

    #[test]
    fn it_is_a_cheat_like_time_set_and_an_error_marks_nothing() {
        let (r, marked, _) = dispatch_line("/weather clear 600", OpLevel::Op);
        assert!(matches!(r, CommandResult::SetWeather(_)), "{r:?}");
        assert!(marked, "a successful /weather marks the World Integrity Ledger");
        for bad in ["/weather", "/weather sunny", "/weather rain 0", "/weather rain soon"] {
            let (r, marked, _) = dispatch_line(bad, OpLevel::Op);
            assert!(matches!(r, CommandResult::Error(_)), "{bad}: {r:?}");
            assert!(!marked, "{bad}: an error is no cheat");
        }
        let cmd = crate::commands::test_support::full_registry();
        assert!(cmd.lookup("weather").unwrap().is_cheat());
    }

    #[test]
    fn a_joiner_gets_permission_denied_exactly_as_for_time_set() {
        let (w, marked, _) = dispatch_line("/weather storm", OpLevel::None);
        let (t, _, _) = dispatch_line("/time set night", OpLevel::None);
        assert_eq!(w, CommandResult::Error("permission denied: /weather".into()));
        assert_eq!(t, CommandResult::Error("permission denied: /time".into()));
        assert!(!marked);
    }

    #[test]
    fn help_lists_it_and_describes_it() {
        let (_, _, lines) = dispatch_line("/help", OpLevel::Op);
        assert!(lines.iter().any(|l| l.contains("/weather")), "{lines:?}");
        let (_, _, lines) = dispatch_line("/help weather", OpLevel::Op);
        assert!(lines.iter().any(|l| l.contains(USAGE)), "{lines:?}");
    }

    #[test]
    fn the_command_returns_the_request_and_refuses_the_workshop() {
        let (r, _) = run_named("weather", &["storm", "60"], OpLevel::Op);
        assert_eq!(r, CommandResult::SetWeather(WeatherRequest { sky: Sky::Storm, seconds: 60, capped_from: None }));
        let (r, lines) = run_named("weather", &["rain", "0"], OpLevel::Op);
        assert!(matches!(r, CommandResult::Error(ref m) if m.contains(USAGE)), "{r:?}");
        assert!(lines.iter().any(|l| l.contains(USAGE)));
        // The Workshop never has weather.
        let reg = crate::commands::test_support::full_registry();
        let cmd = reg.lookup("weather").unwrap();
        let mut world = crate::world::World::new();
        world.is_workshop = true;
        let (mut t, mut s, mut creative) = (0u32, 1u32, false);
        let mut mode = crate::play_mode::PlayMode::Survival;
        let mut players = vec![crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        let (mut log, mut c, mut e, mut p) = (Vec::new(), false, false, false);
        let mut ctx = CommandContext {
            world: &mut world,
            world_time: &mut t,
            world_time_step: &mut s,
            is_creative: &mut creative,
            play_mode: &mut mode,
            seed: 1,
            world_name: "ws",
            players: &mut players,
            player_idx: 0,
            op_level: OpLevel::Op,
            current_tick: 0,
            log: &mut log,
            registry: &reg,
            cheats_used_marker: &mut c,
            ever_creative_marker: &mut e,
            pure_survival_broken_marker: &mut p,
        };
        let r = cmd.execute(&mut ctx, &args(&["clear"]));
        assert_eq!(r, CommandResult::Error("The Workshop has no weather".into()));
    }
}
