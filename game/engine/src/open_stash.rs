//! open-stash — the PUBLIC, unencrypted, npub-signed content store.
//!
//! Counterpart to the private Stash (`cloud.js`, NIP-44 encrypted-to-self):
//! open-stash is how a creator SHARES content (mods / scenarios). The transport
//! now lives entirely in `@forgesworn/beacon` (`window.AxeBeacon`, glued by
//! `beacon.js`): plaintext blobs to Blossom + a public kind-30820 manifest event
//! to the relay; anyone who FOLLOWS your npub can list + download it. One
//! mechanism delivers both official (the AxeNStax npub) and community mods.
//!
//! KEY PROPERTY: reading open-stash content needs only SIGNATURE VERIFICATION
//! (done at the relay/JS layer), NOT the NIP-44 `capable()` gate — so it is
//! robust even where private cross-device Stash is not.
//!
//! This module is now THIN: it owns the official-pubkey seam + pubkey
//! normalisation, and marshals bytes across the Beacon WASM externs. The
//! manifest SHAPE / parse / followed-store that used to live here moved into
//! Beacon; the engine no longer builds Nostr events itself.

// Items that only the WASM Stash column consumes carry a target-scoped
// `cfg_attr(not(wasm32), allow(dead_code))`; the kind constant is a kept reference.

/// Parameterised-replaceable Nostr event kind for an open-stash index (public,
/// cleartext manifest). Distinct from the private Stash's kind-30819 so a
/// cleartext index is never confused with an encrypted one. Beacon owns the
/// event build/parse now; this is kept as the canonical kind reference.
#[allow(dead_code)] // canonical reference only — Beacon owns the event build/parse (see doc)
pub const OPEN_STASH_MANIFEST_KIND: u32 = 30820;

/// The AxeNStax official pubkey (hex), followed by default so official content
/// appears "already there". This is the real semi-burner AxeNStax identity
/// (npub1pwu2jxv68c7zgz3h3ncmt2thj3w7euzlclfsy8mxl2ntqjwt2h7shgnm2m) created
/// 2026-06-06 for the Prague Stash Column delivery — a PUBLIC key, safe to ship.
/// The matching secret lives outside the repo (`~/.config/axenstax/`, mode
/// 600) and is read only by the offline publish tool. Rotate to a secure key
/// store before non-demo use (see the Stash Column goal, Phase 5).
pub const OFFICIAL_AXENSTAX_PUBKEY: &str =
    "0bb8a9199a3e3c240a378cf1b5a977945decf05fc7d3021f66faa6b049cb55fd";

/// Beacon item content-type tags. ONE mechanism (the kind-30820 manifest +
/// Blossom blobs) carries MULTIPLE content types, discriminated by this tag at
/// the item level (`BeaconBrowseItem.content_type`). The Stash Column lists
/// items typed by these tags and dispatches per type. Tag #1 is the Workshop
/// appearance override-set (a skin); tag #2 is a scenario/challenge def. Kept as
/// named constants so the publish call site, the browse filter, and the column
/// classifier can't drift apart on a string literal.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // consumed by the web Stash column only (dead on native)
pub const CONTENT_TYPE_OVERRIDE_SET: &str = "override-set";
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // consumed by the web Stash column only (dead on native)
pub const CONTENT_TYPE_SCENARIO: &str = "scenario";

/// The kind of content a Beacon item carries, decoded from its content-type tag.
/// The column renders a per-kind action: a skin is **Adopted**, a scenario is
/// **Played**.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StashItemKind {
    /// A Workshop appearance override-set (skin).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // consumed by the web Stash column only (dead on native)
    Skin,
    /// A scenario / challenge def.
    Scenario,
}

/// Classify a Beacon item's content-type tag into a known [`StashItemKind`].
/// Unknown / future tags return `None` (forward-compat: a newer publisher may
/// carry a type this build can't render — the column simply skips it rather than
/// crashing or mis-actioning it).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // consumed by the web Stash column only (dead on native)
pub fn classify_content_type(content_type: &str) -> Option<StashItemKind> {
    match content_type {
        CONTENT_TYPE_OVERRIDE_SET => Some(StashItemKind::Skin),
        CONTENT_TYPE_SCENARIO => Some(StashItemKind::Scenario),
        _ => None,
    }
}

/// Normalise a Nostr pubkey to canonical hex (64 lowercase hex chars). Returns
/// `None` for anything else (wrong length / non-hex). npub bech32 decoding is a
/// display-boundary concern handled before this; the store keys on hex (what
/// relay filters use). Reused by the browse path (npub→hex).
pub fn normalize_pubkey(pubkey: &str) -> Option<String> {
    let s = pubkey.trim();
    if s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(s.to_ascii_lowercase())
    } else {
        None
    }
}

/// Config seam for the official AxeNStax pubkey. Returns the real semi-burner
/// AxeNStax identity (Prague delivery). The seam stays so a config/env override
/// can later swap in a rotated production key without touching call sites.
pub fn official_axenstax_pubkey() -> String {
    OFFICIAL_AXENSTAX_PUBKEY.to_string()
}

