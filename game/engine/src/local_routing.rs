//! D3 — how this machine runs its own world, and which kind of session it is in.
//!
//! [`LocalRouting`] is the routing option a `GameState` is built with (native
//! CLI `--solo-hosted`, `lib.rs::run`; tests inject it). The default,
//! [`LocalRouting::Standalone`], is ordinary single-player: the client's own
//! `GameState::tick` runs the whole simulation and no server exists.
//! [`LocalRouting::SoloHosted`] runs single-player through a local,
//! transport-less `HostedServer` (`RemoteTransport::None`, `HostWorld::Lent`)
//! with the same one fixed tick sequence a LAN host uses (Spec 01 §4.1.3).
//!
//! [`SessionMode`] separates *authority presence* from *network exposure*: a
//! solo-hosted world has an authority (the server) but is not networked, so
//! split-screen, in-world Trials and local chat echo behave as single-player.
// The web build keeps its standalone path: it parses no CLI and never routes
// through a local server.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

/// The injected single-player routing option.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LocalRouting {
    /// Ordinary single-player, no server (the default).
    #[default]
    Standalone,
    /// `--solo-hosted`: single-player through a local, transport-less server.
    SoloHosted,
}

/// The native command-line flag that selects [`LocalRouting::SoloHosted`].
pub(crate) const SOLO_HOSTED_FLAG: &str = "--solo-hosted";

/// Why `--solo-hosted` and `--no-lend` cannot be combined.
pub(crate) const NO_LEND_CONFLICT: &str = "--solo-hosted cannot be combined with --no-lend: \
     solo-hosted lends this world to its local server, and --no-lend asks for a second copy";

/// The routing option the native command line asks for. `--solo-hosted`
/// together with `--no-lend` is an error (a startup error in `lib.rs::run`).
pub(crate) fn from_args(args: &[String]) -> Result<LocalRouting, String> {
    let solo = args.iter().any(|a| a == SOLO_HOSTED_FLAG);
    let no_lend = args.iter().any(|a| a == "--no-lend");
    match (solo, no_lend) {
        (true, true) => Err(NO_LEND_CONFLICT.to_string()),
        (true, false) => Ok(LocalRouting::SoloHosted),
        (false, _) => Ok(LocalRouting::Standalone),
    }
}

/// Which kind of session this machine is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionMode {
    /// Own world, no server (default single-player, and the web build).
    LocalStandalone,
    /// Own world through a local, network-silent server (`--solo-hosted`).
    LocalAuthority,
    /// Own world hosted for others (LAN, Fray, online, Keep).
    ExternallyHosted,
    /// Someone else's world.
    Joined,
}

impl SessionMode {
    /// From what this machine holds: a joined client, and/or a hosted server
    /// (`Some(local_only)`).
    pub(crate) fn classify(joined: bool, hosted_local_only: Option<bool>) -> Self {
        match (joined, hosted_local_only) {
            (true, _) => Self::Joined,
            (false, None) => Self::LocalStandalone,
            (false, Some(true)) => Self::LocalAuthority,
            (false, Some(false)) => Self::ExternallyHosted,
        }
    }

    /// Is another machine part of this session (or able to join it)? What
    /// "networked" gates mean: split-screen seats, an arena Trial's refusal,
    /// world chat over the server rather than a local echo.
    pub(crate) fn networked(self) -> bool {
        matches!(self, Self::ExternallyHosted | Self::Joined)
    }
}

impl crate::GameState {
    /// This machine's session kind (see [`SessionMode`]).
    pub(crate) fn session_mode(&self) -> SessionMode {
        SessionMode::classify(
            self.remote_client.is_some(),
            self.hosted_server.as_ref().map(crate::hosted_server::HostedServer::local_only),
        )
    }

    /// Is this session networked (hosting for others, or joined)? A
    /// solo-hosted authority is not.
    pub(crate) fn networked(&self) -> bool {
        self.session_mode().networked()
    }

