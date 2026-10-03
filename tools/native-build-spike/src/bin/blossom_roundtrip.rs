//! One-command live Blossom round-trip harness (spike point #4).
//!
//!   cargo run --bin blossom_roundtrip                 # default: Primal's Blossom
//!   cargo run --bin blossom_roundtrip -- <server-url> # any BUD-02 server
//!
//! Generates a throwaway key, encrypts a small payload to self (Stash blob layout),
//! uploads it, verifies the content address, downloads + decrypts, and prints the
//! hash. Exit 0 = the native Nostr+Blossom stack round-trips against a real server.

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use native_build_spike::native;

    let server = std::env::args()
        .nth(1)
        .unwrap_or_else(|| native::PRIMAL_BLOSSOM.to_string());
    let payload = b"native-build-spike: download-and-play cloud-save round-trip";

    println!("Blossom round-trip against {server} ...");
    let hash = native::blossom_roundtrip(&server, payload).await?;
    println!("OK — blob stored + verified + decrypted. content-address sha256 = {hash}");
    Ok(())
}

// wasm32 never runs harnesses; keep the crate building on that target.
#[cfg(target_arch = "wasm32")]
fn main() {}
