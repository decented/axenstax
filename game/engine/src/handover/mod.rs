//! Explicit checkpoint handover between player-run native hosts.
//!
//! This core supplies bounded encodings, verification, transfer and durable
//! phase boundaries. It has no UI, automatic publication or host-start hook.
//! Prepared records are inert; source retirement precedes committed publication.

pub mod certificate;
mod codec;
pub mod context;
pub mod crypto;
pub mod durable;
pub mod manifest;
pub mod phase;
pub mod stored;
pub mod stream;

pub const PLAINTEXT_LIMIT: usize = 16_384;
pub const CIPHERTEXT_LIMIT: usize = 21_936;
pub const EVENT_LIMIT: usize = 24 * 1024;
pub const RELAY_FRAME_LIMIT: usize = 32 * 1024;
pub const PROOF_LIMIT: usize = 4096;
pub const STORED_KIND: u16 = 1099;
pub type Hash = [u8; 32];
pub type Id = [u8; 16];
pub type Result<T> = std::result::Result<T, Error>;

/// Deliberately does not carry keys, bearers, ciphertext or untrusted frame text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Invalid(&'static str),
    Limit(&'static str),
    State(&'static str),
    Io,
    UncertainDurability,
    Cancelled,
    Deadline,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "handover: {self:?}")
    }
}
impl std::error::Error for Error {}

pub fn hash(bytes: &[u8]) -> Hash {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).into()
}
