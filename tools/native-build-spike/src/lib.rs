//! # native-build-spike — B0 / S0 eval spike (throwaway de-risking harness)
//!
//! Proves the four load-bearing assumptions behind the native cloud-save story
//! *before* any of the real Forgesworn crates (S3 signer wrapper, S4 `stash-rs`)
//! are written. See `docs/goals/native-build-solo.md` §"Phase S0".
//!
//! 1. **They compile together pinned.** `nostr` 0.44.3, `nostr-sdk` 0.44.1,
//!    `nostr-connect` 0.44.1, `nostr-blossom` 0.44.0 resolve and build as one tree.
//! 2. **The WASM-gating idiom holds.** Every native crate lives ONLY under
//!    `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]` (see `Cargo.toml`)
//!    and every reference is `#[cfg(not(target_arch = "wasm32"))]`-gated, so
//!    `cargo build --target wasm32-unknown-unknown` never compiles them. The whole
//!    crate (lib + both bins) builds for wasm32 because the only thing left on that
//!    target is [`shim_marker`]. This is the exact pattern the engine already uses
//!    (`game/engine/Cargo.toml`) and that bucket-3 integration will reuse.
//! 3. **nostr-sdk signs with a locally generated key** ([`native::sign_with_local_key`]).
//! 4. **A blob round-trips to a public Blossom server** ([`native::blossom_roundtrip`]),
//!    using the same blob layout `@forgesworn/stash` uses, so S4 can mirror it.
//!
//! The live Signet-bunker pair (a real `nostr-connect` ↔ bunker handshake) is the
//! hard boundary — it needs the owner's phone. The glue + a one-command harness
//! (`src/bin/bunker_pair.rs`) are here; the live pairing is left to the owner.

/// Compiles on **every** target, native and wasm32. Its existence on a
/// `wasm32-unknown-unknown` build is the positive proof that gating the native
/// crates didn't gate the crate itself out of existence — the WASM bundle keeps
/// building while the native deps are absent.
pub fn shim_marker() -> &'static str {
    "native-build-spike"
}

#[cfg(not(target_arch = "wasm32"))]
pub mod native {
    //! All native-only logic. Excluded wholesale from the wasm32 build, which is
    //! why none of the `nostr*` / `tokio` / `base64` crates touch the WASM bundle.

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use nostr::hashes::sha256::Hash as Sha256Hash;
    use nostr::hashes::Hash as _;
    use nostr::prelude::*;
    use nostr_blossom::prelude::*;

    /// Primal's public Blossom server (BUD-01 auth + BUD-02 upload/get, public read).
    pub const PRIMAL_BLOSSOM: &str = "https://blossom.primal.net";
    /// Our own Blossom bridge on the Trotters relay infra — a fallback target.
    pub const TROTTERS_BLOSSOM: &str = "https://blossom.trotters.cc";

    /// Spike point #3 — sign a Nostr event with a freshly generated key and
    /// verify its Schnorr signature locally. Offline, deterministic enough for CI.
    pub fn sign_with_local_key() -> Result<Event, Box<dyn std::error::Error>> {
        let keys = Keys::generate();
        let event = EventBuilder::text_note("native-build-spike: hello from a generated key")
            .sign_with_keys(&keys)?;
        event.verify()?;
        Ok(event)
    }

    /// NIP-44 v2 encrypt-to-self round-trip with a generated key. This is the
    /// exact crypto primitive `stash-rs` (S4) mirrors from `@forgesworn/stash`'s
    /// blob layer: item bytes are base64'd, NIP-44-encrypted to one's own pubkey,
    /// and the UTF-8 of the ciphertext string is what lands on Blossom.
    pub async fn nip44_self_roundtrip(payload: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        let keys = Keys::generate();
        let pk = keys.public_key();
        let pt_b64 = BASE64.encode(payload);
        let ciphertext = keys.nip44_encrypt(&pk, &pt_b64).await?;
        let decrypted_b64 = keys.nip44_decrypt(&pk, &ciphertext).await?;
        let decrypted = BASE64.decode(decrypted_b64)?;
        assert_eq!(decrypted, payload, "NIP-44 self round-trip must be byte-identical");
        Ok(())
    }

    /// Build the Stash-shaped blob for `payload`: `utf8( NIP44_encrypt_to_self( base64(payload) ) )`.
    /// Returns `(blob_bytes, local_sha256)` — `local_sha256` is the Blossom content
    /// address the upload must echo back (the Stash integrity check).
    pub async fn stash_blob(
        keys: &Keys,
        payload: &[u8],
    ) -> Result<(Vec<u8>, Sha256Hash), Box<dyn std::error::Error>> {
        let pk = keys.public_key();
        let ciphertext = keys.nip44_encrypt(&pk, &BASE64.encode(payload)).await?;
        let blob = ciphertext.into_bytes(); // UTF-8 bytes of the ciphertext string
        let hash = Sha256Hash::hash(&blob);
        Ok((blob, hash))
    }

