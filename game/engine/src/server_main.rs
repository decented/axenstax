//! Headless dedicated-server entry point (`--server`).
//!
//! Runs the authoritative `GameServer` simulation + a WebSocket accept loop with
//! **no window, no renderer, no GPU** — so it runs on a bare Docker host. Both
//! the browser PWA (via the Caddy `wss://…/ws` front) and the native client
//! (`ws://host:6767`) join the same world.
//!
//! Config comes from env vars (Docker-friendly) with `--flag value` CLI
//! overrides. The world lives under `AXENSTAX_WORLDS_DIR` (a mounted volume) and
//! autosaves on a timer + on graceful shutdown (SIGINT/SIGTERM).
//!
//! Access: **sign-in is required by default** (owner decision 2026-10-06, O-7
//! #3) — only players with a verified Signet identity may join. `--allow-guests`
//! (bare, or `--allow-guests 1`) or `AXENSTAX_ALLOW_GUESTS=1` also admits
//! anonymous guests; `--allow-guests 0` / `=false` does not. The retired
//! `--require-signin <v>` / `AXENSTAX_REQUIRE_SIGNIN` are accepted and ignored
//! (logged once at boot) so existing scripts keep starting. See
//! [`load_access_policy`].

#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::ws_transport::DEFAULT_WS_PORT;

/// Resolved dedicated-server configuration.
struct ServerConfig {
    world: String,
    /// Explicit seed override (else the world meta's seed, random for a new world).
    seed: Option<u32>,
    game_mode: String,
    /// Terrain preset for a freshly-bootstrapped world: "normal" (seeded biome
    /// terrain) or "flat" (blank canvas). Stamped onto WorldMeta at creation;
    /// the GameServer reads it back from the meta (`server.rs`). Publish flow P1
    /// (`docs/superpowers/specs/2026-06-22-publish-flow-build-spec.md` §5.5).
    world_type: String,
    /// Ground block for a flat world ("grass"/"sand"/"stone"/"dirt"/"snow"/
    /// "water"/"none"). Ignored when `world_type == "normal"`.
    ground: String,
    /// Water-layer depth for a flat "water" world (blocks). Ignored otherwise.
    water_depth: u8,
    max_players: usize,
    ws_port: u16,
    server_name: String,
    autosave_secs: u64,
}

