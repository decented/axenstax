//! The live invitation's bearer (H1): the secret `online_admission`'s
//! `Accept(ByInvite)` checks. A source session holds the online host's
//! current one; the recipient receives it in the `Frame::Bearer` disclosure
//! (P2) and stores it only in the imported world's Activated record (H3).
//! Never logged: `Debug` prints nothing of it and there is no `Display`.
//! Each copy is overwritten with zeros when it is dropped.
use crate::handover::Id;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bearer(Id);

impl Bearer {
    pub fn new(bytes: Id) -> Self {
        Self(bytes)
    }
    /// The secret itself, for the key derivation and the admission check.
    pub fn expose(&self) -> &Id {
        &self.0
    }
}

impl std::fmt::Debug for Bearer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Bearer([redacted])")
    }
}

impl Drop for Bearer {
    fn drop(&mut self) {
        for b in self.0.iter_mut() {
            // SAFETY: `b` is a valid, aligned, exclusive reference.
            unsafe { std::ptr::write_volatile(b, 0) };
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handover_bearer_debug_never_prints_the_secret() {
        let b = Bearer::new([0xAB; 16]);
        let shown = format!("{b:?}");
        assert!(!shown.contains("171") && !shown.to_lowercase().contains("ab"), "the bearer was printed: {shown}");
        assert_eq!(b.expose(), &[0xAB; 16]);
    }
}