    /// Spike point #4 — a live Blossom round-trip with a generated key, faithful to
    /// the Stash protocol: encrypt-to-self → upload ciphertext → verify the server's
    /// returned hash matches the local hash → download by hash → verify bytes →
    /// decrypt → assert the payload survived byte-identical.
    ///
    /// Network test. Returns the content-addressed hash on success.
    pub async fn blossom_roundtrip(
        server: &str,
        payload: &[u8],
    ) -> Result<String, Box<dyn std::error::Error>> {
        let keys = Keys::generate();
        let client = BlossomClient::new(server.parse()?);

        let (blob, local_hash) = stash_blob(&keys, payload).await?;

        // Upload with persona-key BUD-01 auth (Some(&keys) → signed kind-24242).
        let descriptor: BlobDescriptor = client
            .upload_blob(
                blob.clone(),
                Some("application/octet-stream".to_string()),
                None,
                Some(&keys),
            )
            .await?;

        // Stash integrity check: the server must store exactly what we hashed.
        if descriptor.sha256 != local_hash {
            return Err(format!(
                "blossom stored a different hash than uploaded ({} != {})",
                descriptor.sha256, local_hash
            )
            .into());
        }
        if descriptor.size as usize != blob.len() {
            return Err(format!(
                "blossom reported size {} but blob is {} bytes",
                descriptor.size,
                blob.len()
            )
            .into());
        }

        // Public read — no auth needed; None needs a turbofish for the generic signer.
        let fetched: Vec<u8> = client
            .get_blob(descriptor.sha256, None, None, None::<&Keys>)
            .await?;

        if Sha256Hash::hash(&fetched) != descriptor.sha256 {
            return Err("blossom returned bytes that do not match the requested hash".into());
        }
        if fetched != blob {
            return Err("downloaded blob differs from uploaded blob".into());
        }

        // Decrypt back to the original payload (closes the loop).
        let ciphertext = String::from_utf8(fetched)?;
        let pk = keys.public_key();
        let plaintext_b64 = keys.nip44_decrypt(&pk, &ciphertext).await?;
        let recovered = BASE64.decode(plaintext_b64)?;
        if recovered != payload {
            return Err("decrypted payload differs from the original".into());
        }

        Ok(descriptor.sha256.to_string())
    }

    /// Spike point #2 (live-bunker boundary) — parse a NIP-46 bunker URI and build
    /// a [`nostr_connect::prelude::NostrConnect`] signer for it. The construction +
    /// `get_public_key()` + `sign_event()` calls are all here; what can't be done
    /// solo is the actual relay handshake with a *real* Signet bunker (needs the
    /// owner's phone). `src/bin/bunker_pair.rs` drives this end to end.
    pub async fn pair_bunker_and_sign(
        bunker_uri: &str,
    ) -> Result<(PublicKey, Event), Box<dyn std::error::Error>> {
        use nostr_connect::prelude::*;
        use std::time::Duration;

        let uri = NostrConnectURI::parse(bunker_uri)?;
        let app_keys = Keys::generate(); // ephemeral client identity for the NIP-46 session
        let signer = NostrConnect::new(uri, app_keys, Duration::from_secs(60), None)?;

        let user_pubkey = signer.get_public_key().await?;
        let event = EventBuilder::text_note("native-build-spike: signed via Signet bunker")
            .sign(&signer)
            .await?;
        event.verify()?;
        signer.shutdown().await;
        Ok((user_pubkey, event))
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn marker_is_stable() {
        assert_eq!(shim_marker(), "native-build-spike");
    }

    #[test]
    fn sign_with_local_key_verifies() {
        let event = native::sign_with_local_key().expect("sign + verify should succeed");
        assert!(event.verify_signature());
    }

    #[tokio::test]
    async fn nip44_self_roundtrip_is_byte_identical() {
        // Binary payload incl. NUL + high bytes — proves the base64 wrap is binary-safe.
        let payload: Vec<u8> = (0u16..=511).map(|n| (n % 256) as u8).collect();
        native::nip44_self_roundtrip(&payload)
            .await
            .expect("NIP-44 self round-trip");
    }

    #[tokio::test]
    async fn stash_blob_hash_matches_blossom_content_address() {
        // The local hash we compute must equal sha256 of the exact uploaded bytes.
        let keys = nostr::prelude::Keys::generate();
        let (blob, hash) = native::stash_blob(&keys, b"hello stash blob")
            .await
            .expect("blob build");
        use nostr::hashes::Hash as _;
        assert_eq!(nostr::hashes::sha256::Hash::hash(&blob), hash);
    }

    /// Live network test — ignored by default so `cargo test` stays green offline.
    /// Run with: `cargo test -- --ignored` (or use the `blossom_roundtrip` bin).
    #[tokio::test]
    #[ignore = "network: round-trips a blob to a public Blossom server"]
    async fn blossom_roundtrip_live() {
        let payload = b"native-build-spike live blossom round-trip";
        let hash = native::blossom_roundtrip(native::PRIMAL_BLOSSOM, payload)
            .await
            .expect("live blossom round-trip");
        assert_eq!(hash.len(), 64, "sha256 hex is 64 chars");
    }
}