/// `--key value` CLI lookup (overrides env). Returns the value following `--key`.
fn cli_value(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// CLI override → env var → default. Empty strings are treated as unset.
fn resolve(args: &[String], cli_key: &str, env_key: &str, default: &str) -> String {
    cli_value(args, cli_key)
        .or_else(|| std::env::var(env_key).ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn parse_config(args: &[String]) -> ServerConfig {
    let world = resolve(args, "--world", "AXENSTAX_WORLD", "server-world");
    let seed = resolve(args, "--seed", "AXENSTAX_SEED", "").parse::<u32>().ok();
    let game_mode =
        resolve(args, "--gamemode", "AXENSTAX_GAMEMODE", "survival").to_lowercase();
    let world_type =
        resolve(args, "--world-type", "AXENSTAX_WORLD_TYPE", "normal").to_lowercase();
    let ground = resolve(args, "--ground", "AXENSTAX_GROUND", "grass").to_lowercase();
    let water_depth = resolve(args, "--water-depth", "AXENSTAX_WATER_DEPTH", "3")
        .parse::<u8>()
        .unwrap_or(3);
    let max_players = resolve(args, "--max-players", "AXENSTAX_MAX_PLAYERS", "8")
        .parse::<usize>()
        .unwrap_or(8)
        .clamp(1, 64);
    let ws_port = resolve(args, "--port", "AXENSTAX_WS_PORT", &DEFAULT_WS_PORT.to_string())
        .parse::<u16>()
        .unwrap_or(DEFAULT_WS_PORT);
    let server_name = resolve(args, "--name", "AXENSTAX_SERVER_NAME", "Axe'n'Stax Server");
    let autosave_secs = resolve(args, "--autosave", "AXENSTAX_AUTOSAVE_SECS", "60")
        .parse::<u64>()
        .unwrap_or(60)
        .max(5);

    ServerConfig {
        world,
        seed,
        game_mode,
        world_type,
        ground,
        water_depth,
        max_players,
        ws_port,
        server_name,
        autosave_secs,
    }
}

/// The server identity directory (`<worlds>/.identity`).
fn id_dir() -> std::path::PathBuf {
    crate::server_identity::identity_dir(crate::save::worlds_root().to_string_lossy().as_ref())
}

/// Boot-time classification of the server's identity, for the startup banner.
#[derive(Debug, PartialEq, Eq)]
enum BootIdentity {
    /// No runtime key, or no attestation — runs anonymously (additive).
    Anonymous,
    /// Attestation present but not currently in its validity window.
    Expired,
    /// A valid attestation; carries the operator npub.
    Verified(String),
}

fn classify_boot_identity(
    id: Option<&crate::server_identity::ServerIdentity>,
    now: nostr::Timestamp,
) -> BootIdentity {
    match id {
        Some(i) if i.is_verified(now) => {
            BootIdentity::Verified(i.operator_npub().unwrap_or_default())
        }
        Some(i) if i.attestation().is_some() => BootIdentity::Expired,
        _ => BootIdentity::Anonymous,
    }
}

/// `true` if the env/CLI flag asks the server to refuse booting unverified.
fn require_verified_flag(args: &[String]) -> bool {
    let v = resolve(args, "--require-verified", "AXENSTAX_REQUIRE_VERIFIED", "0");
    v == "1" || v.eq_ignore_ascii_case("true")
}

/// Build a single-threaded tokio runtime for the one-shot CLI signing flows.
fn cli_runtime() -> tokio::runtime::Runtime {
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("could not start async runtime: {e}");
            std::process::exit(1);
        }
    }
}

fn public_host(args: &[String]) -> Option<String> {
    cli_value(args, "--public-host")
        .or_else(|| std::env::var("AXENSTAX_PUBLIC_HOST").ok())
        .filter(|s| !s.is_empty())
}

fn delegation_days(args: &[String]) -> u64 {
    resolve(
        args,
        "--delegation-days",
        "AXENSTAX_DELEGATION_DAYS",
        &crate::server_identity::DEFAULT_DELEGATION_DAYS.to_string(),
    )
    .parse::<u64>()
    .unwrap_or(crate::server_identity::DEFAULT_DELEGATION_DAYS)
}

/// `true` if the operator opted into announcing a Server Card (Spec A). Off by
/// default — announcing is explicit (`project_identity_default_nonpublic`).
fn announce_flag(args: &[String]) -> bool {
    let v = resolve(args, "--announce", "AXENSTAX_ANNOUNCE", "0");
    matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

/// Resolve the showcase / kiosk-containment config from flags. Off by default —
/// a normal dedicated server is unaffected. (Spec 2026-06-19 §8 Phase 2.) The
/// enforcement is client-side (the web kiosk reads the same flags off the served
/// page via `ws_transport_web::showcase_config`); the server logs it so an
/// operator can see the kiosk is armed.
fn showcase_config(args: &[String]) -> crate::showcase::ShowcaseConfig {
    let showcase = resolve(args, "--showcase", "AXENSTAX_SHOWCASE", "0");
    let exit_action = resolve(args, "--exit-action", "AXENSTAX_EXIT_ACTION", "board");
    let auto_loop = resolve(args, "--auto-loop-secs", "AXENSTAX_AUTO_LOOP_SECS", "0");
    crate::showcase::ShowcaseConfig::from_flags(&showcase, &exit_action, &auto_loop)
}

/// The relays to publish the Server Card to: `--card-relays`/`AXENSTAX_CARD_RELAYS`
/// (comma list), defaulting to the public discovery relays.
fn card_relays(args: &[String]) -> Vec<String> {
    let raw = resolve(args, "--card-relays", "AXENSTAX_CARD_RELAYS", "");
    let mut relays: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if relays.is_empty() {
        relays = crate::server_resolve::public_default_relays();
    }
    relays
}

/// Whether the advertised endpoint should be `wss://` — true when fronted by
/// Caddy with a domain (`AXENSTAX_DOMAIN`), overridable via `--card-tls`.
fn announce_tls(args: &[String]) -> bool {
    let forced = resolve(args, "--card-tls", "AXENSTAX_CARD_TLS", "");
    match forced.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => !std::env::var("AXENSTAX_DOMAIN").unwrap_or_default().is_empty(),
    }
}

/// Parse npub (bech32) / hex strings into x-only pubkey bytes, skipping blanks,
/// `#` comments, and anything unparseable. (Track 4 operator allowlist.)
fn parse_whitelist_npubs(entries: &[String]) -> Vec<[u8; 32]> {
    entries
        .iter()
        .filter_map(|s| {
            let s = s.trim();
            if s.is_empty() || s.starts_with('#') {
                return None;
            }
            nostr::PublicKey::parse(s).ok().map(|pk| pk.to_bytes())
        })
        .collect()
}

/// A boolean word, any case: `1` / `true` / `yes` / `on` → `true`,
/// `0` / `false` / `no` / `off` → `false`, anything else → `None`.
fn parse_bool_word(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn bad_bool_message(source: &str, value: &str) -> String {
    format!(
        "invalid value '{value}' for {source} — expected 1/true/yes/on or 0/false/no/off"
    )
}

/// `--allow-guests` on the command line. `Ok(None)` = not given. A bare flag
/// (last argument, or followed by another `-…` flag) means `true`; otherwise it
/// takes an optional boolean value — `--allow-guests 0`, `--allow-guests=false`
/// — like the other boolean flags, so a falsy value cannot open the server.
/// A value that is not a boolean word is an error rather than a silent "yes".
fn allow_guests_cli(args: &[String]) -> Result<Option<bool>, String> {
    const FLAG: &str = "--allow-guests";
    for (i, arg) in args.iter().enumerate() {
        if let Some(value) = arg.strip_prefix("--allow-guests=") {
            return parse_bool_word(value)
                .map(Some)
                .ok_or_else(|| bad_bool_message(FLAG, value));
        }
        if arg == FLAG {
            return match args.get(i + 1) {
                Some(next) if !next.starts_with('-') => parse_bool_word(next)
                    .map(Some)
                    .ok_or_else(|| bad_bool_message(FLAG, next)),
                _ => Ok(Some(true)),
            };
        }
    }
    Ok(None)
}

/// `AXENSTAX_ALLOW_GUESTS`: only an explicit truthy word enables; empty/unset
/// is "not given"; anything unrecognised is an error.
fn allow_guests_env(env: Option<&str>) -> Result<Option<bool>, String> {
    match env.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(v) => parse_bool_word(v)
            .map(Some)
            .ok_or_else(|| bad_bool_message("AXENSTAX_ALLOW_GUESTS", v)),
    }
}

/// Did the operator open this server to anonymous guests? `--allow-guests`
/// (CLI wins, like every other flag here) else `AXENSTAX_ALLOW_GUESTS` (`env`);
/// neither means sign-in is required (owner decision 2026-10-06). `Err` for a
/// value that is not a boolean word — boot refuses to start on it.
fn parse_allow_guests(args: &[String], env: Option<&str>) -> Result<bool, String> {
    Ok(allow_guests_cli(args)?
        .or(allow_guests_env(env)?)
        .unwrap_or(false))
}

/// [`parse_allow_guests`], failing closed: a malformed value never admits guests.
fn allow_guests(args: &[String], env: Option<&str>) -> bool {
    parse_allow_guests(args, env).unwrap_or(false)
}

/// Is the retired sign-in switch still configured? It no longer does anything
/// (sign-in is the default); boot logs a pointer to `--allow-guests` instead.
fn retired_signin_switch_present(args: &[String], env: Option<&str>) -> bool {
    args.iter().any(|a| a == "--require-signin") || env.is_some()
}

/// Resolve the access policy: the sign-in requirement and the operator
/// allowlist (from `AXENSTAX_WHITELIST` comma-list + a `whitelist.txt` in the
/// identity dir, one npub per line). A non-empty allowlist forces sign-in.
///
/// Sign-in precedence: the `<identity-dir>/require_signin` file (written by the
/// `require-signin` admin command / the Operator Console toggle — survives
/// restarts, toggles at runtime) wins; otherwise sign-in is required unless
/// [`allow_guests`]. `--require-signin` / `AXENSTAX_REQUIRE_SIGNIN` are not read.
fn load_access_policy(args: &[String], dir: &std::path::Path) -> (bool, Vec<[u8; 32]>) {
    let require_signin = if let Ok(s) = std::fs::read_to_string(dir.join("require_signin")) {
        s.trim().eq_ignore_ascii_case("true")
    } else {
        !allow_guests(args, std::env::var("AXENSTAX_ALLOW_GUESTS").ok().as_deref())
    };
    let mut entries: Vec<String> = Vec::new();
    if let Ok(env) = std::env::var("AXENSTAX_WHITELIST") {
        entries.extend(env.split(',').map(str::to_string));
    }
    if let Ok(contents) = std::fs::read_to_string(dir.join("whitelist.txt")) {
        entries.extend(contents.lines().map(str::to_string));
    }
    (require_signin, parse_whitelist_npubs(&entries))
}

/// Load the operator blocklist (`blocklist.txt` in the identity dir, one npub per
/// line). Blocked npubs are refused regardless of the allowlist — block wins
/// (Spec B §5 precedence). Same on-disk format as the allowlist.
fn load_blocklist(dir: &std::path::Path) -> Vec<[u8; 32]> {
    let entries: Vec<String> = std::fs::read_to_string(dir.join("blocklist.txt"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    parse_whitelist_npubs(&entries)
}

/// npub (bech32) for an x-only pubkey, for writing the allowlist file.
fn bech32_of(pk: &[u8; 32]) -> Option<String> {
    use nostr::ToBech32;
    nostr::PublicKey::from_slice(pk).ok().and_then(|p| p.to_bech32().ok())
}

/// Apply a verified admin command to the on-disk policy under `dir`. The running
/// server picks the change up on its periodic policy reload. Returns a status line.
fn apply_admin_command_to_files(
    cmd: &crate::server_identity::AdminCommand,
    dir: &std::path::Path,
) -> Result<String, String> {
    use crate::server_identity::AdminCommand;
    let wl_path = dir.join("whitelist.txt");
    match cmd {
        AdminCommand::WhitelistAdd(pk) => {
            let npub = bech32_of(pk).ok_or("invalid pubkey")?;
            let mut lines: Vec<String> = std::fs::read_to_string(&wl_path)
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect();
            if !lines.iter().any(|l| l.trim() == npub) {
                lines.push(npub.clone());
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                std::fs::write(&wl_path, lines.join("\n") + "\n").map_err(|e| e.to_string())?;
            }
            Ok(format!("allowlist += {npub}"))
        }
        AdminCommand::WhitelistRemove(pk) => {
            let npub = bech32_of(pk).ok_or("invalid pubkey")?;
            let lines: Vec<String> = std::fs::read_to_string(&wl_path)
                .unwrap_or_default()
                .lines()
                .filter(|l| l.trim() != npub)
                .map(str::to_string)
                .collect();
            std::fs::write(&wl_path, lines.join("\n") + "\n").map_err(|e| e.to_string())?;
            Ok(format!("allowlist -= {npub}"))
        }
        AdminCommand::BlacklistAdd(pk) => {
            let npub = bech32_of(pk).ok_or("invalid pubkey")?;
            let bl_path = dir.join("blocklist.txt");
            let mut lines: Vec<String> = std::fs::read_to_string(&bl_path)
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect();
            if !lines.iter().any(|l| l.trim() == npub) {
                lines.push(npub.clone());
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                std::fs::write(&bl_path, lines.join("\n") + "\n").map_err(|e| e.to_string())?;
            }
            Ok(format!("blocklist += {npub}"))
        }
        AdminCommand::BlacklistRemove(pk) => {
            let npub = bech32_of(pk).ok_or("invalid pubkey")?;
            let bl_path = dir.join("blocklist.txt");
            let lines: Vec<String> = std::fs::read_to_string(&bl_path)
                .unwrap_or_default()
                .lines()
                .filter(|l| l.trim() != npub)
                .map(str::to_string)
                .collect();
            std::fs::write(&bl_path, lines.join("\n") + "\n").map_err(|e| e.to_string())?;
            Ok(format!("blocklist -= {npub}"))
        }
        AdminCommand::SetRequireSignin(b) => {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            std::fs::write(dir.join("require_signin"), if *b { "true" } else { "false" })
                .map_err(|e| e.to_string())?;
            Ok(format!("require-signin = {b}"))
        }
        AdminCommand::KickPlayer(pk) => {
            let npub = bech32_of(pk).ok_or("invalid pubkey")?;
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            let path = dir.join("kick");
            let mut q = std::fs::read_to_string(&path).unwrap_or_default();
            q.push_str(&npub);
            q.push('\n');
            std::fs::write(&path, q).map_err(|e| e.to_string())?;
            Ok(format!("kick queued: {npub}"))
        }
        // Erasure (Spec C §6) — operator-private session log.
        AdminCommand::ForgetPlayer(pk) => {
            let npub = bech32_of(pk).ok_or("invalid pubkey")?;
            let mut log = crate::console_telemetry::SessionLog::load(dir);
            log.forget_player(&npub);
            log.save(dir)?;
            Ok(format!("forgot player {npub}"))
        }
        AdminCommand::PurgeAllHistory => {
            crate::console_telemetry::SessionLog::default().save(dir)?;
            Ok("purged all session history".to_string())
        }
        // Console settings (Spec B §5) → console.json.
        cmd @ (AdminCommand::SetMaxPlayers(_)
        | AdminCommand::SetAnnounce(_)
        | AdminCommand::SetServerName(_)
        | AdminCommand::SetAbout(_)
        | AdminCommand::SetRegion(_)
        | AdminCommand::SetPrivacy(_, _)) => {
            let mut s = crate::console_settings::ConsoleSettings::load(dir);
            s.apply(cmd);
            s.save(dir)?;
            Ok(format!("settings updated ({cmd:?})"))
        }
    }
}

/// `--admin <signed-command.json>`: verify an operator-signed admin command
/// against this server's operator identity, then apply it to the on-disk policy.
/// The signed command can be produced anywhere (delivery over a relay is a later
/// owner-side step); this is the authenticated apply path.
pub fn run_admin(args: &[String]) {
    use nostr::JsonUtil;
    let dir = id_dir();
    let identity = match crate::server_identity::load(&dir).ok().flatten() {
        Some(i) => i,
        None => {
            eprintln!("server not provisioned — run --pair-server first");
            std::process::exit(1);
        }
    };
    let operator = match identity.attestation() {
        Some(a) => a.operator,
        None => {
            eprintln!("server has no attestation — run --pair-server first");
            std::process::exit(1);
        }
    };
    let path = match cli_value(args, "--admin") {
        Some(p) => p,
        None => {
            eprintln!("usage: --admin <signed-command.json>");
            std::process::exit(1);
        }
    };
    let json = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {path}: {e}");
            std::process::exit(1);
        }
    };
    let event = match nostr::Event::from_json(&json) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("parse command event: {e}");
            std::process::exit(1);
        }
    };
    // Audience-bound to this server + replay-deduped (Spec 08 §9.0.1). The
    // replay record is shared (under a file lock) with the relay listener.
    match crate::server_identity::accept_admin_command_in(
        &dir,
        &event,
        &operator,
        &identity.runtime_pubkey(),
        nostr::Timestamp::now(),
        crate::server_identity::DEFAULT_ADMIN_SKEW_SECS,
    ) {
        Ok(cmd) => match apply_admin_command_to_files(&cmd, &dir) {
            Ok(msg) => println!("✅ {msg} (live within ~5s on the running server)"),
            Err(e) => {
                eprintln!("apply failed: {e}");
                std::process::exit(1);
            }
        },
        Err(e) => {
            eprintln!("❌ admin command rejected: {e:?}");
            std::process::exit(1);
        }
    }
}

