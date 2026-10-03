//! [`WorldRoom`] over a `kithmoot-agent` child process.
//!
//! **Wrap, don't port.** KithMoot is a mature TypeScript implementation of
//! rooms over Nostr; reimplementing it in Rust to avoid a subprocess would be
//! trading a supervised process for a pile of protocol we would have to keep in
//! step with somebody else's. So this supervises `node bin/kithmoot-agent.mjs`
//! and speaks its newline-delimited-JSON "stdio brain" seam.
//!
//! The parts that DECIDE anything — the relay lint, the link policy gate, the
//! codec — live in [`crate::world_room`] and are tested without Node. What is
//! here is process lifecycle and plumbing, which is the part that cannot be
//! unit-tested honestly, so it is kept as small and as boring as possible.
//!
//! Native only: there is no subprocess on the web, and there is no chat on the
//! web either (world-chat spec §6).

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crate::world_room::{
    check_link_policy_json, encode_command, lint_relays, InboundLine, KeeperCommand, KeeperEvent,
    OutboundLine, RoomConfig, RoomError, RosterMember, WorldRoom,
};

/// How long to wait for the child's `ready` line before calling the start
/// failed. Generous: it has to reach relays over the network first.
const READY_TIMEOUT: Duration = Duration::from_secs(30);

/// How long to let the child exit cleanly after `leave` before killing it.
const STOP_GRACE: Duration = Duration::from_secs(3);

/// Where the kithmoot checkout lives. An env var rather than a hardcoded path,
/// because it is a sibling checkout on a developer's machine and a shipped
/// directory on a server, and neither should be baked into the binary.
const KITHMOOT_DIR_ENV: &str = "AXENSTAX_KITHMOOT_DIR";

/// A room backed by a supervised `kithmoot-agent` process.
pub struct KithMootKeeper {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    events: Option<Receiver<KeeperEvent>>,
    roster: Vec<RosterMember>,
    /// The room URL the child reported at `ready`, for `/room` to print.
    url: Option<String>,
}

impl Default for KithMootKeeper {
    fn default() -> Self {
        Self::new()
    }
}

impl KithMootKeeper {
    pub fn new() -> Self {
        KithMootKeeper {
            child: None,
            stdin: None,
            events: None,
            roster: Vec::new(),
            url: None,
        }
    }

    /// The room link, once attached.
    // BRIDGE: no caller yet — the `/room` status/invite commands (spec §4.5)
    // are not wired up in this build. Kept ready for them.
    #[allow(dead_code)]
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    /// Where the kithmoot checkout is. `None` with an explanation is better than
    /// a guess: a wrong path produces a confusing ENOENT from `node`.
    fn kithmoot_dir() -> Result<std::path::PathBuf, RoomError> {
        match std::env::var(KITHMOOT_DIR_ENV) {
            Ok(d) if !d.trim().is_empty() => {
                let path = std::path::PathBuf::from(d);
                if path.join("bin/kithmoot-agent.mjs").is_file() {
                    Ok(path)
                } else {
                    Err(RoomError::Failed(format!(
                        "{KITHMOOT_DIR_ENV} does not look like a kithmoot checkout \
                         (no bin/kithmoot-agent.mjs under it)."
                    )))
                }
            }
            _ => Err(RoomError::Failed(format!(
                "Set {KITHMOOT_DIR_ENV} to your kithmoot checkout to attach a room. \
                 It must be built first (npm install && npm run build:lib) — the CLI \
                 runs from dist/, not from source."
            ))),
        }
    }
}

