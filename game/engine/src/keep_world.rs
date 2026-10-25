//! `--keep-world --world <name>`: this engine as the always-on world of a home
//! Keep (Ingle). Native only.
//!
//! The Keep runs this binary as a child process and owns everything a player
//! meets before the world: the room on its one Fray listener, invites, the
//! gate, blocks and the status page. This mode opens **no socket at all** (no
//! WebSocket listener, no QUIC, no relay, no LAN beacon, no announce); it
//! talks to the Keep only over stdin and stdout (`keep_pipe`), and logs to
//! stderr.
//!
//! - The world is `$AXENSTAX_WORLDS_DIR/<name>`; `--world` and
//!   `$AXENSTAX_WORLD` must both name it, or the server refuses to start. It
//!   is created on the first start with a random seed and kept after.
//! - An exclusive lock on [`LOCK_FILE`] inside the world's folder is held for
//!   the life of the process: a second server on the same world exits at once,
//!   having touched nothing.
//! - The Keep's `Hello` comes first; the server opens the world and answers
//!   `Ready` with its engine protocol. `Hello`'s `max_players` is the world's
//!   size.
//! - Each joiner's signed Signet join is verified by the server's own code
//!   over the binding the Keep passed for its connection; the server then asks
//!   the Keep's gate about the persona it verified and seats the player only
//!   on a yes (`HostedServer::start_keep_world`).
//! - The server's own access files (allowlist, blocklist, require-signin),
//!   Operator Console files, Server Card, announce and admin relay are all off:
//!   the Keep decides who plays. Sign-in is required.
//! - `Shutdown`, or stdin ending, saves the world and exits 0 (`Saved` is sent
//!   first). Autosave is the dedicated server's own (every 60 s). A malformed
//!   frame saves too, then exits non-zero.

#![cfg(not(target_arch = "wasm32"))]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::hosted_server::HostedServer;
use crate::keep_pipe::{self, Control, FromChild};

/// The lock file inside the world's folder. Never deleted: deleting a lock
/// file under its holder would let a second process lock a second file.
pub const LOCK_FILE: &str = ".keep-world.lock";

/// Exit status: the arguments or the environment are wrong.
const EXIT_CONFIG: i32 = 1;
/// Exit status: the pipe broke its protocol (or stdout could not be taken).
const EXIT_PROTOCOL: i32 = 2;
/// Exit status: another process holds the world.
const EXIT_LOCKED: i32 = 3;
/// Exit status: the world could not be opened.
const EXIT_WORLD: i32 = 4;

/// Autosave, as the dedicated server's default.
const AUTOSAVE: Duration = Duration::from_secs(60);
/// How long the last frames (`Saved`) are given to reach the Keep.
const FLUSH_WAIT: Duration = Duration::from_secs(5);
/// The name a new Keep world's meta carries (shown nowhere a joiner sees it).
const WORLD_DISPLAY_NAME: &str = "Keep world";

/// Entry point, dispatched from `run()` before anything else starts (no data
/// dir migration, no boot services, no worker that might dial out).
pub fn run(args: &[String]) -> ! {
    // stdout carries frames only. Take it for the pipe, then point fd 1 at
    // stderr, so nothing else in the engine (a stray `println!`) can ever write
    // into the pipe.
    let pipe = match take_stdout() {
        Ok(pipe) => pipe,
        Err(e) => {
            eprintln!("--keep-world: cannot take stdout for the pipe: {e}");
            std::process::exit(EXIT_PROTOCOL);
        }
    };
    let env = |key: &str| std::env::var(key).ok();
    let world = match keep_world_folder(args, env) {
        Ok(world) => world,
        Err(why) => {
            eprintln!("--keep-world: {why}");
            std::process::exit(EXIT_CONFIG);
        }
    };
    let worlds = crate::save::worlds_root();
    let _lock = match lock_world(&worlds, &world.name) {
        Ok(lock) => lock,
        Err(LockFailure::Held(path)) => {
            eprintln!(
                "--keep-world: another process is running world '{}' ({} is locked); this one exits and touches nothing",
                world.name,
                path.display()
            );
            std::process::exit(EXIT_LOCKED);
        }
        Err(LockFailure::Other(why)) => {
            eprintln!("--keep-world: {why}");
            std::process::exit(EXIT_WORLD);
        }
    };
    // The rest of the engine's native state (profile, settings) lives beside
    // the worlds, inside the Keep's data, never under $HOME.
    crate::data_dir::init_at(world.data_root);
    let code = serve(&world.name, std::io::stdin(), pipe, AUTOSAVE);
    std::process::exit(code);
}

/// Where the Keep's world lives.
#[derive(Debug, PartialEq, Eq)]
struct KeepWorld {
    /// The world's folder name under the worlds root.
    name: String,
    /// The engine's data root: `$AXENSTAX_DATA_DIR`, else the folder that holds
    /// the worlds root.
    data_root: PathBuf,
}