/// Relay used to carry operator admin commands (`--admin-relay` / env). Defaults
/// to the default pairing relay (a public relay); `off`/`none` disables relay
/// delivery.
fn admin_relay_url(args: &[String]) -> String {
    let v = resolve(
        args,
        "--admin-relay",
        "AXENSTAX_ADMIN_RELAY",
        crate::server_identity::DEFAULT_PAIR_RELAY,
    );
    if v.eq_ignore_ascii_case("off") || v.eq_ignore_ascii_case("none") {
        String::new()
    } else {
        v
    }
}

/// `--admin-publish <signed-command.json>`: publish an already-operator-signed
/// admin command to the relay so a running server applies it — no box access.
pub fn run_admin_publish(args: &[String]) {
    use nostr::JsonUtil;
    let path = match cli_value(args, "--admin-publish") {
        Some(p) => p,
        None => {
            eprintln!("usage: --admin-publish <signed-command.json> [--admin-relay <url>]");
            std::process::exit(1);
        }
    };
    let json = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("read {path}: {e}");
        std::process::exit(1);
    });
    if nostr::Event::from_json(&json).is_err() {
        eprintln!("not a valid signed event JSON: {path}");
        std::process::exit(1);
    }
    let relay = admin_relay_url(args);
    if relay.is_empty() {
        eprintln!("admin relay is disabled — set --admin-relay <url>");
        std::process::exit(1);
    }
    let rt = cli_runtime();
    match rt.block_on(crate::server_identity::admin_relay::publish_event(&relay, &json)) {
        Ok(()) => println!("✅ published to {relay} (the running server applies it within ~5s)"),
        Err(e) => {
            eprintln!("❌ publish failed: {e}");
            std::process::exit(1);
        }
    }
}

