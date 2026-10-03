//! The pairing state machine (spec D3, D5, D6, D13). Pure: the service drives
//! it from the UI thread and the ack-waiter thread.
//!
//! Idle → `start` (fresh app key + fresh challenge, D5) → Waiting (QR + link)
//! → ack → Code (the verification code, large, D6) → Continue → the `Grant` is
//! handed out to persist. Nothing is persisted, fetched or used before
//! Continue. Cancel from any state discards everything; a retry is a new
//! `start` and so a new key and challenge. A late ack for a cancelled attempt
//! is dropped by its generation number.

use nostr::Keys;

use crate::signet::contacts_wire::{
    build_pairing_uri, format_pairing_code, pairing_code, web_carrier, Ack, Capability, Directory,
    PairingError,
};

use super::store::Grant;

/// What the Friends row draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    /// Showing the QR / link (D3).
    Waiting { carrier: String },
    /// Showing the verification code (D6).
    Code { code: String },
    /// No ack inside the window (D13).
    TimedOut,
}

struct Attempt {
    app_keys: Keys,
    challenge: String,
    ack: Option<Ack>,
}

/// One pairing attempt at a time.
pub struct PairingFlow {
    generation: u64,
    attempt: Option<Attempt>,
    phase: Phase,
}

impl Default for PairingFlow {
    fn default() -> Self {
        Self { generation: 0, attempt: None, phase: Phase::Idle }
    }
}

/// What the waiter thread needs to start.
pub struct Ticket {
    pub generation: u64,
    pub app_keys: Keys,
    pub challenge: String,
    pub relay: String,
}

/// The pairing request fields this game sends.
pub struct Request<'a> {
    pub app_name: &'a str,
    pub caps: &'a [Capability],
    pub relay: &'a str,
    pub web_host: &'a str,
}

impl PairingFlow {
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// Begin a new attempt with a fresh key and challenge, discarding any
    /// earlier one.
    pub fn start(&mut self, req: &Request, app_keys: Keys, challenge: String, now: u64) -> Result<Ticket, PairingError> {
        self.cancel();
        let uri = build_pairing_uri(
            &app_keys.public_key().to_hex(),
            req.app_name,
            req.caps,
            Directory::Owner,
            req.relay,
            now,
            &challenge,
        )?;
        self.phase = Phase::Waiting { carrier: web_carrier(req.web_host, &uri) };
        self.attempt = Some(Attempt { app_keys: app_keys.clone(), challenge: challenge.clone(), ack: None });
        Ok(Ticket { generation: self.generation, app_keys, challenge, relay: req.relay.to_owned() })
    }

    /// The waiter found an ack. Ignored unless it belongs to the live attempt
    /// and that attempt is still waiting. Returns whether it was taken.
    pub fn on_ack(&mut self, generation: u64, ack: Ack) -> bool {
        if generation != self.generation || !matches!(self.phase, Phase::Waiting { .. }) {
            return false;
        }
        let Some(a) = self.attempt.as_mut() else { return false };
        let app_hex = a.app_keys.public_key().to_hex();
        let Some(code) = pairing_code(&app_hex, &a.challenge, &ack.grant_id, &ack.rail_pubkey) else {
            // No code can be shown → the ack can never be confirmed; drop it.
            self.cancel();
            self.phase = Phase::TimedOut;
            return false;
        };
        a.ack = Some(ack);
        self.phase = Phase::Code { code: format_pairing_code(&code) };
        true
    }

    /// The waiter gave up (D13).
    pub fn on_timeout(&mut self, generation: u64) {
        if generation == self.generation && matches!(self.phase, Phase::Waiting { .. }) {
            self.attempt = None;
            self.phase = Phase::TimedOut;
        }
    }

    /// The player typed the code into Signet and pressed Continue: the ONLY
    /// way a grant leaves this machine's memory for disk.
    pub fn continue_pressed(&mut self, now: u64) -> Option<Grant> {
        if !matches!(self.phase, Phase::Code { .. }) {
            return None;
        }
        let a = self.attempt.take()?;
        let ack = a.ack?;
        self.generation += 1;
        self.phase = Phase::Idle;
        Some(Grant::from_ack(&ack, &a.app_keys, now))
    }