/// The world `--world` and `$AXENSTAX_WORLD` name (they must agree), under
/// `$AXENSTAX_WORLDS_DIR` (which must be set and exist).
fn keep_world_folder(args: &[String], env: impl Fn(&str) -> Option<String>) -> Result<KeepWorld, String> {
    let flag = args.iter().position(|a| a == "--world").and_then(|i| args.get(i + 1)).cloned();
    let Some(flag) = flag.filter(|w| !w.is_empty()) else {
        return Err("--world <name> is required".to_string());
    };
    let from_env = env("AXENSTAX_WORLD").filter(|w| !w.is_empty());
    if from_env.as_deref() != Some(flag.as_str()) {
        return Err(format!(
            "--world '{flag}' and AXENSTAX_WORLD ({}) must name the same world",
            from_env.map_or("unset".to_string(), |w| format!("'{w}'"))
        ));
    }
    let plain = flag.len() <= 64 && flag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !plain {
        return Err(format!("the world name '{flag}' must be 1-64 letters, digits, '-' or '_'"));
    }
    let Some(worlds) = env("AXENSTAX_WORLDS_DIR").filter(|d| !d.is_empty()).map(PathBuf::from) else {
        return Err("AXENSTAX_WORLDS_DIR must name the folder the world is kept in".to_string());
    };
    if !worlds.is_dir() {
        return Err(format!("AXENSTAX_WORLDS_DIR ({}) is not a folder", worlds.display()));
    }
    let data_root = match env(crate::data_dir::DATA_DIR_ENV).filter(|d| !d.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => match worlds.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => worlds.clone(),
        },
    };
    Ok(KeepWorld { name: flag, data_root })
}

/// Why the world could not be locked.
#[derive(Debug)]
enum LockFailure {
    /// Another process holds it (the lock file's path).
    Held(PathBuf),
    Other(String),
}

/// Create the world's folder if it is new, and take the exclusive lock on
/// [`LOCK_FILE`] inside it (`flock` on Unix, through the standard library).
/// The lock lasts as long as the returned file is open.
fn lock_world(worlds: &Path, name: &str) -> Result<std::fs::File, LockFailure> {
    let dir = worlds.join(name);
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(&dir).map_err(|e| LockFailure::Other(format!("cannot create {}: {e}", dir.display())))?;
    let path = dir.join(LOCK_FILE);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| LockFailure::Other(format!("cannot open {}: {e}", path.display())))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(LockFailure::Held(path)),
        Err(std::fs::TryLockError::Error(e)) => {
            Err(LockFailure::Other(format!("cannot lock {}: {e}", path.display())))
        }
    }
}

/// Duplicate fd 1 for the pipe, then make fd 1 a copy of stderr.
#[cfg(unix)]
fn take_stdout() -> std::io::Result<std::fs::File> {
    use std::os::fd::FromRawFd;
    std::io::stdout().flush()?;
    // SAFETY: plain fd calls on the process's own standard descriptors. The
    // duplicate is a fresh descriptor (close-on-exec) owned by the `File` alone.
    unsafe {
        let pipe = libc::fcntl(1, libc::F_DUPFD_CLOEXEC, 3);
        if pipe < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::dup2(2, 1) < 0 {
            let e = std::io::Error::last_os_error();
            libc::close(pipe);
            return Err(e);
        }
        Ok(std::fs::File::from_raw_fd(pipe))
    }
}

#[cfg(not(unix))]
fn take_stdout() -> std::io::Result<std::fs::File> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "--keep-world runs on Unix only"))
}

/// Why the loop stopped.
enum Stop {
    Shutdown,
    Ended,
    Fatal(String),
}

