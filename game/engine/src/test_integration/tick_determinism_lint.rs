//! Keep slice 1 — permanent guard: no wall clock and no OS randomness in the
//! authority's tick paths (Spec 01 §4.1.4 "Tick determinism"). A tick must be a
//! function of the world, the tick counter, the seed and the events it was
//! fed; `Instant`, `SystemTime` or `getrandom` in it makes two runs of the
//! same input part (the twin test, `twin_run.rs`, would see that only if its
//! fixture happened to reach the line).
//!
//! Scans the production code (`#[cfg(test)]` items skipped, `//` comments
//! stripped) of `server.rs`, every `sim_*` module and the modules
//! `GameServer::tick` calls into, for the words `Instant`, `SystemTime` and
//! `getrandom` (`web_time::Instant` included). The only sanctioned uses are in
//! [`ALLOWED`]; each needs a comment saying why it cannot change an outcome.
//! An allowlist entry that no longer matches exactly one line fails too, so
//! the list can't go stale.

/// The scanned files, relative to `src/`. `sim_*.rs` is added by listing the
/// directory, so a new sim phase is covered without editing this.
const TICK_PATH_FILES: &[&str] = &[
    "server.rs",
    "server_stream.rs",
    "server_death.rs",
    "server_machines.rs",
    "block_machines.rs",
    "fire.rs",
    "water.rs",
    "lava.rs",
    "fluids.rs",
    "falling_blocks.rs",
    "leaf_decay.rs",
    "hopper.rs",
    "spawning.rs",
    "mob_ai.rs",
    "species_ai.rs",
    "breeding.rs",
    "tether.rs",
    "entity.rs",
    "combat.rs",
    "explosion.rs",
    "power.rs",
    "cart.rs",
    "furnace.rs",
    "campfire.rs",
    "piston.rs",
    "growth.rs",
    "dispenser.rs",
    "composter.rs",
    "drying_rack.rs",
    "bee_hive.rs",
    "snowfall.rs",
    "rubber.rs",
    "salt_lick.rs",
    "bounty.rs",
    "brigand_hideout_gen.rs",
    "latent_print.rs",
    "weather.rs",
    "wind.rs",
    "reputation_day.rs",
];

/// `(file, text the line contains)` — the sanctioned uses. Each is drawn once
/// when a server is built, never inside a tick, and is an identity compared
/// for equality, never a roll that picks an outcome.
const ALLOWED: &[(&str, &str)] = &[
    // `GameServer::new` — the run id, so a joiner can't sleep twice in one
    // night across a restart (`run_id` gate) and a lent day offer is matched
    // to its run. Compared, never rolled. Twin tests derive it from the seed
    // (`TestHost::start_seeded`).
    ("server.rs", "run_id: { let mut id = [0; 16]; getrandom::fill"),
    // `DayClock::new` — the reputation clock's owner id, matched against day
    // pulses (`DayConsumer::consume`). Compared, never rolled.
    ("reputation_day.rs", "getrandom::fill(&mut owner)"),
];

const FORBIDDEN: &[&str] = &["Instant", "SystemTime", "getrandom"];

/// Does `line` use a forbidden word (as a whole identifier)?
fn forbidden_word(line: &str) -> Option<&'static str> {
    for &word in FORBIDDEN {
        let mut from = 0;
        while let Some(i) = line[from..].find(word) {
            let start = from + i;
            let end = start + word.len();
            let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
            let before = line[..start].chars().next_back().is_some_and(ident);
            let after = line[end..].chars().next().is_some_and(ident);
            if !before && !after {
                return Some(word);
            }
            from = end;
        }
    }
    None
}

