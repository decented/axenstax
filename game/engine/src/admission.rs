// The seam's only consumer today is the native `ws_transport` accept loop, so
// these are "unused" on the wasm build — allow it (future policies + the
// Operator Console in Spec B consume them too).
#![allow(dead_code)]
//! Admission control seam (Spec A §6).
//!
//! Today the only policy is a hard player cap. The seam exists so future
//! policies — Queue, OneInOneOut, BitcoinGated (pay-to-enter, on the
//! non-custodial settlement canon ADR-004 / Spec 6 §1.5) — drop in *here*
//! instead of editing the accept path. Pure + unit-tested; the caller (the
//! server accept loop) supplies the live counts.

/// How the server decides whether to admit a new connection at capacity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AdmissionPolicy {
    /// Reject once the cap is reached — today's behaviour.
    #[default]
    HardCap,
    // Future variants (Spec A §6): Queue, OneInOneOut, BitcoinGated. Each will
    // take more context (a queue position, a payment proof) — which is exactly
    // why admission is a seam and not an inline `if`.
}

/// The decision for one prospective join.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admission {
    /// Let the connection through.
    Admit,
    /// Refuse it, with a human-readable reason (surfaced to the client where the
    /// transport allows).
    Reject(String),
}

/// Decide whether to admit a connection. `current` = remote players already
/// connected; `max` = the configured cap.
pub fn decide(policy: AdmissionPolicy, current: usize, max: usize) -> Admission {
    match policy {
        AdmissionPolicy::HardCap => {
            if current >= max {
                Admission::Reject("Server full".to_string())
            } else {
                Admission::Admit
            }
        }
    }
}

/// Hand a reserved seat back to the shared remote-player counter. Saturating:
/// a double release (or one for a seat that was never reserved, e.g. an
/// in-process test transport) can never wrap the counter to "permanently full"
/// (engine audit 2026-06-04, D).
#[cfg(not(target_arch = "wasm32"))]
pub fn release_seat(current: &std::sync::atomic::AtomicUsize) {
    use std::sync::atomic::Ordering;
    let _ = current.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_seat_never_underflows() {
        let c = std::sync::atomic::AtomicUsize::new(1);
        release_seat(&c);
        release_seat(&c);
        assert_eq!(c.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[test]
    fn hardcap_admits_below_cap() {
        assert_eq!(decide(AdmissionPolicy::HardCap, 0, 8), Admission::Admit);
        assert_eq!(decide(AdmissionPolicy::HardCap, 7, 8), Admission::Admit);
    }

    #[test]
    fn hardcap_rejects_at_and_over_cap() {
        assert_eq!(
            decide(AdmissionPolicy::HardCap, 8, 8),
            Admission::Reject("Server full".into())
        );
        assert_eq!(
            decide(AdmissionPolicy::HardCap, 9, 8),
            Admission::Reject("Server full".into())
        );
    }

    #[test]
    fn hardcap_zero_cap_rejects_everyone() {
        assert_eq!(
            decide(AdmissionPolicy::HardCap, 0, 0),
            Admission::Reject("Server full".into())
        );
    }

    #[test]
    fn default_policy_is_hardcap() {
        assert_eq!(AdmissionPolicy::default(), AdmissionPolicy::HardCap);
    }
}