/// The world, served over the pipe until the Keep asks it to stop. Returns the
/// exit status. `input` is stdin and `output` the pipe (a test's streams in
/// tests); `world` is a folder under the worlds root, already locked.
pub(crate) fn serve(world: &str, input: impl Read + Send + 'static, output: impl Write + Send + 'static, autosave: Duration) -> i32 {
    let out = match keep_pipe::spawn_writer(output) {
        Ok(out) => out,
        Err(e) => {
            log::error!("keep world: cannot start the pipe's writer: {e}");
            return EXIT_PROTOCOL;
        }
    };
    let (joiners, joined) = mpsc::channel();
    let control = match keep_pipe::spawn_reader(input, out.clone(), joiners) {
        Ok(control) => control,
        Err(e) => {
            log::error!("keep world: cannot start the pipe's reader: {e}");
            return EXIT_PROTOCOL;
        }
    };
    let max_players = match control.recv() {
        Ok(Control::Hello { max_players }) => max_players,
        Ok(Control::Fatal(why)) => {
            log::error!("keep world: protocol error before the world opened: {why}");
            return EXIT_PROTOCOL;
        }
        Ok(other) => {
            log::error!("keep world: the pipe ended before Hello ({other:?})");
            return EXIT_PROTOCOL;
        }
        Err(_) => {
            log::error!("keep world: the pipe ended before Hello");
            return EXIT_PROTOCOL;
        }
    };
    let mut hs = match open_world(world, usize::from(max_players), joined) {
        Ok(hs) => hs,
        Err(why) => {
            log::error!("keep world: {why}");
            return EXIT_WORLD;
        }
    };
    out.send(&FromChild::Ready { protocol: crate::protocol::PROTOCOL_VERSION });
    log::info!(
        "keep world '{world}' is ready: {max_players} players, engine protocol {}, no sockets of its own",
        crate::protocol::PROTOCOL_VERSION
    );

    // 20 TPS, autosave on its own timer, as the dedicated server.
    let tick_dt = Duration::from_millis(50);
    let mut next_tick = Instant::now();
    let mut last_save = Instant::now();
    let stop = loop {
        match control.try_recv() {
            Ok(Control::Shutdown) => break Stop::Shutdown,
            Ok(Control::Ended) | Err(mpsc::TryRecvError::Disconnected) => break Stop::Ended,
            Ok(Control::Fatal(why)) => break Stop::Fatal(why),
            Ok(Control::Hello { .. }) => break Stop::Fatal("a second Hello".to_string()),
            Err(mpsc::TryRecvError::Empty) => {}
        }
        let now = Instant::now();
        if now >= next_tick {
            hs.tick();
            next_tick += tick_dt;
            if next_tick < now {
                next_tick = now + tick_dt;
            }
        }
        if last_save.elapsed() >= autosave {
            hs.server.save();
            last_save = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(2));
    };

    match &stop {
        Stop::Shutdown => log::info!("keep world: the Keep asked for a stop; saving '{world}'"),
        Stop::Ended => log::info!("keep world: the pipe ended; saving '{world}'"),
        Stop::Fatal(why) => log::error!("keep world: protocol error: {why}; saving '{world}' and exiting"),
    }
    hs.shutdown();
    let saved = hs.server.try_save();
    match &saved {
        Ok(()) => log::info!("keep world: saved '{world}'"),
        Err(e) => log::error!("keep world: saving '{world}' FAILED: {e}"),
    }
    // The players' links go first (each says Disconnect), so Saved is the last
    // frame the Keep reads.
    drop(hs);
    let code = match stop {
        Stop::Fatal(_) => EXIT_PROTOCOL,
        Stop::Shutdown | Stop::Ended => {
            out.send(&FromChild::Saved { ok: saved.is_ok() });
            0
        }
    };
    if !out.flush(FLUSH_WAIT) {
        log::warn!("keep world: the last frames did not reach the Keep in {} s", FLUSH_WAIT.as_secs());
    }
    code
}

/// Open (or create, with a random seed) the world and its server, as the
/// dedicated server's boot does, minus everything an operator configures.
fn open_world(
    world: &str,
    max_players: usize,
    joined: mpsc::Receiver<Box<dyn crate::transport::ServerTransport>>,
) -> Result<HostedServer, String> {
    // A world saved by a newer engine is refused before anything loads or
    // writes it (Spec 02 §8.4).
    if let Some(why) = crate::save::world_open_refusal(world) {
        return Err(format!("world '{world}': {why}"));
    }
    let is_new = crate::world_open::is_new_world(world)
        .map_err(|why| crate::world_open::server_refusal_message(world, &why))?;
    if is_new {
        // `WorldMeta::new` gives a random seed and a survival world on normal
        // terrain: the dedicated server's defaults.
        let mut meta = crate::save::WorldMeta::new(world);
        meta.display_name = WORLD_DISPLAY_NAME.to_string();
        crate::save::save_world_meta(world, &meta).map_err(|e| format!("cannot write the new world's meta: {e}"))?;
        log::info!("keep world: created '{world}' (seed {})", meta.seed);
    }
    let seed = crate::save::load_world_meta(world).seed;
    let mut hs = HostedServer::start_keep_world(world.to_string(), seed, max_players.max(1), joined)?;
    hs.server.set_sim_distance(crate::server_stream::DEFAULT_SIM_DISTANCE);
    hs.set_chunk_sync(crate::chunk_push::ChunkSync::Touched);
    crate::server_main::settle_pop_secret(world, &mut hs.server);
    // `world.dat` from tick 0, as the dedicated server: a crash before the
    // first autosave doesn't lose the generation.
    hs.server.save();
    Ok(hs)
}

#[cfg(test)]
mod tests;
