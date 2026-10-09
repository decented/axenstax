//! Canonical invite capability context. Cosmetic names never enter its digest.
use super::{Error, Hash, Result, codec::frame};
use nostr::{FromBech32, PublicKey, ToBech32, Url};

pub const CONTEXT_LIMIT: usize = 18 * 1024;
const ROOT_DOMAIN: &str = "AxeNStax/handover/root-context/v1";

/// Validated public root binding; bearer is passed separately to key derivation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootContext {
    runtime: Hash,
    persona: Hash,
    expiry: u64,
    relays: Vec<String>,
    digest: Hash,
}
impl RootContext {
    pub fn new(
        runtime: Hash,
        persona_npub: &str,
        expiry: u64,
        relays: &[String],
        now: u64,
    ) -> Result<Self> {
        PublicKey::from_slice(&runtime).map_err(|_| Error::Invalid("runtime curve point"))?;
        let persona = canonical_npub(persona_npub)?;
        if expiry < now {
            return Err(Error::Invalid("expired invite"));
        }
        if relays.is_empty() || relays.len() > 8 {
            return Err(Error::Limit("relay count"));
        }
        let mut total = 0usize;
        for relay in relays {
            validate_relay(relay)?;
            total = total
                .checked_add(relay.len())
                .ok_or(Error::Limit("relay bytes"))?;
            if total > 16_384 {
                return Err(Error::Limit("relay bytes"));
            }
        }
        let mut relays = relays.to_vec();
        relays.sort();
        relays.dedup();
        let mut out = Vec::new();
        frame(&mut out, ROOT_DOMAIN.as_bytes());
        out.extend_from_slice(&2u32.to_be_bytes());
        out.extend_from_slice(&runtime);
        out.extend_from_slice(&persona);
        out.extend_from_slice(&expiry.to_be_bytes());
        out.extend_from_slice(&(relays.len() as u32).to_be_bytes());
        for relay in &relays {
            frame(&mut out, relay.as_bytes());
        }
        if out.len() > CONTEXT_LIMIT {
            return Err(Error::Limit("root context"));
        }
        Ok(Self {
            runtime,
            persona,
            expiry,
            relays,
            digest: super::hash(&out),
        })
    }
    pub fn runtime(&self) -> Hash {
        self.runtime
    }
    pub fn persona(&self) -> Hash {
        self.persona
    }
    pub fn expiry(&self) -> u64 {
        self.expiry
    }
    pub fn relays(&self) -> &[String] {
        &self.relays
    }
    pub fn digest(&self) -> Hash {
        self.digest
    }
}

pub fn canonical_npub(value: &str) -> Result<Hash> {
    let key = PublicKey::from_bech32(value).map_err(|_| Error::Invalid("npub"))?;
    if key.to_bech32().map_err(|_| Error::Invalid("npub"))? != value {
        return Err(Error::Invalid("noncanonical npub"));
    }
    PublicKey::from_slice(&key.to_bytes()).map_err(|_| Error::Invalid("persona curve point"))?;
    Ok(key.to_bytes())
}

pub fn validate_relay(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 2048 {
        return Err(Error::Limit("relay URL"));
    }
    let url = Url::parse(value).map_err(|_| Error::Invalid("relay URL"))?;
    if url.scheme() != "wss"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || value.trim() != value
        || !value.starts_with("wss://")
    {
        return Err(Error::Invalid("relay URL"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handover_context_order_is_canonical_but_path_is_not_rewritten() {
        let k = nostr::Keys::generate();
        let npub = k.public_key().to_bech32().unwrap();
        let rs = vec![
            "wss://relay.example/b".into(),
            "wss://relay.example/a".into(),
        ];
        let a = RootContext::new(k.public_key().to_bytes(), &npub, 900, &rs, 100).unwrap();
        let mut reversed = rs.clone();
        reversed.reverse();
        reversed.push(rs[0].clone());
        assert_eq!(
            a,
            RootContext::new(k.public_key().to_bytes(), &npub, 900, &reversed, 100).unwrap()
        );
        let b = RootContext::new(
            k.public_key().to_bytes(),
            &npub,
            900,
            &["wss://relay.example/a/".into()],
            100,
        )
        .unwrap();
        assert_ne!(a.digest(), b.digest());
        assert!(
            RootContext::new(
                k.public_key().to_bytes(),
                &npub,
                900,
                &vec![rs[0].clone(); 9],
                100
            )
            .is_err()
        );
    }
    #[test]
    fn handover_context_rejects_identity_and_url_substitution() {
        let k = nostr::Keys::generate();
        let npub = k.public_key().to_bech32().unwrap();
        for bad in [
            "https://relay.example",
            "wss://a@relay.example",
            "wss://relay.example/#x",
            " wss://relay.example",
        ] {
            assert!(validate_relay(bad).is_err());
        }
        assert!(canonical_npub(&k.public_key().to_hex()).is_err());
        assert!(
            RootContext::new([255; 32], &npub, 900, &["wss://relay.example".into()], 100).is_err()
        );
        assert!(
            RootContext::new(
                k.public_key().to_bytes(),
                &npub,
                99,
                &["wss://relay.example".into()],
                100
            )
            .is_err()
        );
    }
}