    /// Cancel / mismatch / dismiss: discard the attempt (key, challenge, ack).
    pub fn cancel(&mut self) {
        self.generation += 1;
        self.attempt = None;
        self.phase = Phase::Idle;
    }
}

/// A fresh 128-bit challenge, 32 lowercase hex chars.
pub fn fresh_challenge() -> Option<String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).ok()?;
    Some(hex::encode(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPS: [Capability; 1] = [Capability::ReadDirectory];

    fn req() -> Request<'static> {
        Request { app_name: "Test App", caps: &CAPS, relay: "wss://relay.example.com", web_host: "signet.example" }
    }

    fn ack_for(challenge: &str) -> Ack {
        Ack {
            v: 2,
            grant_id: "1".repeat(32),
            rail_pubkey: Keys::generate().public_key().to_hex(),
            projection_tag: "2".repeat(32),
            proposal_tag: "3".repeat(32),
            relay: "wss://other.example.com".into(),
            granted_capabilities: CAPS.to_vec(),
            max_staleness_seconds: 21_600,
            challenge: challenge.into(),
        }
    }

    #[test]
    fn nothing_is_handed_out_before_continue_and_the_code_is_shown() {
        let mut f = PairingFlow::default();
        let t = f.start(&req(), Keys::generate(), fresh_challenge().unwrap(), 1000).unwrap();
        let Phase::Waiting { carrier } = f.phase().clone() else { panic!() };
        assert!(carrier.starts_with("https://signet.example/?pair=1&v=2&app="));
        assert!(carrier.contains("&dir=owner&"), "D2: owner pairing only");
        assert_eq!(f.continue_pressed(1001), None, "no grant while waiting");

        assert!(f.on_ack(t.generation, ack_for(&t.challenge)));
        let Phase::Code { code } = f.phase().clone() else { panic!("code screen expected") };
        assert_eq!(code.len(), 7, "formatted `123 456`: {code}");

        let g = f.continue_pressed(1002).expect("Continue persists the grant");
        assert_eq!(g.relay, "wss://other.example.com", "the ack's relay, D4");
        assert_eq!(g.app_secret_hex, t.app_keys.secret_key().to_secret_hex());
        assert_eq!(*f.phase(), Phase::Idle);
        assert_eq!(f.continue_pressed(1003), None, "only once");
    }

    #[test]
    fn cancel_discards_and_a_late_ack_is_ignored() {
        let mut f = PairingFlow::default();
        let t = f.start(&req(), Keys::generate(), fresh_challenge().unwrap(), 1000).unwrap();
        f.cancel();
        assert!(!f.on_ack(t.generation, ack_for(&t.challenge)), "ack for a cancelled attempt");
        assert_eq!(*f.phase(), Phase::Idle);
        assert_eq!(f.continue_pressed(1001), None);
    }

    #[test]
    fn cancel_on_the_code_screen_persists_nothing_and_a_retry_is_fresh() {
        let mut f = PairingFlow::default();
        let t1 = f.start(&req(), Keys::generate(), fresh_challenge().unwrap(), 1000).unwrap();
        assert!(f.on_ack(t1.generation, ack_for(&t1.challenge)));
        f.cancel();
        assert_eq!(f.continue_pressed(1001), None);
        let t2 = f.start(&req(), Keys::generate(), fresh_challenge().unwrap(), 1002).unwrap();
        assert_ne!(t1.challenge, t2.challenge);
        assert_ne!(t1.app_keys.public_key(), t2.app_keys.public_key());
        assert_ne!(t1.generation, t2.generation);
        assert!(!f.on_ack(t1.generation, ack_for(&t1.challenge)), "old attempt's ack");
    }

    #[test]
    fn timeout_shows_the_limit_hint_state() {
        let mut f = PairingFlow::default();
        let t = f.start(&req(), Keys::generate(), fresh_challenge().unwrap(), 1000).unwrap();
        f.on_timeout(t.generation + 7);
        assert!(matches!(f.phase(), Phase::Waiting { .. }), "stale timeout ignored");
        f.on_timeout(t.generation);
        assert_eq!(*f.phase(), Phase::TimedOut);
    }
}
