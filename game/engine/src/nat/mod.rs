//! Getting two home machines to talk directly.
//!
//! Everything here is about **addresses**, not about the game: gather the
//! addresses this machine might be reachable at, punch a hole through the
//! router toward the peer's, and race a QUIC connect across all of them. The
//! game traffic that follows is the existing transport, unchanged.
//!
//! Spec: `docs/superpowers/specs/2026-09-06-online-play-by-contact-design.md` §4.
#![cfg(not(target_arch = "wasm32"))]

pub mod candidates;
pub mod punch;
pub mod stun;
pub mod upnp;
