//! The encrypted setup handshake two players exchange over public relays before
//! their machines talk directly.
//!
//! A relay sees two runtime pubkeys and a timestamp. It does not see who the
//! players are, which world is involved, or what addresses they gave each other
//! — all of that is inside NIP-44 ciphertext (CLAUDE.md red line 3). Nothing
//! here is a directory: an offer is addressed to exactly one runtime key that
//! the joiner already had, from a contact or an invite.
//!
//! Spec: `docs/superpowers/specs/2026-09-06-online-play-by-contact-design.md`
//! §3.2.
#![cfg(not(target_arch = "wasm32"))]

pub mod payload;
pub mod relay_client;
pub mod verify;
