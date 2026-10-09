//! C3c-3r — source lint for the Plan and economy actions a joined client is
//! refused (the arms need a GPU client; `test_game_harness` drives each one,
//! and this lint keeps the next edit from moving a change above its gate).
//!
//! Each refused arm checks `joined()` BEFORE its first inventory or world
//! change: the toast line has a `self.joined()` test just above it, and the
//! change comes after the toast (in the unrefused remainder of the arm).

fn game_loop_lines() -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("game_loop.rs");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("joiner-refusal lint: cannot read {} ({e})", path.display()));
    raw.lines().map(str::to_string).collect()
}

fn code_hits(lines: &[String], needle: &str) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.trim_start().starts_with("//") && l.contains(needle))
        .map(|(i, _)| i)
        .collect()
}

/// Does a line in `lines[from..to]` contain `needle`?
fn any_in(lines: &[String], from: usize, to: usize, needle: &str) -> bool {
    lines[from.min(lines.len())..to.min(lines.len())].iter().any(|l| l.contains(needle))
}

#[test]
fn each_refused_plan_arm_checks_joined_before_its_first_change() {
    let lines = game_loop_lines();
    // (name, toast constant, the arm's first change, where it must sit after the toast)
    let arms = [
        ("Plan Q-drop", "remote_mobs::JOINED_PLAN_DROP_TOAST", ".take_one_from_hotbar(slot_idx)"),
        ("lay a Plan flat", "remote_mobs::JOINED_PLAN_LAY_TOAST", "lay_blueprint_on_floor("),
        ("Blueprint peel", "remote_mobs::JOINED_BLUEPRINT_LIFT_TOAST", ".remove_face_attachment((pos[0], pos[1], pos[2]), fi)"),
        ("Auto build", "remote_mobs::JOINED_AUTO_BUILD_TOAST", "crate::plan::start_build("),
    ];
    for (name, toast, change) in arms {
        let t = code_hits(&lines, toast);
        assert_eq!(t.len(), 1, "{name}: expected one use of {toast} in game_loop.rs, found {t:?}");
        let t = t[0];
        assert!(
            any_in(&lines, t.saturating_sub(8), t, "self.joined()") || any_in(&lines, t.saturating_sub(8), t, "joined_blueprint"),
            "game_loop.rs:{}: {name} refuses without a joined() test just above",
            t + 1
        );
        let after: Vec<usize> = code_hits(&lines, change).into_iter().filter(|&c| c > t && c < t + 40).collect();
        assert!(
            !after.is_empty(),
            "game_loop.rs:{}: {name} — `{change}` must follow the refusal (within 40 lines), never come before it",
            t + 1
        );
        let before: Vec<usize> = code_hits(&lines, change).into_iter().filter(|&c| c < t && c + 12 > t).collect();
        assert!(before.is_empty(), "game_loop.rs:{}: {name} — `{change}` sits above its refusal", before.first().unwrap() + 1);
    }
    // `choose_auto_build` itself: the gate is its first statement.
    let f = code_hits(&lines, "fn choose_auto_build(");
    assert_eq!(f.len(), 1);
    assert!(lines[f[0] + 1].contains("if self.joined()"), "choose_auto_build must test joined() first");
}

/// C3c-3b (decision 4, replacing C3c-3r's joined-Blueprint rule) — a
/// joined client's break arms recover NO attachment: the server spills them
/// once and its stream takes them out of the client's copy. Both break arms
/// recover through the shared helper, behind a `!self.joined()` gate.
#[test]
fn both_break_arms_leave_a_joined_clients_attachments_to_the_server() {
    let lines = game_loop_lines();
    let hits = code_hits(&lines, "take_recoverable_attachments(");
    assert_eq!(hits.len(), 2, "the creative and the survival break arm each recover through the helper: {hits:?}");
    for h in hits {
        assert!(
            any_in(&lines, h.saturating_sub(2), h + 1, "!self.joined()"),
            "game_loop.rs:{}: a break arm's recovery must sit behind `!self.joined()`",
            h + 1
        );
    }
    assert!(
        code_hits(&lines, "recovered_item_for(&att)").is_empty(),
        "a break arm recovers attachments past the joined() rule"
    );
}

#[test]
fn every_economy_open_is_refused_for_a_joiner_first() {
    let lines = game_loop_lines();
    let toast = "remote_mobs::JOINED_ECONOMY_TOAST";
    let uses = code_hits(&lines, toast);
    assert!(uses.len() >= 9, "eight arms and the commission villager refuse with the shared toast: {uses:?}");
    for &t in &uses {
        assert!(any_in(&lines, t.saturating_sub(4), t, "self.joined()"), "game_loop.rs:{}: economy refusal without joined() above", t + 1);
    }
    // Every assignment that OPENS an economy screen has a refusal just above.
    let opens = [
        ".open_vendor = Some(",
        ".open_bounty_board =",
        ".open_tip_jar =",
        ".open_repair_bench =",
        ".open_market_hub =",
        ".open_auction =",
        ".open_bazaar =",
        ".open_commission_villager = Some(",
    ];
    for needle in opens {
        let sites: Vec<usize> = code_hits(&lines, needle)
            .into_iter()
            .filter(|&i| !lines[i].contains("==") && !lines[i].contains("= None"))
            .collect();
        assert!(!sites.is_empty(), "economy lint: `{needle}` not found in game_loop.rs");
        for a in sites {
            assert!(
                uses.iter().any(|&t| t < a && a - t <= 45),
                "game_loop.rs:{}: `{needle}` opens an economy screen with no joined-client refusal above it",
                a + 1
            );
        }
    }
}

#[test]
fn a_joined_client_runs_no_develop_or_auction_tick() {
    let lines = game_loop_lines();
    assert_eq!(
        code_hits(&lines, "let develops_here = !self.joined() && self.sim_runs(SimSystem::Develop);").len(),
        1,
        "the develop gate is `!self.joined()` and the lend table"
    );
    for needle in ["latent_print::tick_develop(", "latent_print::tick_develop_attachments(", "auction::tick_auctions("] {
        let hits = code_hits(&lines, needle);
        assert_eq!(hits.len(), 1, "`{needle}` expected once in game_loop.rs, found {hits:?}");
        // C3c-3b — the develop ticks sit under `develops_here`, which is
        // `!self.joined()` and the lend table's `SimSystem::Develop`.
        assert!(
            any_in(&lines, hits[0].saturating_sub(6), hits[0], "self.joined()")
                || any_in(&lines, hits[0].saturating_sub(6), hits[0], "develops_here"),
            "game_loop.rs:{}: `{needle}` must not run on a joined client",
            hits[0] + 1
        );
    }
}
