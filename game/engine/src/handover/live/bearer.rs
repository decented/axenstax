//! The live invitation's bearer (H1): the secret `online_admission`'s
//! `Accept(ByInvite)` checks. A source session holds the online host's
//! current one; the recipient receives it in the `Frame::Bearer` disclosure
//! (P2) and stores it only in the imported world's `handover/` records
//! (H3, amended in CP10): the Prepared receipt, written before its
//! PreparedAck, then the Activated record. No archive carries it.
//! Never logged: `Debug` prints nothing of it and there is no `Display`.
//! Each copy is overwritten with zeros when it is dropped.
use crate::handover::Id;

/// CP8 review LOW-4: no `Serialize`/`Deserialize` of its own: the only
/// serialisation is `wire`'s private `bearer_field` module, used by
/// `Frame::Bearer` alone, so nothing else can serialise it.
#[derive(Clone, PartialEq, Eq)]
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
        zeroize::Zeroize::zeroize(&mut self.0);
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