// ---------------------------------------------------------------------------
// WASM transport bridge — window.AxeBeacon, glued by beacon.js.
//
// The browser owns the I/O: Blossom PUT/GET (capped, signed) + relay
// publish/subscribe + Signet signing + the kind-30820 manifest build/parse.
// This Rust side just marshals bytes + strings across the Beacon externs.
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
use js_sys::Uint8Array;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = axenstax_openstash_available)]
    fn js_beacon_available() -> bool;
    #[wasm_bindgen(js_name = axenstax_openstash_publish, catch)]
    async fn js_beacon_publish(
        name: String,
        content_type: String,
        bytes: Uint8Array,
    ) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = axenstax_openstash_list, catch)]
    async fn js_beacon_list(pubkey: String) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = axenstax_openstash_download, catch)]
    async fn js_beacon_download(blob_hash: String) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = axenstax_openstash_follow, catch)]
    async fn js_beacon_follow(pubkey: String) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = axenstax_openstash_unfollow, catch)]
    async fn js_beacon_unfollow(pubkey: String) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = axenstax_openstash_following, catch)]
    async fn js_beacon_following() -> Result<JsValue, JsValue>;
}

#[cfg(target_arch = "wasm32")]
fn jsval_err(e: JsValue) -> String {
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}

/// One browse-listing item as emitted by beacon.js (`axenstax_openstash_list`).
/// The extra `updated` field beacon.js emits is ignored (serde skips unknown
/// fields by default — do NOT add `deny_unknown_fields`).
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, serde::Deserialize)]
pub struct BeaconBrowseItem {
    pub name: String,
    #[serde(rename = "blobHash")]
    pub blob_hash: String,
    #[serde(rename = "contentType")]
    pub content_type: String,
    #[serde(default)]
    pub size: u64,
}

#[cfg(target_arch = "wasm32")]
pub fn beacon_available() -> bool {
    js_beacon_available()
}

/// Publish one named blob to YOUR open-stash via Beacon. Returns the Blossom
/// blob hash (the download key). Beacon handles the manifest publish.
#[cfg(target_arch = "wasm32")]
pub async fn beacon_publish(name: &str, content_type: &str, bytes: &[u8]) -> Result<String, String> {
    let arr = Uint8Array::new_with_length(bytes.len() as u32);
    arr.copy_from(bytes);
    js_beacon_publish(name.into(), content_type.into(), arr)
        .await
        .map_err(jsval_err)?
        .as_string()
        .ok_or_else(|| "beacon publish: no hash".to_string())
}

/// List a pubkey's shared items (empty string / `None` → the caller's own).
#[cfg(target_arch = "wasm32")]
pub async fn beacon_list(pubkey_hex: &str) -> Result<Vec<BeaconBrowseItem>, String> {
    let v = js_beacon_list(pubkey_hex.into()).await.map_err(jsval_err)?;
    let txt = v
        .as_string()
        .ok_or_else(|| "beacon list: not a string".to_string())?;
    serde_json::from_str(&txt).map_err(|e| format!("parse beacon items: {e}"))
}

/// Public Blossom GET of a shared blob by hash (no auth, no decryption).
#[cfg(target_arch = "wasm32")]
pub async fn beacon_download(blob_hash: &str) -> Result<Vec<u8>, String> {
    let v = js_beacon_download(blob_hash.into()).await.map_err(jsval_err)?;
    if v.is_null() || v.is_undefined() {
        return Err("beacon download: empty".to_string());
    }
    Ok(Uint8Array::new(&v).to_vec())
}

#[cfg(target_arch = "wasm32")]
pub async fn beacon_follow(pubkey_hex: &str) -> Result<(), String> {
    js_beacon_follow(pubkey_hex.into()).await.map_err(jsval_err)?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub async fn beacon_unfollow(pubkey_hex: &str) -> Result<(), String> {
    js_beacon_unfollow(pubkey_hex.into())
        .await
        .map_err(jsval_err)?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub async fn beacon_following() -> Result<Vec<String>, String> {
    let v = js_beacon_following().await.map_err(jsval_err)?;
    let txt = v
        .as_string()
        .ok_or_else(|| "beacon following: not a string".to_string())?;
    serde_json::from_str(&txt).map_err(|e| format!("parse following: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_pubkey_is_the_real_axenstax_key() {
        // The seam returns the canonical const, which must be a real 64-hex
        // pubkey — NOT the old all-zeros placeholder. (Public key; safe in src.)
        assert_eq!(official_axenstax_pubkey(), OFFICIAL_AXENSTAX_PUBKEY);
        assert_eq!(official_axenstax_pubkey().len(), 64);
        assert!(
            official_axenstax_pubkey().bytes().all(|b| b.is_ascii_hexdigit()),
            "official pubkey must be valid lowercase hex"
        );
        assert_ne!(
            official_axenstax_pubkey(),
            "0".repeat(64),
            "the all-zeros placeholder must have been replaced with the real key"
        );
        // And it must normalise cleanly through the pubkey boundary.
        assert_eq!(normalize_pubkey(&official_axenstax_pubkey()), Some(official_axenstax_pubkey()));
    }

    #[test]
    fn normalize_pubkey_still_validates() {
        assert!(normalize_pubkey(&"a".repeat(64)).is_some());
        assert!(normalize_pubkey("nope").is_none());
    }

    #[test]
    fn content_type_tags_classify_to_kinds() {
        assert_eq!(
            classify_content_type(CONTENT_TYPE_OVERRIDE_SET),
            Some(StashItemKind::Skin)
        );
        assert_eq!(
            classify_content_type(CONTENT_TYPE_SCENARIO),
            Some(StashItemKind::Scenario)
        );
        // The two tags are distinct, and an unknown/future tag is skipped.
        assert_ne!(CONTENT_TYPE_OVERRIDE_SET, CONTENT_TYPE_SCENARIO);
        assert_eq!(classify_content_type("world-save"), None);
        assert_eq!(classify_content_type(""), None);
    }
}
