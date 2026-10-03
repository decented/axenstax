//! Owner-only NIP-46 bunker pairing harness (the live-Signet BOUNDARY).
//!
//! Everything up to the relay handshake is solo-verifiable; the handshake itself
//! needs the owner's Signet app on a phone to approve the connect + sign requests.
//!
//! 1. On the phone: Signet → export a `bunker://<pubkey>?relay=wss://...&secret=...` URI.
//! 2. On this host:
//!    `cargo run --bin bunker_pair -- 'bunker://<pubkey>?relay=wss://relay.trotters.cc&secret=...'`
//! 3. Approve the connect + signature prompts in Signet.
//!
//! Success prints the user pubkey (npub) and a bunker-signed, locally-verified event.
//! This is the exact flow the S3 signer wrapper will own; proving it live is bucket 3.

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use native_build_spike::native;
    use nostr::prelude::ToBech32;

    let uri = match std::env::args().nth(1) {
        Some(u) => u,
        None => {
            eprintln!(
                "usage: bunker_pair '<bunker:// URI from Signet>'\n\
                 (the live pair needs the owner's phone — see this file's header)"
            );
            std::process::exit(2);
        }
    };

    println!("Pairing with bunker (approve the prompts in Signet) ...");
    let (pubkey, event) = native::pair_bunker_and_sign(&uri).await?;
    println!("OK — paired with {}", pubkey.to_bech32()?);
    println!("    bunker-signed event id {} verified locally", event.id);
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
