//! Resource-pack networking + cache primitives (Spec 03 §11.6 / §11.7).
//!
//! P4b: content hashing (cache key + integrity) and pure LRU eviction over the
//! hash-keyed cache. The fetch + decode + apply paths (native download, WASM
//! HTTP) build on these. The native cache lives at `resource_pack_cache/` beside
//! the worlds folder; WASM caches in browser storage (P4d). Keeping the policy
//! pure (no I/O) makes it fully unit-testable on every platform.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn default_web_resolution() -> u32 {
    16
}

/// One entry in the web pack index (`/static/packs/index.json`) — what the web
/// picker lists and what tells the WASM fetcher which `<key>.png` files to pull.
/// The web index is an aggregated discovery doc (one fetch lists every pack);
/// native packs are discovered by scanning the disk instead.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub struct WebPackDescriptor {
    pub name: String,
    #[serde(default = "default_web_resolution")]
    pub resolution: u32,
    /// Texture keys the pack overrides (no `.png`), e.g. `blocks/stone`.
    #[serde(default)]
    pub files: Vec<String>,
}

/// Parse `/static/packs/index.json` leniently — malformed JSON yields an empty
/// list rather than failing the picker (Spec 03 §11.4: a pack only ships what it
/// overrides; the index only lists what exists).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn parse_pack_index(json: &str) -> Vec<WebPackDescriptor> {
    serde_json::from_str(json).unwrap_or_default()
}

/// Lowercase-hex SHA-256 of a pack artifact — the cache key and the integrity
/// value carried in [`crate::protocol::ResourcePackSuggestPacket::sha256`]
/// (Spec 03 §11.7: "cache entries are keyed by SHA-256 hash, not by name").
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn pack_hash(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let mut hex = String::with_capacity(64);
    for b in digest {
        hex.push_str(&format!("{b:02x}"));
    }
    hex
}

/// True when `bytes` hashes to `expected` (case-insensitive hex). An empty
/// `expected` means "no integrity declared" and accepts — a suggest packet may
/// omit the hash for a trusted same-origin pack (Spec 03 §11.7 verifies when one
/// is present; a mismatch triggers re-download).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn verify_integrity(bytes: &[u8], expected: &str) -> bool {
    let expected = expected.trim();
    expected.is_empty() || pack_hash(bytes).eq_ignore_ascii_case(expected)
}

/// What the client does with a server-suggested pack after the player decides
/// (Spec 03 §11.6 step 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub enum SuggestionOutcome {
    /// Accepted — fetch + load it as the active pack.
    Apply,
    /// Declined a non-required pack — keep the current configuration.
    KeepCurrent,
    /// Declined a *required* pack — disconnect from the server.
    Disconnect,
}

/// Resolve a player's Accept/Decline on a server-suggested pack. Accepting always
/// applies; declining a `required` pack disconnects, else it keeps the current
/// pack (Spec 03 §11.6 step 4).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn resolve_suggestion_outcome(required: bool, accepted: bool) -> SuggestionOutcome {
    match (accepted, required) {
        (true, _) => SuggestionOutcome::Apply,
        (false, true) => SuggestionOutcome::Disconnect,
        (false, false) => SuggestionOutcome::KeepCurrent,
    }
}

/// Build a [`crate::protocol::ResourcePackSuggestPacket`] from explicit fields —
/// `Some` only when a URL is given (an empty URL means "no pack configured").
/// Pure; the env reader wraps it.
pub fn build_suggestion(
    name: String,
    url: String,
    sha256: String,
    size_bytes: u64,
    required: bool,
) -> Option<crate::protocol::ResourcePackSuggestPacket> {
    if url.trim().is_empty() {
        return None;
    }
    Some(crate::protocol::ResourcePackSuggestPacket { name, url, sha256, size_bytes, required })
}

/// The operator-configured server pack suggestion, read from the environment
/// (the dedicated server has no config file yet). `None` unless `AXENSTAX_PACK_URL`
/// is set. Companions: `AXENSTAX_PACK_NAME`, `AXENSTAX_PACK_SHA256`,
/// `AXENSTAX_PACK_SIZE`, `AXENSTAX_PACK_REQUIRED` (`1`/`true`).
#[cfg(not(target_arch = "wasm32"))]
pub fn configured_suggestion() -> Option<crate::protocol::ResourcePackSuggestPacket> {
    let url = std::env::var("AXENSTAX_PACK_URL").ok()?;
    let name = std::env::var("AXENSTAX_PACK_NAME").unwrap_or_else(|_| "Server pack".to_string());
    let sha256 = std::env::var("AXENSTAX_PACK_SHA256").unwrap_or_default();
    let size_bytes = std::env::var("AXENSTAX_PACK_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let required = std::env::var("AXENSTAX_PACK_REQUIRED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    build_suggestion(name, url, sha256, size_bytes, required)
}

// BRIDGE: the 500 MB default cache budget (Spec 03 §11.7) had no consumer yet
// (the cache-apply path that would read it isn't built), so the truly-unused
// constant was removed here rather than kept as dead weight — re-add it
// (`pub const CACHE_LIMIT_BYTES: u64 = 500 * 1024 * 1024;`) when that path
// lands and calls `lru_evictions` with a real budget.

/// One cached pack, for eviction accounting.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub struct CacheEntry {
    pub hash: String,
    pub size_bytes: u64,
    /// Last-access stamp — only its ordering matters (older = smaller).
    pub last_access: u64,
}