/// `--admin-sign "<verb> <arg>"`: sign an admin command with the operator's
/// Heartwood (restored session) AND publish it to the relay in one step. Verbs:
/// `whitelist-add <npub>` · `whitelist-remove <npub>` · `require-signin <true|false>`.
/// With the relay disabled it prints the signed event for manual delivery.
pub fn run_admin_sign(args: &[String]) {
    use nostr::JsonUtil;
    let spec = match cli_value(args, "--admin-sign") {
        Some(s) => s,
        None => {
            eprintln!(
                "usage: --admin-sign \"<verb> <arg>\"  e.g. \"whitelist-add npub1…\" | \"require-signin true\""
            );
            std::process::exit(1);
        }
    };
    let (verb, arg) = match spec.trim().split_once(' ') {
        Some((v, a)) => (v.trim(), a.trim()),
        None => (spec.trim(), ""),
    };
    let dir = id_dir();
    let rt = cli_runtime();
    let event = match rt.block_on(crate::server_identity::sign_admin_command_via_session(
        &dir, verb, arg,
    )) {
        Ok(ev) => ev,
        Err(e) => {
            eprintln!("❌ signing failed: {e}");
            std::process::exit(1);
        }
    };
    let relay = admin_relay_url(args);
    if relay.is_empty() {
        // No relay configured — emit the signed event for manual delivery / --admin.
        println!("{}", event.as_json());
        return;
    }
    match rt.block_on(crate::server_identity::admin_relay::publish_event(
        &relay,
        &event.as_json(),
    )) {
        Ok(()) => println!("✅ signed + published to {relay} (live within ~5s)"),
        Err(e) => {
            eprintln!("❌ publish failed (event was signed):\n{}\n{e}", event.as_json());
            std::process::exit(1);
        }
    }
}

/// The NIP-46 pairing relay for `--pair-server`: `--pair-relay` /
/// `AXENSTAX_PAIR_RELAY`, defaulting to a public relay.
fn pair_relay(args: &[String]) -> String {
    resolve(
        args,
        "--pair-relay",
        "AXENSTAX_PAIR_RELAY",
        crate::server_identity::DEFAULT_PAIR_RELAY,
    )
}

/// `--pair-server`: interactive first-time pairing with the operator's Heartwood.
pub fn run_pair(args: &[String]) {
    let cfg = parse_config(args);
    let relay = pair_relay(args);
    let host = public_host(args);
    let days = delegation_days(args);
    let rt = cli_runtime();
    match rt.block_on(crate::server_identity::pair_server(
        &id_dir(),
        &relay,
        &cfg.server_name,
        host.as_deref(),
        days,
    )) {
        Ok(npub) => {
            println!("\n✅ Paired. Operator identity: {npub}");
            let h = host.unwrap_or_else(|| "<host>".to_string());
            println!(
                "Share this connect-string with players:\n  {}",
                crate::server_identity::build_connect_string(&h, cfg.ws_port, Some(&npub))
            );
        }
        Err(e) => {
            eprintln!("\n❌ Pairing failed: {e}");
            std::process::exit(1);
        }
    }
}

/// `--refresh-delegation`: silent renewal via the stored bunker session.
pub fn run_refresh(args: &[String]) {
    let cfg = parse_config(args);
    let host = public_host(args);
    let days = delegation_days(args);
    let rt = cli_runtime();
    // The relay argument is unused by `refresh_delegation` (it reconnects via
    // the stored bunker session, whose relays were fixed at pairing time).
    match rt.block_on(crate::server_identity::refresh_delegation(
        &id_dir(),
        crate::server_identity::DEFAULT_PAIR_RELAY,
        &cfg.server_name,
        host.as_deref(),
        days,
    )) {
        Ok(npub) => println!("\n✅ Delegation refreshed for operator {npub}"),
        Err(e) => {
            eprintln!("\n❌ Refresh failed: {e}");
            std::process::exit(1);
        }
    }
}

/// `--show-connect`: print the connect-string for this server.
pub fn run_show_connect(args: &[String]) {
    let cfg = parse_config(args);
    let host = public_host(args).unwrap_or_else(|| "<host>".to_string());
    let npub = crate::server_identity::load(&id_dir())
        .ok()
        .flatten()
        .and_then(|i| i.operator_npub());
    println!(
        "{}",
        crate::server_identity::build_connect_string(&host, cfg.ws_port, npub.as_deref())
    );
}