impl WorldRoom for KithMootKeeper {
    fn start(&mut self, cfg: &RoomConfig) -> Result<(), RoomError> {
        // Both gates before we spend a process on it.
        //
        // The relay list is passed EXPLICITLY on the command line and is never
        // allowed to fall back: kithmoot's own DEFAULT_RELAYS begins with our
        // relay, so "pass nothing" would route a family's conversation through
        // AxeNStax infrastructure. See world_room::FORBIDDEN_RELAY_HOST.
        let relays = lint_relays(&cfg.relays).map_err(RoomError::Relays)?;
        for dropped in &relays.dropped {
            log::warn!("room: dropping relay {} — {}", dropped.url, dropped.reason);
        }
        // The link must require agents to be owned by a member. Upstream that
        // rule is off by default, so it is ours to insist on.
        if let Some((_, fragment)) = cfg.link.split_once('#') {
            check_link_policy_json(fragment).map_err(RoomError::Link)?;
        } else {
            return Err(RoomError::Link(crate::world_room::LinkError::Malformed));
        }

        let dir = Self::kithmoot_dir()?;

        let mut cmd = Command::new("node");
        cmd.current_dir(&dir)
            .arg("bin/kithmoot-agent.mjs")
            .arg("join")
            .arg(&cfg.link)
            .arg("--name")
            .arg(&cfg.name)
            .arg("--brain")
            .arg("stdio")
            // No --listen: a text-only room never constructs the WebRTC media
            // factory, so this pulls in no media stack at all.
            .arg("--quiet")
            .arg("--relays")
            .arg(relays.kept.join(","))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            RoomError::Failed(format!(
                "could not start the room process ({e}). Is node installed and on PATH?"
            ))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| RoomError::Failed("no stdin on the room process".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| RoomError::Failed("no stdout on the room process".to_string()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| RoomError::Failed("no stderr on the room process".to_string()))?;

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(ev) = crate::world_room::decode_event(&line) {
                    // A closed channel means the keeper was dropped; stop
                    // reading rather than spinning on a dead receiver.
                    if tx.send(ev).is_err() {
                        break;
                    }
                }
            }
        });

        // stderr MUST be drained even though we only log it. A child whose
        // stderr pipe fills up blocks on write, and a blocked child stops
        // answering the room — a failure that looks like "chat mysteriously
        // stopped" rather than like a full pipe.
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                log::debug!("kithmoot-agent: {line}");
            }
        });

        // Wait for `ready`, which the child always writes first.
        let deadline = Instant::now() + READY_TIMEOUT;
        let mut ready_url = None;
        while Instant::now() < deadline {
            match rx.try_recv() {
                Ok(KeeperEvent::Ready { url, .. }) => {
                    ready_url = Some(url);
                    break;
                }
                Ok(KeeperEvent::Error { message }) => {
                    let _ = child.kill();
                    // A closed room is terminal upstream and must never be
                    // retried; anything else might be transient.
                    return Err(if message.to_lowercase().contains("closed") {
                        RoomError::Closed
                    } else {
                        RoomError::Failed(message)
                    });
                }
                Ok(_) => {}
                Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(50)),
                Err(TryRecvError::Disconnected) => break,
            }
        }

        let Some(url) = ready_url else {
            let _ = child.kill();
            return Err(RoomError::NoReady);
        };

        log::info!("room: attached as '{}'", cfg.name);
        self.child = Some(child);
        self.stdin = Some(stdin);
        self.events = Some(rx);
        self.url = Some(url);
        self.roster.clear();
        Ok(())
    }

    fn post(&mut self, line: &OutboundLine) -> Result<(), RoomError> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Err(RoomError::Failed("no room attached".to_string()));
        };
        let cmd = encode_command(&KeeperCommand::Say {
            text: line.text.clone(),
        });
        stdin
            .write_all(cmd.as_bytes())
            .and_then(|()| stdin.flush())
            .map_err(|e| RoomError::Failed(format!("room process stopped listening: {e}")))
    }

    fn poll(&mut self) -> Vec<InboundLine> {
        let mut out = Vec::new();
        let Some(rx) = self.events.as_ref() else {
            return out;
        };
        loop {
            match rx.try_recv() {
                Ok(KeeperEvent::Chat(line)) => out.push(line),
                Ok(KeeperEvent::Roster { participants }) => self.roster = participants,
                Ok(KeeperEvent::Error { message }) => {
                    log::warn!("room: {message}");
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        out
    }

    fn members(&self) -> Vec<RosterMember> {
        self.roster.clone()
    }

    fn stop(&mut self) -> Result<(), RoomError> {
        // Ask to leave, so the room sees a departure rather than a timeout.
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = stdin.write_all(encode_command(&KeeperCommand::Leave).as_bytes());
            let _ = stdin.flush();
        }
        // Dropping stdin closes the pipe, which is what tells the child we are
        // finished; without it a well-behaved child waits for more commands.
        self.stdin = None;

        if let Some(mut child) = self.child.take() {
            let deadline = Instant::now() + STOP_GRACE;
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    // Out of patience, or we cannot tell: kill it. A room
                    // process outliving the game would keep answering the link.
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                }
            }
        }
        self.events = None;
        self.roster.clear();
        self.url = None;
        Ok(())
    }
}

impl Drop for KithMootKeeper {
    /// A room process must never outlive the game that started it — it holds a
    /// delegation that keeps answering the link.
    fn drop(&mut self) {
        if self.child.is_some() {
            let _ = self.stop();
        }
    }
}