/// The production lines of a Rust source: `//` comments stripped, and every
/// item under `#[cfg(test)]` / `#[cfg(all(test, ...))]` skipped (a one-line
/// item to its `;`, a block item to its matching `}`). Yields
/// `(1-based line number, code)`.
fn production_lines(src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut lines = src.lines().enumerate().peekable();
    while let Some((n, raw)) = lines.next() {
        let code = raw.split("//").next().unwrap_or("").to_string();
        let t = code.trim_start();
        if t.starts_with("#[cfg(test)]") || t.starts_with("#[cfg(all(test") {
            // Skip the attributed item: further attributes, then either a
            // `;`-terminated line or a brace-balanced block.
            let mut depth: i64 = 0;
            let mut opened = false;
            for (_, raw) in lines.by_ref() {
                let code = raw.split("//").next().unwrap_or("");
                let t = code.trim_start();
                if !opened && t.starts_with("#[") {
                    continue;
                }
                for c in code.chars() {
                    match c {
                        '{' => {
                            depth += 1;
                            opened = true;
                        }
                        '}' => depth -= 1,
                        _ => {}
                    }
                }
                if (opened && depth <= 0) || (!opened && code.trim_end().ends_with(';')) {
                    break;
                }
            }
            continue;
        }
        out.push((n + 1, code));
    }
    out
}

/// Every forbidden use in `src` (the file `name`) that no [`ALLOWED`] entry
/// covers, as messages; and how many lines each allow entry matched.
fn scan(name: &str, src: &str, allow_hits: &mut [usize]) -> Vec<String> {
    let mut bad = Vec::new();
    for (n, code) in production_lines(src) {
        let Some(word) = forbidden_word(&code) else { continue };
        let allowed = ALLOWED
            .iter()
            .position(|(file, needle)| *file == name && code.contains(needle));
        match allowed {
            Some(i) => allow_hits[i] += 1,
            None => bad.push(format!("{name}:{n}: `{word}` in a tick path: {}", code.trim())),
        }
    }
    bad
}

fn src_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

#[test]
fn no_wall_clock_or_os_randomness_in_tick_paths() {
    let dir = src_dir();
    let mut files: Vec<String> = TICK_PATH_FILES.iter().map(|s| s.to_string()).collect();
    for entry in std::fs::read_dir(&dir).expect("read src/") {
        let name = entry.expect("dir entry").file_name().to_string_lossy().into_owned();
        // `sim_*_tests.rs` are `#[cfg(test)] #[path]` test modules.
        if name.starts_with("sim_") && name.ends_with(".rs") && !name.ends_with("_tests.rs") {
            files.push(name);
        }
    }
    files.sort();
    files.dedup();
    let mut allow_hits = vec![0usize; ALLOWED.len()];
    let mut bad = Vec::new();
    for name in &files {
        let src = std::fs::read_to_string(dir.join(name))
            .unwrap_or_else(|e| panic!("tick-path file {name} unreadable: {e}"));
        bad.extend(scan(name, &src, &mut allow_hits));
    }
    assert!(
        bad.is_empty(),
        "wall clock / OS randomness in a tick path (Spec 01 §4.1.4 tick determinism). \
         Derive it from the tick counter, the seed and positions instead, or — if it can \
         never change an outcome — add it to ALLOWED with the reason:\n{}",
        bad.join("\n")
    );
    for (i, hits) in allow_hits.iter().enumerate() {
        assert_eq!(
            *hits, 1,
            "ALLOWED entry {:?} matched {hits} lines, not 1: update or remove it",
            ALLOWED[i]
        );
    }
}

/// The scanner itself: a new use is caught, comments and test items are not.
#[test]
fn the_tick_lint_catches_a_new_clock_and_skips_tests_and_comments() {
    let src = "\
fn tick(&mut self) {
    // Instant::now() in a comment is fine
    let t = std::time::Instant::now();
}
#[cfg(test)]
mod tests {
    fn bench() { let t = std::time::Instant::now(); }
}
fn after() { let mut b = [0u8; 4]; getrandom::fill(&mut b).unwrap(); }
fn instantly() { let Instantaneous = 1; }
";
    let mut hits = vec![0usize; ALLOWED.len()];
    let bad = scan("server.rs", src, &mut hits);
    assert_eq!(bad.len(), 2, "{bad:?}");
    assert!(bad[0].starts_with("server.rs:3:"), "{bad:?}");
    assert!(bad[1].starts_with("server.rs:9:"), "{bad:?}");
}