/// Entry point dispatched from `main()` when `--server` is present.
pub fn run(args: &[String]) {
    // A malformed `--allow-guests` / `AXENSTAX_ALLOW_GUESTS` value must be loud:
    // refuse to boot rather than guess whether the operator meant to open the
    // server (the access policy itself fails closed on it).
    if let Err(e) = parse_allow_guests(args, std::env::var("AXENSTAX_ALLOW_GUESTS").ok().as_deref()) {
        eprintln!("{e}");
        std::process::exit(1);
    }
    let cfg = parse_config(args);

    // Server identity (Heartwood-backed). Loaded before anything starts so
    // `AXENSTAX_REQUIRE_VERIFIED` can refuse to boot; additive otherwise.
    let identity = crate::server_identity::load(&id_dir()).ok().flatten();
    if let Err(e) = crate::server_identity::startup_gate(
        identity.as_ref(),
        require_verified_flag(args),
        nostr::Timestamp::now(),
    ) {
        log::error!("{e}");
        std::process::exit(1);
    }

    // A world saved by a newer engine is refused before anything loads or writes
    // it (Spec 02 §8.4). Booting on would fail the load, generate a fresh world
    // and save it at tick 0 (below) — over the real one.
    if let Some(why) = crate::save::world_open_refusal(&cfg.world) {
        log::error!("world '{}': {why}", cfg.world);
        std::process::exit(1);
    }

    // Bootstrap a fresh world's metadata so the chosen seed + game mode persist
    // and the world shows the right mode. Existing worlds are loaded as-is — and
    // "existing" is anything saved in the folder, not just a world.dat: a folder
    // that lost its world.dat but holds chunks must not get a new meta written
    // over its real one before the load refuses it (Spec 02 §8.4).
    let is_new = crate::world_open::is_new_world(&cfg.world).unwrap_or_else(|why| {
        log::error!(
            "world '{}' ({}) couldn't be opened: {why}. Nothing was changed.",
            cfg.world,
            crate::save::world_dir(&cfg.world).display()
        );
        std::process::exit(1);
    });
    if is_new {
        let mut meta = crate::save::WorldMeta::new(&cfg.world);
        meta.display_name = cfg.server_name.clone();
        meta.game_mode = cfg.game_mode.clone();
        // Terrain preset (Publish flow P1) — the GameServer reads these back from
        // the meta to drive blank-canvas vs seeded biome generation (server.rs).
        meta.world_type = cfg.world_type.clone();
        meta.ground = cfg.ground.clone();
        meta.water_depth = cfg.water_depth;
        if let Some(s) = cfg.seed {
            meta.seed = s;
        }
        if let Err(e) = crate::save::save_world_meta(&cfg.world, &meta) {
            log::error!("Failed to write world meta for '{}': {e}", cfg.world);
            std::process::exit(1);
        }
        log::info!(
            "Bootstrapped new server world '{}' (seed {}, mode {})",
            cfg.world,
            meta.seed,
            meta.game_mode
        );
    }

    // The seed HostedServer generates terrain from is the world meta's seed
    // (which we just wrote for a new world, or the persisted one for an existing
    // world). A CLI/env `--seed` override only applies to a brand-new world.
    let seed = crate::save::load_world_meta(&cfg.world).seed;

    // 0 local players — a pure dedicated server. Remote players arrive over
    // WebSocket and are server-simulated.
    let mut hs = match HostedServer::start(
        0,
        cfg.world.clone(),
        seed,
        cfg.max_players,
        RemoteTransport::WebSocket { port: cfg.ws_port },
    ) {
        Ok(hs) => hs,
        Err(e) => {
            // A world that failed to load lands here with its file and cause
            // ("world '<name>' (<dir>) couldn't be opened: … Nothing was changed."):
            // the server exits BEFORE the tick-0 save below, so a broken world is
            // never replaced by a fresh one (Spec 02 §8.4).
            log::error!("Failed to start dedicated server: {e}");
            std::process::exit(1);
        }
    };

    // Persist the freshly-generated world immediately so `world.dat` exists from
    // tick 0 (a crash before the first autosave doesn't lose the generation).
    // Reached only when the world loaded or there was nothing saved to load.
    hs.server.save();

    log::info!("──────────────────────────────────────────────");
    log::info!("  Axe'n'Stax dedicated server");
    log::info!("  world      : {}", cfg.world);
    log::info!("  mode       : {}", cfg.game_mode);
    log::info!("  seed       : {seed}");
    log::info!("  max players: {}", cfg.max_players);
    log::info!("  ws port    : {}", cfg.ws_port);
    match classify_boot_identity(identity.as_ref(), nostr::Timestamp::now()) {
        BootIdentity::Verified(npub) => log::info!("  identity   : VERIFIED ({npub})"),
        BootIdentity::Expired => {
            log::info!("  identity   : EXPIRED — run --refresh-delegation")
        }
        BootIdentity::Anonymous => {
            log::info!("  identity   : anonymous (run --pair-server to add one)")
        }
    }
    log::info!("  native join: ws://<host>:{}", cfg.ws_port);
    log::info!("  web join   : open the Caddy HTTPS URL (wss://<host>/ws)");
    log::info!("──────────────────────────────────────────────");

    // Capture the operator pubkey before `identity` is moved into the server, so
    // the admin-relay listener (below) can verify operator-signed commands.
    let operator_pubkey = identity.as_ref().and_then(|i| i.attestation().map(|a| a.operator));
    // …and this server's own runtime key: admin commands must name it.
    let server_pubkey = identity.as_ref().map(|i| i.runtime_pubkey());

    // Hand the loaded identity to the server so every JoinAccept carries a
    // server-identity proof (Track 3). Moves `identity` (last use).
    hs.set_identity(identity);

    // Access policy (Track 4): sign-in requirement + operator allowlist.
    if retired_signin_switch_present(args, std::env::var("AXENSTAX_REQUIRE_SIGNIN").ok().as_deref()) {
        log::warn!(
            "--require-signin / AXENSTAX_REQUIRE_SIGNIN is no longer read: sign-in is \
             required by default. Use --allow-guests (AXENSTAX_ALLOW_GUESTS=1) to admit guests."
        );
    }
    let (require_signin, whitelist) = load_access_policy(args, &id_dir());
    if require_signin || !whitelist.is_empty() {
        log::info!(
            "  access     : sign-in required{}",
            if whitelist.is_empty() {
                String::new()
            } else {
                format!(" + allowlist ({} npub(s))", whitelist.len())
            }
        );
    } else {
        log::info!("  access     : guests admitted (--allow-guests / require_signin file)");
    }
    hs.set_access_policy(require_signin, whitelist, load_blocklist(&id_dir()));

    // World chat (Phase 3) — operator tightening policy (§2.5: Charter sets
    // the ceiling, the operator may only tighten). `CommsLevel::parse` returns
    // `None` on a typo, which is a startup error here rather than a silently
    // permissive world — the whole point of making a typo loud.
    let chat_level_arg = resolve(args, "--chat-level", "AXENSTAX_CHAT_LEVEL", "anyone");
    let chat_level = match crate::comms::CommsLevel::parse(&chat_level_arg) {
        Some(level) => level,
        None => {
            eprintln!(
                "invalid --chat-level '{chat_level_arg}' — expected one of: blocked, approved, anyone"
            );
            std::process::exit(1);
        }
    };
    log::info!("  chat level : {}", chat_level.as_str());
    hs.set_chat_level(chat_level);

    // Showcase / kiosk-containment (Spec 2026-06-19 §8 Phase 2). Enforcement is
    // client-side (the web kiosk reads the same flags off the served page); the
    // server only surfaces that the kiosk is armed so an operator can see it.
    let showcase = showcase_config(args);
    if showcase.enabled {
        log::info!(
            "  showcase   : kiosk armed (exit={:?}{})",
            showcase.exit_action,
            match showcase.auto_loop_secs {
                Some(s) => format!(", auto-loop {s}s"),
                None => ", dead-end".to_string(),
            }
        );
    }

    // Operator telemetry (Spec B §6 / Spec C): wire the session log + privacy
    // level. Default `none` (no-tracking) until the operator sets a level via the
    // `privacy` admin command / console.json.
    {
        let s = crate::console_settings::ConsoleSettings::load(&id_dir());
        hs.set_telemetry(
            id_dir(),
            crate::privacy::from_settings(&s.privacy_level, s.privacy_retention_days),
        );
    }

    // C2 — operator admin commands over a relay (live policy changes, no shell
    // access). Only a provisioned server has an operator to trust. The listener
    // runs on its own thread and only writes the on-disk policy files; the ~5s
    // reload in the loop below applies them — so there's no shared state.
    if let (Some(operator), Some(server_pk)) = (operator_pubkey, server_pubkey) {
        let relay = admin_relay_url(args);
        if !relay.is_empty() {
            log::info!("  admin      : relay listener on {relay}");
            let dir = id_dir();
            let op_hex = operator.to_hex();
            std::thread::spawn(move || {
                use nostr::JsonUtil;
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        log::error!("admin relay: could not start runtime: {e}");
                        return;
                    }
                };
                rt.block_on(crate::server_identity::admin_relay::subscribe_admin_commands(
                    &relay,
                    &op_hex,
                    |event_json| {
                        let ev = match nostr::Event::from_json(event_json) {
                            Ok(e) => e,
                            Err(e) => {
                                log::warn!("admin relay: unparseable event: {e}");
                                return;
                            }
                        };
                        // Audience-bound + replay-deduped (Spec 08 §9.0.1).
                        // The replay record is re-read under a file lock on
                        // every command, so `--admin` runs aren't overwritten.
                        match crate::server_identity::accept_admin_command_in(
                            &dir,
                            &ev,
                            &operator,
                            &server_pk,
                            nostr::Timestamp::now(),
                            crate::server_identity::DEFAULT_ADMIN_SKEW_SECS,
                        ) {
                            Ok(cmd) => match apply_admin_command_to_files(&cmd, &dir) {
                                Ok(msg) => log::info!("admin relay: applied — {msg}"),
                                Err(e) => log::warn!("admin relay: apply failed: {e}"),
                            },
                            Err(e) => log::warn!("admin relay: rejected command: {e:?}"),
                        }
                    },
                ));
            });
        }
    }

    // Server Card (Spec A) — opt-in announcement. Publishes the runtime-signed
    // Address Card to the configured relays on boot + a ~5-min heartbeat so
    // players can resolve this server by its operator npub. Off unless
    // `--announce`; only a provisioned server can sign a card. The live relay
    // publish is the owner boundary. (Live player-count in the card is a
    // follow-up — the publisher thread doesn't yet share the main loop's count,
    // so it advertises `0/max` for now.)
    // Announce if the CLI/env asks OR the operator set it via a console command
    // (persisted in console.json). Live mid-session toggling is a follow-up.
    let cli_announce = announce_flag(args);
    if crate::server_card_publish::should_announce(
        cli_announce,
        crate::console_settings::ConsoleSettings::load(&id_dir()).announce,
    ) {
        match public_host(args) {
            Some(host) => {
                let relays = card_relays(args);
                let name = cfg.server_name.clone();
                let port = cfg.ws_port;
                let max = cfg.max_players.min(u16::MAX as usize) as u16;
                let tls = announce_tls(args);
                log::info!("  announce   : Server Card on {} relay(s)", relays.len());
                std::thread::spawn(move || {
                    let rt = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(rt) => rt,
                        Err(e) => {
                            log::error!("card publish: could not start runtime: {e}");
                            return;
                        }
                    };
                    rt.block_on(async move {
                        let Some(id) = crate::server_identity::load(&id_dir()).ok().flatten()
                        else {
                            log::warn!("card publish: no server identity — not announcing");
                            return;
                        };
                        loop {
                            // Reflect the operator's console settings on each
                            // heartbeat: descriptor + capacity (Spec B) and the
                            // privacy posture (Spec C — default "none"/no tracking
                            // when unset). Settings fall back to the CLI config.
                            let s = crate::console_settings::ConsoleSettings::load(&id_dir());
                            // Red line 1: announcing is opt-in and `announce false`
                            // must turn it fully off — withdraw the card (NIP-09)
                            // and stop publishing. Re-enabling takes a restart.
                            if !crate::server_card_publish::should_announce(cli_announce, s.announce) {
                                crate::server_card_publish::retract_card(&id, &relays).await;
                                log::info!("card publish: announce turned off — card withdrawn, heartbeat stopped");
                                break;
                            }
                            let card_name = s.server_name.clone().unwrap_or_else(|| name.clone());
                            let card_max = s.max_players.unwrap_or(max);
                            let privacy = crate::privacy::encode(&crate::privacy::from_settings(
                                &s.privacy_level,
                                s.privacy_retention_days,
                            ));
                            let card = crate::server_card_publish::build_card_from_config(
                                &host,
                                port,
                                tls,
                                &card_name,
                                &s.about,
                                &s.region,
                                0,
                                card_max,
                                crate::protocol::PROTOCOL_VERSION,
                                &privacy,
                            );
                            crate::server_card_publish::publish_card(&id, &card, &relays).await;
                            // ~5-min heartbeat, but re-check the setting every 15s
                            // so `announce false` withdraws the card promptly.
                            for _ in 0..20 {
                                tokio::time::sleep(Duration::from_secs(15)).await;
                                let on = crate::console_settings::ConsoleSettings::load(&id_dir())
                                    .announce;
                                if !crate::server_card_publish::should_announce(cli_announce, on) {
                                    break;
                                }
                            }
                        }
                    });
                });
            }
            None => log::warn!("--announce set but no --public-host — not publishing a Card"),
        }
    }

    // Graceful shutdown: SIGINT (Ctrl-C) / SIGTERM (`docker stop`) → save + exit.
    let running = Arc::new(AtomicBool::new(true));
    {
        let running = running.clone();
        if let Err(e) = ctrlc::set_handler(move || {
            log::info!("Shutdown signal received — finishing tick and saving…");
            running.store(false, Ordering::Relaxed);
        }) {
            log::warn!(
                "Could not install signal handler: {e} (autosave still protects the world)"
            );
        }
    }

    // 20 TPS authoritative loop with periodic autosave.
    let tick_dt = Duration::from_millis(50);
    let autosave_dt = Duration::from_secs(cfg.autosave_secs);
    let mut next_tick = Instant::now();
    let mut last_save = Instant::now();
    // Track 5 — re-read the access policy periodically so operator admin commands
    // (which mutate the on-disk policy) take effect live, without a restart.
    let policy_reload_dt = Duration::from_secs(5);
    let mut last_policy_reload = Instant::now();

    while running.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now >= next_tick {
            hs.tick();
            next_tick += tick_dt;
            // If we fell badly behind (e.g. a long save), resync rather than
            // spiral trying to catch up every missed tick.
            if next_tick < now {
                next_tick = now + tick_dt;
            }
        }

        if last_save.elapsed() >= autosave_dt {
            hs.server.save();
            log::info!("Autosaved '{}'", cfg.world);
            last_save = Instant::now();
        }

        if last_policy_reload.elapsed() >= policy_reload_dt {
            let (require_signin, whitelist) = load_access_policy(args, &id_dir());
            hs.set_access_policy(require_signin, whitelist, load_blocklist(&id_dir()));
            hs.process_pending_kicks(&id_dir());
            // Refresh the privacy level (a `privacy` admin command may have changed
            // it) then apply retention + persist the session log (Spec C).
            let s = crate::console_settings::ConsoleSettings::load(&id_dir());
            hs.set_telemetry(
                id_dir(),
                crate::privacy::from_settings(&s.privacy_level, s.privacy_retention_days),
            );
            hs.flush_telemetry();
            last_policy_reload = Instant::now();
        }

        // Yield so we don't busy-spin between ticks.
        std::thread::sleep(Duration::from_millis(2));
    }

    log::info!("Saving '{}' and shutting down…", cfg.world);
    hs.shutdown();
    hs.server.save();
    log::info!("Saved. Goodbye.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{Keys, Timestamp};

    fn tmp(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("axe_srvmain_{}_{}", tag, std::process::id()))
    }

    #[test]
    fn showcase_off_by_default() {
        let cfg = showcase_config(&[]);
        assert!(!cfg.enabled, "a normal dedicated server is not a kiosk");
    }

    #[test]
    fn showcase_cli_flag_enables_and_reads_loop() {
        let args: Vec<String> = ["--showcase", "1", "--auto-loop-secs", "60"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let cfg = showcase_config(&args);
        assert!(cfg.enabled);
        assert_eq!(cfg.auto_loop_secs, Some(60));
    }

    #[test]
    fn parse_config_reads_terrain_from_cli() {
        // Publish flow P1: the console passes the operator's terrain choice via
        // env/CLI; CLI is deterministic (beats env) so the plumbing is testable
        // without mutating process env. The GameServer reads these back off the
        // bootstrapped world meta (server.rs).
        let args: Vec<String> = ["--world-type", "flat", "--ground", "sand", "--water-depth", "5"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let cfg = parse_config(&args);
        assert_eq!(cfg.world_type, "flat");
        assert_eq!(cfg.ground, "sand");
        assert_eq!(cfg.water_depth, 5);
    }

    #[test]
    fn parse_whitelist_keeps_valid_drops_junk() {
        use nostr::ToBech32;
        let keys = Keys::generate();
        let npub = keys.public_key().to_bech32().unwrap();
        let entries = vec![
            npub,
            "# a comment".to_string(),
            "   ".to_string(),
            "not-a-real-npub".to_string(),
        ];
        let parsed = parse_whitelist_npubs(&entries);
        assert_eq!(parsed.len(), 1, "only the valid npub survives");
        assert_eq!(parsed[0], keys.public_key().to_bytes());
    }

    #[test]
    fn admin_whitelist_add_remove_persists_via_files() {
        use crate::server_identity::AdminCommand;
        let dir = tmp("adminfiles");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pk = Keys::generate().public_key().to_bytes();
        apply_admin_command_to_files(&AdminCommand::WhitelistAdd(pk), &dir).unwrap();
        let (_rs, wl) = load_access_policy(&[], &dir);
        assert!(wl.contains(&pk), "added npub must be in the loaded allowlist");
        apply_admin_command_to_files(&AdminCommand::WhitelistRemove(pk), &dir).unwrap();
        let (_rs2, wl2) = load_access_policy(&[], &dir);
        assert!(!wl2.contains(&pk), "removed npub must be gone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn admin_blacklist_add_remove_persists_via_files() {
        use crate::server_identity::AdminCommand;
        let dir = tmp("adminblock");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pk = Keys::generate().public_key().to_bytes();
        apply_admin_command_to_files(&AdminCommand::BlacklistAdd(pk), &dir).unwrap();
        assert!(
            load_blocklist(&dir).contains(&pk),
            "added npub must be in the loaded blocklist"
        );
        apply_admin_command_to_files(&AdminCommand::BlacklistRemove(pk), &dir).unwrap();
        assert!(
            !load_blocklist(&dir).contains(&pk),
            "removed npub must be gone"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn admin_settings_command_persists_to_console_json() {
        use crate::server_identity::AdminCommand;
        let dir = tmp("adminsettings");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        apply_admin_command_to_files(&AdminCommand::SetServerName("Cool SMP".into()), &dir).unwrap();
        apply_admin_command_to_files(&AdminCommand::SetMaxPlayers(20), &dir).unwrap();
        apply_admin_command_to_files(&AdminCommand::SetPrivacy("sessions".into(), 30), &dir).unwrap();
        let s = crate::console_settings::ConsoleSettings::load(&dir);
        assert_eq!(s.server_name.as_deref(), Some("Cool SMP"));
        assert_eq!(s.max_players, Some(20));
        assert_eq!(s.privacy_level, "sessions");
        assert_eq!(s.privacy_retention_days, 30);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn admin_kick_command_queues_to_file() {
        use crate::server_identity::AdminCommand;
        let dir = tmp("adminkick");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pk = Keys::generate().public_key().to_bytes();
        apply_admin_command_to_files(&AdminCommand::KickPlayer(pk), &dir).unwrap();
        let contents = std::fs::read_to_string(dir.join("kick")).unwrap();
        assert!(
            !contents.trim().is_empty(),
            "kick queue file must contain the npub"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn admin_forget_player_erases_from_session_log() {
        use crate::server_identity::AdminCommand;
        let dir = tmp("adminforget");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Seed the session log with the target's npub (stored bech32, as a join would).
        let pkb = Keys::generate().public_key().to_bytes();
        let npub = bech32_of(&pkb).unwrap();
        let mut log = crate::console_telemetry::SessionLog::default();
        log.record_connect(&npub, 10);
        log.record_connect("npub1other", 20);
        log.save(&dir).unwrap();

        apply_admin_command_to_files(&AdminCommand::ForgetPlayer(pkb), &dir).unwrap();
        let reloaded = crate::console_telemetry::SessionLog::load(&dir);
        assert_eq!(reloaded.sessions.len(), 1);
        assert_eq!(reloaded.sessions[0].npub, "npub1other");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Owner decision 2026-10-06 (O-7 #3): a dedicated server with no
    /// configuration requires sign-in. (Reads the real `AXENSTAX_ALLOW_GUESTS`,
    /// which is unset in the test environment; tests never set env vars.)
    #[test]
    fn sign_in_is_required_by_default() {
        let dir = tmp("signin-default");
        let _ = std::fs::remove_dir_all(&dir);
        let (rs, wl) = load_access_policy(&[], &dir);
        assert!(rs, "no flag, no env, no file => sign-in required");
        assert!(wl.is_empty());
    }

    #[test]
    fn allow_guests_flag_opens_the_server() {
        let dir = tmp("signin-allow-guests");
        let _ = std::fs::remove_dir_all(&dir);
        let args = vec!["--allow-guests".to_string()];
        let (rs, _wl) = load_access_policy(&args, &dir);
        assert!(!rs, "--allow-guests admits guests");
        assert!(allow_guests(&[], Some("1")) && allow_guests(&[], Some("TRUE")));
        assert!(!allow_guests(&[], Some("0")) && !allow_guests(&[], None));
    }

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// `--allow-guests` takes an optional boolean value (as the other boolean
    /// flags do): bare means yes, but `--allow-guests 0` / `=false` must not
    /// open the server (it used to — the flag was matched without its value).
    #[test]
    fn allow_guests_flag_takes_an_optional_boolean_value() {
        // Bare, or followed by another flag: admits guests.
        assert!(allow_guests(&argv(&["--allow-guests"]), None));
        assert!(allow_guests(&argv(&["--allow-guests", "--port", "7000"]), None));
        assert!(allow_guests(&argv(&["--server", "--allow-guests"]), None));
        // An explicit truthy value.
        for v in ["1", "true", "TRUE", "yes", "on"] {
            assert!(allow_guests(&argv(&["--allow-guests", v]), None), "{v}");
            assert!(allow_guests(&argv(&[&format!("--allow-guests={v}")]), None), "={v}");
        }
        // An explicit falsy value does NOT admit guests.
        for v in ["0", "false", "False", "no", "off"] {
            assert!(!allow_guests(&argv(&["--allow-guests", v]), None), "{v}");
            assert!(!allow_guests(&argv(&[&format!("--allow-guests={v}")]), None), "={v}");
        }
        // The value belongs to the flag: a following flag is not swallowed.
        assert!(!allow_guests(&argv(&["--allow-guests", "0", "--port", "1"]), None));
    }

    /// An unrecognised value is an error, never a silent "yes" — and the policy
    /// fails closed (sign-in required) while boot refuses to start.
    #[test]
    fn allow_guests_rejects_an_unrecognised_value() {
        for args in [
            argv(&["--allow-guests", "maybe"]),
            argv(&["--allow-guests=2"]),
            argv(&["--allow-guests="]),
        ] {
            let err = parse_allow_guests(&args, None).unwrap_err();
            assert!(err.contains("--allow-guests"), "{err}");
            assert!(!allow_guests(&args, None), "fails closed: {args:?}");
        }
        let err = parse_allow_guests(&[], Some("maybe")).unwrap_err();
        assert!(err.contains("AXENSTAX_ALLOW_GUESTS"), "{err}");
        assert!(!allow_guests(&[], Some("maybe")), "a garbled env value fails closed");
    }

    /// Env: only an explicit truthy word enables; empty is unset. The CLI
    /// (like every other flag here) overrides the env var, in both directions.
    #[test]
    fn allow_guests_env_and_cli_precedence() {
        for v in ["1", "true", "Yes", "on"] {
            assert!(allow_guests(&[], Some(v)), "{v}");
        }
        for v in ["0", "false", "no", "off", "", "  "] {
            assert!(!allow_guests(&[], Some(v)), "{v:?}");
        }
        assert!(!allow_guests(&argv(&["--allow-guests", "0"]), Some("1")), "CLI 0 beats env 1");
        assert!(allow_guests(&argv(&["--allow-guests"]), Some("0")), "bare CLI beats env 0");
        assert_eq!(parse_allow_guests(&[], None), Ok(false), "unset: sign-in required");
    }

    /// End to end through the policy: a falsy flag value keeps sign-in required.
    #[test]
    fn allow_guests_zero_keeps_sign_in_required() {
        let dir = tmp("signin-allow-guests-zero");
        let _ = std::fs::remove_dir_all(&dir);
        for args in [argv(&["--allow-guests", "0"]), argv(&["--allow-guests=false"])] {
            assert!(load_access_policy(&args, &dir).0, "{args:?} still requires sign-in");
        }
        assert!(!load_access_policy(&argv(&["--allow-guests", "1"]), &dir).0);
    }

    #[test]
    fn retired_require_signin_switch_is_a_no_op() {
        let dir = tmp("signin-retired");
        let _ = std::fs::remove_dir_all(&dir);
        // The old way to open a server no longer opens it…
        let args = vec!["--require-signin".to_string(), "0".to_string()];
        let (rs, _wl) = load_access_policy(&args, &dir);
        assert!(rs, "--require-signin 0 is ignored: still sign-in required");
        // …but it is still recognised, so boot can point at --allow-guests.
        assert!(retired_signin_switch_present(&args, None));
        assert!(retired_signin_switch_present(&[], Some("0")));
        assert!(!retired_signin_switch_present(&[], None));
    }

    /// Join `hs` as an unsigned guest; `Ok(())` on JoinAccept, `Err(reason)`
    /// on JoinReject.
    fn guest_join(hs: &mut HostedServer) -> Result<(), String> {
        use crate::transport::ClientTransport;
        let client = hs.attach_test_remote();
        let req = crate::remote_client::build_join_request_guest("Guest", 0);
        client.send_to_server(&crate::protocol::serialize_packet(
            crate::protocol::PacketType::JoinRequest,
            &req,
        ));
        hs.tick();
        let mut outcome = Err("no answer".to_string());
        while let Some(pkt) = client.try_recv_from_server() {
            match crate::protocol::deserialize_header(&pkt) {
                Some((crate::protocol::PacketType::JoinAccept, _)) => outcome = Ok(()),
                Some((crate::protocol::PacketType::JoinReject, payload)) => {
                    let rej: crate::protocol::JoinRejectPacket =
                        crate::protocol::safe_deserialize(payload).unwrap();
                    outcome = Err(rej.reason);
                }
                _ => {}
            }
        }
        outcome
    }

    /// End to end through the real join path: the dedicated server's default
    /// policy refuses an unsigned joiner; `--allow-guests` admits it.
    #[test]
    fn default_policy_refuses_an_unsigned_join_and_allow_guests_admits_it() {
        let dir = tmp("signin-e2e");
        let _ = std::fs::remove_dir_all(&dir);
        let mut hs = HostedServer::start(
            0,
            "signin-e2e-test".to_string(),
            42,
            0,
            RemoteTransport::WebSocket { port: 0 },
        )
        .expect("hosted server starts");

        let (rs, wl) = load_access_policy(&[], &dir);
        hs.set_access_policy(rs, wl, Vec::new());
        assert_eq!(
            guest_join(&mut hs),
            Err(crate::access_policy::reject_reason(
                crate::access_policy::AccessReject::SignInRequired
            )),
            "default: an unsigned join is refused with the sign-in reason"
        );

        let (rs, wl) = load_access_policy(&["--allow-guests".to_string()], &dir);
        hs.set_access_policy(rs, wl, Vec::new());
        assert_eq!(guest_join(&mut hs), Ok(()), "--allow-guests admits the guest");
    }

    #[test]
    fn require_signin_file_beats_the_flag_both_ways() {
        use crate::server_identity::AdminCommand;
        let dir = tmp("signin-file-wins");
        let _ = std::fs::remove_dir_all(&dir);
        let guests = vec!["--allow-guests".to_string()];
        apply_admin_command_to_files(&AdminCommand::SetRequireSignin(true), &dir).unwrap();
        assert!(load_access_policy(&guests, &dir).0, "file 'true' beats --allow-guests");
        apply_admin_command_to_files(&AdminCommand::SetRequireSignin(false), &dir).unwrap();
        assert!(!load_access_policy(&[], &dir).0, "file 'false' (console toggle) admits guests");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn admin_require_signin_file_overrides() {
        use crate::server_identity::AdminCommand;
        let dir = tmp("reqsignin");
        let _ = std::fs::remove_dir_all(&dir);
        apply_admin_command_to_files(&AdminCommand::SetRequireSignin(true), &dir).unwrap();
        let (rs, _wl) = load_access_policy(&[], &dir);
        assert!(rs, "require_signin file must force sign-in");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn anonymous_when_no_identity() {
        assert_eq!(
            classify_boot_identity(None, Timestamp::from(1)),
            BootIdentity::Anonymous
        );
    }

    #[tokio::test]
    async fn verified_in_window_else_expired() {
        let dir = tmp("classify");
        let _ = std::fs::remove_dir_all(&dir);
        let mut id = crate::server_identity::generate_runtime(&dir).unwrap();
        let op = Keys::generate();
        let ev = crate::server_identity::attestation::mint_attestation(
            &op,
            &id.runtime_pubkey(),
            Timestamp::from(100),
            Timestamp::from(200),
            &[27420],
            "W",
            None,
        )
        .await
        .unwrap();
        id.store_attestation(&dir, ev).unwrap();
        // In window → Verified with the operator npub.
        match classify_boot_identity(Some(&id), Timestamp::from(150)) {
            BootIdentity::Verified(npub) => assert!(npub.starts_with("npub1")),
            other => panic!("expected Verified, got {other:?}"),
        }
        // Outside window → Expired (attestation present but not currently valid).
        assert_eq!(
            classify_boot_identity(Some(&id), Timestamp::from(500)),
            BootIdentity::Expired
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
