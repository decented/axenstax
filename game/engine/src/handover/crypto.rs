//! Standard HKDF/NIP-44 around the canonical application certificate.
use super::{
    Error, Hash, Id, Result,
    certificate::Core,
    codec::frame,
    context::RootContext,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use nostr::hashes::Hash as _;
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, PublicKey, SecretKey, Tag, Timestamp};

pub fn recipient(root: &RootContext, bearer: &Id, predecessor: Option<Hash>) -> Result<Keys> {
    let mut salt = Vec::new();
    frame(&mut salt, b"AxeNStax/handover/bearer-kdf/v1");
    salt.extend_from_slice(&root.digest());
    let prk = nostr::util::hkdf::extract(&salt, bearer);
    derive_scalar(|counter| {
        let mut info = Vec::new();
        frame(
            &mut info,
            if predecessor.is_some() {
                b"AxeNStax/handover/hop-recipient-secret/v1"
            } else {
                b"AxeNStax/handover/recipient-secret/v1"
            },
        );
        info.extend_from_slice(&root.digest());
        if let Some(prev) = predecessor {
            info.extend_from_slice(&prev);
        }
        info.extend_from_slice(&counter.to_be_bytes());
        nostr::util::hkdf::expand(prk.as_byte_array(), &info, 32)
    })
}
/// C excludes the bearer. Retirement identity additionally binds the opaque
/// root recipient, so same-second fresh bearers do not share a retired context.
pub fn root_capability(root: &RootContext, bearer: &Id) -> Result<Hash> {
    let mut bytes = Vec::new();
    frame(&mut bytes, b"AxeNStax/handover/retired-capability/v1");
    bytes.extend_from_slice(&root.digest());
    bytes.extend_from_slice(&recipient(root, bearer, None)?.public_key().to_bytes());
    Ok(super::hash(&bytes))
}
fn derive_scalar(mut expand: impl FnMut(u32) -> Vec<u8>) -> Result<Keys> {
    for counter in 0..256 {
        let bytes = expand(counter);
        if bytes.len() != 32 {
            return Err(Error::Invalid("HKDF output"));
        }
        if let Ok(sk) = SecretKey::from_slice(&bytes) {
            return Ok(Keys::new(sk));
        }
    }
    Err(Error::Invalid("recipient scalar exhaustion"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Prepared,
    Committed,
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    v: u32,
    phase: u8,
    core: String,
    ready: String,
    source_attestation: String,
    successor_attestation: String,
    prepared: Option<String>,
}
/// Only `open` produces this value: its fields are private to this file
/// (CP5 review MEDIUM-1), so no caller can assert a verification.
#[derive(Clone, Debug)]
pub struct VerifiedPrepared {
    core: Core,
    event_hash: Hash,
}
impl VerifiedPrepared {
    pub fn core(&self) -> &Core {
        &self.core
    }
    pub fn event_hash(&self) -> Hash {
        self.event_hash
    }
}
/// Only `open`/`open_candidates` produce this value (fields private to
/// this file, CP5 review MEDIUM-1).
#[derive(Clone, Debug)]
pub struct VerifiedCommit {
    core: Core,
    event_hash: Hash,
    prepared_hash: Hash,
}
impl VerifiedCommit {
    pub fn core(&self) -> &Core {
        &self.core
    }
    pub fn event_hash(&self) -> Hash {
        self.event_hash
    }
    pub fn prepared_hash(&self) -> Hash {
        self.prepared_hash
    }
    /// Test builds only: the certificate prefix tests' synthetic commits.
    /// No production code can construct one.
    #[cfg(test)]
    pub(crate) fn for_test(core: Core, event_hash: Hash, prepared_hash: Hash) -> Self {
        Self { core, event_hash, prepared_hash }
    }
}
/// CP13b (D2): a source's Aborted after it disclosed the bearer: the same
/// envelope and kind as Committed, inner `phase: 2` (never readable as
/// Committed's `1`), bound like Committed to the transfer and context (the
/// exact core) and to the Prepared hash. Only `open_candidates` builds it.
#[derive(Clone, Debug)]
pub struct VerifiedAbort {
    core: Core,
    event_hash: Hash,
    prepared_hash: Hash,
}
impl VerifiedAbort {
    pub fn core(&self) -> &Core {
        &self.core
    }
    pub fn event_hash(&self) -> Hash {
        self.event_hash
    }
    pub fn prepared_hash(&self) -> Hash {
        self.prepared_hash
    }
}
#[derive(Clone, Debug)]
pub enum Verified {
    Prepared(VerifiedPrepared),
    Committed(VerifiedCommit),
    Aborted(VerifiedAbort),
}

fn ready_bytes(core: &Core, challenge: Id) -> Vec<u8> {
    let mut v = Vec::new();
    frame(&mut v, b"AxeNStax/handover/ready/v1");
    v.extend_from_slice(&core.digest());
    v.extend_from_slice(&core.manifest);
    v.extend_from_slice(&core.transfer);
    v.extend_from_slice(&challenge);
    v
}
pub async fn sign_ready(successor: &Keys, core: &Core, challenge: Id) -> Result<Event> {
    if successor.public_key().to_bytes() != core.successor_runtime || challenge == [0; 16] {
        return Err(Error::Invalid("Ready recipient/challenge"));
    }
    EventBuilder::new(
        Kind::Custom(super::STORED_KIND),
        STANDARD.encode(ready_bytes(core, challenge)),
    )
    .custom_created_at(Timestamp::from(core.issued_at))
    .sign(successor)
    .await
    .map_err(|_| Error::Invalid("Ready signing"))
}
fn decode_base64(value: &str, limit: usize) -> Result<Vec<u8>> {
    if value.len() > limit.saturating_mul(4).div_ceil(3) + 4 {
        return Err(Error::Limit("base64 field"));
    }
    let bytes = STANDARD
        .decode(value)
        .map_err(|_| Error::Invalid("base64"))?;
    if bytes.len() > limit {
        return Err(Error::Limit("decoded field"));
    }
    if STANDARD.encode(&bytes) != value {
        return Err(Error::Invalid("noncanonical base64"));
    }
    Ok(bytes)
}
fn proof(value: &str) -> Result<Event> {
    if value.len() > super::PROOF_LIMIT {
        return Err(Error::Limit("proof"));
    }
    Event::from_json(value).map_err(|_| Error::Invalid("proof event"))
}
pub(crate) fn verify_ready(ev: &Event, core: &Core, expected_challenge: Option<Id>) -> Result<()> {
    if ev.as_json().len() > super::PROOF_LIMIT
        || ev.pubkey.to_bytes() != core.successor_runtime
        || ev.kind != Kind::Custom(super::STORED_KIND)
        || !ev.tags.is_empty()
        || ev.created_at.as_secs().abs_diff(core.issued_at) > 120
        || ev.verify().is_err()
    {
        return Err(Error::Invalid("Ready proof"));
    }
    let bytes = decode_base64(&ev.content, 512)?;
    let mut r = super::codec::Reader::new(&bytes, 512)?;
    r.label("AxeNStax/handover/ready/v1")?;
    if r.array::<32>()? != core.digest()
        || r.array::<32>()? != core.manifest
        || r.array::<16>()? != core.transfer
    {
        return Err(Error::Invalid("Ready binding"));
    }
    let challenge = r.array::<16>()?;
    r.end()?;
    if challenge == [0; 16] || expected_challenge.is_some_and(|expected| expected != challenge) {
        return Err(Error::Invalid("Ready challenge"));
    }
    Ok(())
}
fn attestation(ev: &Event, runtime: Hash, persona: Hash, core: &Core, now: u64) -> Result<()> {
    if ev.as_json().len() > super::PROOF_LIMIT {
        return Err(Error::Limit("attestation"));
    }
    let runtime = PublicKey::from_slice(&runtime).map_err(|_| Error::Invalid("runtime"))?;
    let att = crate::server_identity::attestation::verify(ev, &runtime, Timestamp::from(now))
        .map_err(|_| Error::Invalid("attestation"))?;
    if att.role.as_deref() != Some(crate::runtime_identity::PLAYER_ROLE)
        || att.operator.to_bytes() != persona
        || core.expires_at > att.valid_until.as_secs()
        || core.issued_at < att.valid_from.as_secs()
    {
        return Err(Error::Invalid("attestation binding"));
    }
    // This verifies persona binding, not an allowed-kinds1099 grant. Authority
    // comes from the invitation root or verified committed predecessor.
    Ok(())
}

pub struct Seal<'a> {
    pub source: &'a Keys,
    pub recipient: &'a Keys,
    pub core: &'a Core,
    pub ready: &'a Event,
    pub challenge: Id,
    pub source_attestation: &'a Event,
    pub successor_attestation: &'a Event,
    pub prepared: Option<&'a VerifiedPrepared>,
}
pub async fn seal(request: Seal<'_>, root: &RootContext, now: u64) -> Result<Event> {
    let phase = u8::from(request.prepared.is_some());
    seal_phase(request, root, now, phase).await
}
/// CP13b (D2): the source's Aborted over its Prepared (`request.prepared`
/// required): the Committed envelope with inner `phase: 2`.
pub async fn seal_aborted(request: Seal<'_>, root: &RootContext, now: u64) -> Result<Event> {
    if request.prepared.is_none() {
        return Err(Error::Invalid("Aborted without Prepared"));
    }
    seal_phase(request, root, now, 2).await
}
async fn seal_phase(request: Seal<'_>, root: &RootContext, now: u64, phase: u8) -> Result<Event> {
    let r = request;
    r.core.validate(root, now)?;
    if r.source.public_key().to_bytes() != r.core.source_runtime {
        return Err(Error::Invalid("source signer"));
    }
    verify_ready(r.ready, r.core, Some(r.challenge))?;
    attestation(
        r.source_attestation,
        r.core.source_runtime,
        r.core.source_persona,
        r.core,
        now,
    )?;
    attestation(
        r.successor_attestation,
        r.core.successor_runtime,
        r.core.successor_persona,
        r.core,
        now,
    )?;
    if r.prepared.is_some_and(|p| p.core != *r.core) {
        return Err(Error::Invalid("Prepared binding"));
    }
    let env = Envelope {
        v: 1,
        phase,
        core: STANDARD.encode(r.core.encode()),
        ready: r.ready.as_json(),
        source_attestation: r.source_attestation.as_json(),
        successor_attestation: r.successor_attestation.as_json(),
        prepared: r.prepared.map(|p| STANDARD.encode(p.event_hash)),
    };
    let text = serde_json::to_string(&env).map_err(|_| Error::Invalid("envelope"))?;
    if text.len() > super::PLAINTEXT_LIMIT {
        return Err(Error::Limit("plaintext envelope"));
    }
    let ciphertext = nostr::nips::nip44::encrypt(
        r.source.secret_key(),
        &r.recipient.public_key(),
        text,
        nostr::nips::nip44::Version::V2,
    )
    .map_err(|_| Error::Invalid("NIP44 encryption"))?;
    if ciphertext.len() > super::CIPHERTEXT_LIMIT {
        return Err(Error::Limit("ciphertext"));
    }
    let ev = EventBuilder::new(Kind::Custom(super::STORED_KIND), ciphertext)
        .tags([Tag::public_key(r.recipient.public_key())])
        .custom_created_at(Timestamp::from(r.core.issued_at))
        .sign(r.source)
        .await
        .map_err(|_| Error::Invalid("descriptor signature"))?;
    if ev.as_json().len() > super::EVENT_LIMIT {
        return Err(Error::Limit("outer event"));
    }
    Ok(ev)
}

/// Exact outer checks precede decryption. The caller derives recipient from the
/// root/current predecessor and expected source from the verified prefix.
pub fn open(
    ev: &Event,
    keys: &Keys,
    expected_source: Hash,
    root: &RootContext,
    now: u64,
    prepared: Option<&VerifiedPrepared>,
) -> Result<Verified> {
    open_candidates(
        ev,
        keys,
        expected_source,
        root,
        now,
        prepared.map(std::slice::from_ref).unwrap_or(&[]),
    )
}

pub fn open_candidates(
    ev: &Event,
    keys: &Keys,
    expected_source: Hash,
    root: &RootContext,
    now: u64,
    prepared: &[VerifiedPrepared],
) -> Result<Verified> {
    if prepared.len() > 128 {
        return Err(Error::Limit("Prepared candidates"));
    }
    if ev.as_json().len() > super::EVENT_LIMIT || ev.content.len() > super::CIPHERTEXT_LIMIT {
        return Err(Error::Limit("outer event"));
    }
    let tags: Vec<_> = ev.tags.iter().collect();
    if ev.kind != Kind::Custom(super::STORED_KIND)
        || ev.pubkey.to_bytes() != expected_source
        || tags.len() != 1
        || tags[0].as_slice() != ["p", keys.public_key().to_hex().as_str()]
        || ev.created_at.as_secs() > now.saturating_add(120)
        || ev.verify().is_err()
    {
        return Err(Error::Invalid("outer signature/filter"));
    }
    let text = nostr::nips::nip44::decrypt(keys.secret_key(), &ev.pubkey, &ev.content)
        .map_err(|_| Error::Invalid("NIP44 decryption"))?;
    if text.len() > super::PLAINTEXT_LIMIT {
        return Err(Error::Limit("plaintext envelope"));
    }
    let env: Envelope =
        serde_json::from_str(&text).map_err(|_| Error::Invalid("strict envelope"))?;
    if env.v != 1 {
        return Err(Error::Invalid("envelope version"));
    }
    let core = Core::decode(&decode_base64(&env.core, super::certificate::CORE_LIMIT)?)?;
    core.validate(root, now)?;
    if core.source_runtime != expected_source || ev.created_at.as_secs() != core.issued_at {
        return Err(Error::Invalid("outer/core binding"));
    }
    verify_ready(&proof(&env.ready)?, &core, None)?;
    attestation(
        &proof(&env.source_attestation)?,
        core.source_runtime,
        core.source_persona,
        &core,
        now,
    )?;
    attestation(
        &proof(&env.successor_attestation)?,
        core.successor_runtime,
        core.successor_persona,
        &core,
        now,
    )?;
    match (env.phase, env.prepared) {
        (0, None) => Ok(Verified::Prepared(VerifiedPrepared {
            core,
            event_hash: ev.id.to_bytes(),
        })),
        // CP13b (D2): Committed (1) and Aborted (2) share ONE binding check.
        (phase @ (1 | 2), Some(h)) => {
            let h: Hash = decode_base64(&h, 32)?
                .try_into()
                .map_err(|_| Error::Invalid("Prepared hash"))?;
            let p = prepared
                .iter()
                .find(|p| p.event_hash == h)
                .ok_or(Error::Invalid("missing Prepared proof"))?;
            if h != p.event_hash || core != p.core {
                return Err(Error::Invalid("Committed Prepared binding"));
            }
            let event_hash = ev.id.to_bytes();
            Ok(if phase == 1 {
                Verified::Committed(VerifiedCommit { core, event_hash, prepared_hash: h })
            } else {
                Verified::Aborted(VerifiedAbort { core, event_hash, prepared_hash: h })
            })
        }
        _ => Err(Error::Invalid("phase")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handover_hkdf_rfc5869_vector_and_scalar_retry() {
        let prk = nostr::util::hkdf::extract(
            &hex::decode("000102030405060708090a0b0c").unwrap(),
            &[0x0b; 22],
        );
        assert_eq!(
            hex::encode(prk.as_byte_array()),
            "077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5"
        );
        assert_eq!(
            hex::encode(nostr::util::hkdf::expand(
                prk.as_byte_array(),
                &hex::decode("f0f1f2f3f4f5f6f7f8f9").unwrap(),
                42
            )),
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
        );
        let mut attempts = 0;
        let keys = derive_scalar(|n| {
            attempts += 1;
            if n < 2 { vec![0; 32] } else { vec![1; 32] }
        })
        .unwrap();
        assert_eq!(attempts, 3);
        assert_eq!(keys.secret_key().to_secret_hex(), "01".repeat(32));
        assert!(derive_scalar(|_| vec![0; 32]).is_err());
    }
    #[test]
    fn handover_context_recipient_independently_generated_fixture() {
        // Computed independently with Python hashlib/hmac and struct.pack,
        // not by this implementation or by an application self-roundtrip.
        use nostr::ToBech32;
        let key = Keys::new(SecretKey::from_hex(&format!("{:064x}", 1)).unwrap());
        let root = RootContext::new(
            key.public_key().to_bytes(),
            &key.public_key().to_bech32().unwrap(),
            900,
            &["wss://relay.example".into()],
            100,
        )
        .unwrap();
        assert_eq!(
            hex::encode(root.digest()),
            "98488dcfeae12911bb330824b75fe8b850ed15d880562aab7c96e9e9d5bb01e8"
        );
        assert_eq!(
            recipient(&root, &[1; 16], None)
                .unwrap()
                .secret_key()
                .to_secret_hex(),
            "19410d4f34973db776eec1187f77412e1bb0e80339206bac660d41d874cc3176"
        );
        let duplicate = "{\"v\":1,\"v\":1}";
        assert!(serde_json::from_str::<Envelope>(duplicate).is_err());
        assert!(proof(&"x".repeat(super::super::PROOF_LIMIT + 1)).is_err());
    }
    #[test]
    fn handover_nip44_primary_fixed_ciphertext_vector() {
        let keys = Keys::new(SecretKey::from_hex(&format!("{:064x}", 2)).unwrap());
        let other = Keys::new(SecretKey::from_hex(&format!("{:064x}", 1)).unwrap());
        let ciphertext = "AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABee0G5VSK0/9YypIObAtDKfYEAjD35uVkHyB0F4DwrcNaCXlCWZKaArsGrY6M9wnuTMxWfp1RTN9Xga8no+kF5Vsb";
        assert_eq!(
            nostr::nips::nip44::decrypt(keys.secret_key(), &other.public_key(), ciphertext)
                .unwrap(),
            "a"
        );
    }
    #[test]
    fn handover_ciphertext_bound_matches_standard_maximum_plaintext() {
        let a = Keys::generate();
        let b = Keys::generate();
        let cipher = nostr::nips::nip44::encrypt(
            a.secret_key(),
            &b.public_key(),
            vec![b'x'; super::super::PLAINTEXT_LIMIT],
            nostr::nips::nip44::Version::V2,
        )
        .unwrap();
        assert_eq!(cipher.len(), super::super::CIPHERTEXT_LIMIT);
    }
    #[test]
    fn handover_phase_requires_outer_author_and_exact_ready_prepared() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            use nostr::ToBech32;
            let a = Keys::generate();
            let b = Keys::generate();
            let root = RootContext::new(
                a.public_key().to_bytes(),
                &a.public_key().to_bech32().unwrap(),
                900,
                &["wss://relay.example".into()],
                100,
            )
            .unwrap();
            let (_, mut c) = crate::handover::certificate::tests::fixture();
            c.context = root.digest();
            c.root_runtime = root.runtime();
            c.root_persona = root.persona();
            c.source_runtime = root.runtime();
            c.source_persona = root.persona();
            c.successor_runtime = b.public_key().to_bytes();
            c.successor_persona = b.public_key().to_bytes();
            let att_a = crate::runtime_identity::mint_player_attestation(
                &a,
                &a.public_key(),
                Timestamp::from(100),
                1,
            )
            .await
            .unwrap();
            let att_b = crate::runtime_identity::mint_player_attestation(
                &b,
                &b.public_key(),
                Timestamp::from(100),
                1,
            )
            .await
            .unwrap();
            let keys = recipient(&root, &[1; 16], None).unwrap();
            let ready = sign_ready(&b, &c, [8; 16]).await.unwrap();
            let ev = seal(
                Seal {
                    source: &a,
                    recipient: &keys,
                    core: &c,
                    ready: &ready,
                    challenge: [8; 16],
                    source_attestation: &att_a,
                    successor_attestation: &att_b,
                    prepared: None,
                },
                &root,
                100,
            )
            .await
            .unwrap();
            let Verified::Prepared(p) = open(&ev, &keys, root.runtime(), &root, 100, None).unwrap()
            else {
                panic!("Prepared")
            };
            let commit = seal(
                Seal {
                    source: &a,
                    recipient: &keys,
                    core: &c,
                    ready: &ready,
                    challenge: [8; 16],
                    source_attestation: &att_a,
                    successor_attestation: &att_b,
                    prepared: Some(&p),
                },
                &root,
                100,
            )
            .await
            .unwrap();
            assert!(open(&commit, &keys, root.runtime(), &root, 100, None).is_err());
            assert!(matches!(
                open(&commit, &keys, root.runtime(), &root, 100, Some(&p)).unwrap(),
                Verified::Committed(_)
            ));
            let forge =
                EventBuilder::new(Kind::Custom(super::super::STORED_KIND), ev.content.clone())
                    .tags([Tag::public_key(keys.public_key())])
                    .custom_created_at(Timestamp::from(100))
                    .sign(&keys)
                    .await
                    .unwrap();
            assert!(open(&forge, &keys, root.runtime(), &root, 100, None).is_err());
            let wrong = recipient(&root, &[2; 16], None).unwrap();
            assert!(open(&ev, &wrong, root.runtime(), &root, 100, None).is_err());
            assert_ne!(
                recipient(&root, &[1; 16], Some([3; 32]))
                    .unwrap()
                    .public_key(),
                keys.public_key()
            );
        });
    }
}