    /// The hosted server, only when it is exposed to other machines (a LAN,
    /// Fray, online or Keep host): room and hosting commands act on this one.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn network_host(&self) -> Option<&crate::hosted_server::HostedServer> {
        self.hosted_server.as_ref().filter(|hs| !hs.local_only())
    }

    /// [`Self::network_host`], mutably.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn network_host_mut(&mut self) -> Option<&mut crate::hosted_server::HostedServer> {
        self.hosted_server.as_mut().filter(|hs| !hs.local_only())
    }

    /// The ONE completed-local-world-entry step: once a world this machine
    /// owns has loaded (the Loading→Playing hand-off, or the blocking
    /// `initial_load` fallback), a `--solo-hosted` game starts its local
    /// authority here, before the first tick. Every entry path reaches one
    /// of those two (lobby create/open, Workshop, Trials, harness loads).
    ///
    /// Leaves alone a session with a server already staged (LAN / Fray /
    /// online host, started before the load), a joined session, and the
    /// default routing. Idempotent. A start that fails leaves for the lobby
    /// with the reason, touching nothing on disk, and returns `false`: no
    /// live world and no authority are left behind.
    pub(crate) fn enter_local_world(&mut self) -> bool {
        if self.local_routing != LocalRouting::SoloHosted
            || self.hosted_server.is_some()
            || self.remote_client.is_some()
            || !self.persists_locally()
        {
            return true;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            match crate::hosted_server::HostedServer::start_solo(self.world_name.clone(), self.biome_gen.seed) {
                Ok(mut hs) => {
                    self.align_local_authority(&mut hs);
                    log::info!("Solo-hosted: '{}' runs through a local server", self.world_name);
                    self.hosted_server = Some(hs);
                }
                Err(why) => {
                    log::error!("solo-hosted authority for '{}' did not start: {why}", self.world_name);
                    self.leave_world_with_notice(
                        crate::world_exit::SaveChoice::Abandon,
                        format!("This world couldn't be started: {why}. Nothing was changed."),
                    );
                    return false;
                }
            }
        }
        true
    }

    /// Hand a local-only authority this client's rules and seats: its play
    /// mode, difficulty, explosives and fire spread (the client's own, which
    /// the pause menu and scenarios may have changed since the meta was
    /// written), and every local seat. Run at entry and before every lent
    /// tick of a local-only server.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn align_local_authority(&self, hs: &mut crate::hosted_server::HostedServer) {
        if !hs.local_only() {
            return;
        }
        hs.align_local_rules(
            self.play_mode,
            &self.difficulty,
            self.explosives_enabled,
            self.fire_spread_enabled,
        );
        let slots: Vec<(glam::Vec3, f32, f32, f32)> = self
            .players
            .iter()
            .map(|p| (p.player.pos, p.camera.yaw, p.camera.pitch, p.combat.health))
            .collect();
        hs.sync_local_slots(&slots);
        let seated: Vec<bool> = self.players.iter().map(crate::player_slot::PlayerSlot::seated).collect();
        hs.sync_local_seats(&seated);
    }

    /// `reset_for_world_change`: a previous world's local-only authority
    /// never carries into the next world (reopen gets fresh authority
    /// state). A staged LAN / online server is kept — it was started for the
    /// world about to load.
    pub(crate) fn drop_local_authority(&mut self) {
        if self.hosted_server.as_ref().is_some_and(crate::hosted_server::HostedServer::local_only) {
            self.hosted_server = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_default_routing_is_standalone() {
        assert_eq!(from_args(&args(&["axenstax-engine"])), Ok(LocalRouting::Standalone));
        assert_eq!(from_args(&args(&["axenstax-engine", "--no-lend"])), Ok(LocalRouting::Standalone));
        assert_eq!(LocalRouting::default(), LocalRouting::Standalone);
    }

    #[test]
    fn solo_hosted_is_opt_in_and_refuses_no_lend() {
        assert_eq!(from_args(&args(&["x", "--solo-hosted"])), Ok(LocalRouting::SoloHosted));
        assert_eq!(
            from_args(&args(&["x", "--solo-hosted", "--no-lend"])),
            Err(NO_LEND_CONFLICT.to_string())
        );
        assert_eq!(
            from_args(&args(&["x", "--no-lend", "--solo-hosted"])),
            Err(NO_LEND_CONFLICT.to_string())
        );
    }

    #[test]
    fn authority_presence_is_not_network_exposure() {
        use SessionMode::*;
        assert_eq!(SessionMode::classify(false, None), LocalStandalone);
        assert_eq!(SessionMode::classify(false, Some(true)), LocalAuthority);
        assert_eq!(SessionMode::classify(false, Some(false)), ExternallyHosted);
        assert_eq!(SessionMode::classify(true, None), Joined);
        assert!(!LocalStandalone.networked());
        assert!(!LocalAuthority.networked());
        assert!(ExternallyHosted.networked());
        assert!(Joined.networked());
    }
}