/// The hashes to evict so the cache total fits `limit_bytes`, **least-recently-
/// used first** (Spec 03 §11.7). Returns `[]` when already under budget. Ties on
/// `last_access` break by `hash` for deterministic eviction.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn lru_evictions(entries: &[CacheEntry], limit_bytes: u64) -> Vec<String> {
    let mut running: u64 = entries.iter().map(|e| e.size_bytes).sum();
    if running <= limit_bytes {
        return Vec::new();
    }
    let mut by_age: Vec<&CacheEntry> = entries.iter().collect();
    by_age.sort_by(|a, b| a.last_access.cmp(&b.last_access).then_with(|| a.hash.cmp(&b.hash)));
    let mut evict = Vec::new();
    for e in by_age {
        if running <= limit_bytes {
            break;
        }
        evict.push(e.hash.clone());
        running -= e.size_bytes;
    }
    evict
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_hash_is_lowercase_hex_sha256() {
        // Known vector: SHA-256("hello").
        let h = pack_hash(b"hello");
        assert_eq!(h, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    }

    #[test]
    fn verify_integrity_is_case_insensitive_and_lenient_on_empty() {
        let bytes = b"hello";
        let upper = pack_hash(bytes).to_uppercase();
        assert!(verify_integrity(bytes, &upper), "case-insensitive match");
        assert!(verify_integrity(bytes, "  2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824  "));
        assert!(!verify_integrity(bytes, "deadbeef"), "wrong hash rejected");
        assert!(verify_integrity(bytes, ""), "empty expected → accept (no integrity declared)");
    }

    fn entry(hash: &str, size: u64, t: u64) -> CacheEntry {
        CacheEntry { hash: hash.to_string(), size_bytes: size, last_access: t }
    }

    #[test]
    fn lru_evictions_drops_oldest_until_under_budget() {
        let cache = vec![entry("a", 100, 1), entry("b", 100, 2), entry("c", 100, 3)];
        assert_eq!(lru_evictions(&cache, 300), Vec::<String>::new(), "exactly at budget → none");
        assert_eq!(lru_evictions(&cache, 350), Vec::<String>::new(), "under budget → none");
        assert_eq!(lru_evictions(&cache, 250), vec!["a"], "evict oldest only");
        assert_eq!(lru_evictions(&cache, 150), vec!["a", "b"], "evict two oldest");
        assert_eq!(lru_evictions(&cache, 0), vec!["a", "b", "c"], "evict all to fit 0");
    }

    #[test]
    fn parse_pack_index_reads_descriptors_and_is_lenient() {
        let json = r#"[{"name":"Vivid","resolution":16,"files":["blocks/stone","blocks/dirt"]},
                       {"name":"Crisp","resolution":32,"files":[]}]"#;
        let v = parse_pack_index(json);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].name, "Vivid");
        assert_eq!(v[0].files, vec!["blocks/stone".to_string(), "blocks/dirt".to_string()]);
        assert_eq!(v[1].resolution, 32);
        assert!(parse_pack_index("garbage").is_empty(), "bad JSON → empty list, never a failure");
        assert!(parse_pack_index("").is_empty());
    }

    #[test]
    fn suggestion_outcome_honours_required_on_decline() {
        use SuggestionOutcome::*;
        assert_eq!(resolve_suggestion_outcome(true, true), Apply, "accepted always applies");
        assert_eq!(resolve_suggestion_outcome(false, true), Apply);
        assert_eq!(resolve_suggestion_outcome(true, false), Disconnect, "required + declined");
        assert_eq!(resolve_suggestion_outcome(false, false), KeepCurrent, "optional + declined");
    }

    #[test]
    fn build_suggestion_requires_a_url() {
        assert!(build_suggestion("P".into(), "".into(), "".into(), 0, false).is_none());
        assert!(build_suggestion("P".into(), "   ".into(), "".into(), 0, false).is_none(), "blank URL");
        let s = build_suggestion("P".into(), "https://x/pack.json".into(), "ab".into(), 9, true).unwrap();
        assert_eq!(s.url, "https://x/pack.json");
        assert_eq!(s.size_bytes, 9);
        assert!(s.required);
    }

    #[test]
    fn parse_pack_index_defaults_resolution_to_16() {
        let v = parse_pack_index(r#"[{"name":"X","files":["blocks/sand"]}]"#);
        assert_eq!(v[0].resolution, 16, "absent resolution → 16");
    }

    #[test]
    fn lru_evictions_breaks_ties_by_hash() {
        // Same last_access → deterministic order by hash.
        let cache = vec![entry("z", 100, 5), entry("a", 100, 5)];
        assert_eq!(lru_evictions(&cache, 100), vec!["a"], "tie broken by hash, 'a' before 'z'");
    }
}
