//! Fixed codec/cryptographic component controls, not evidence of GameState park.
use super::*;
use crate::handover::{context::RootContext, live::eligibility::Seat};
use nostr::{SecretKey, ToBech32};

fn fixture() -> (Keys, Authority, Fence, Attachment) {
    let mut secret = [0; 32];
    secret[31] = 1;
    let keys = Keys::new(SecretKey::from_slice(&secret).unwrap());
    let key = keys.public_key().to_bytes();
    let root = RootContext::new(
        key,
        &keys.public_key().to_bech32().unwrap(),
        1_792_003_600,
        &["wss://relay.example".into()],
        1_792_000_000,
    )
    .unwrap();
    let authority = Authority::root(&root, [0x22; 16]).unwrap();
    let fence = Fence::issue(
        [0x44; 16],
        [0x55; 16],
        &authority,
        Seat {
            slot: 3,
            generation: 7,
            key,
        },
        key,
    )
    .unwrap();
    let attachment = Attachment::checked(
        [0x44; 16],
        3,
        [0x66; 16],
        GameplayContext::WorldLocal {
            scope: [0x33; 16],
            player: WorldPlayer::Npub(key),
        },
        5,
        5,
    )
    .unwrap();
    (keys, authority, fence, attachment)
}

/// A signed component claim only. Actual-link signing uses from_parked and
/// has separate genuine admission/owner controls; this does not emulate park.
fn signed_claim(
    keys: &Keys,
    authority: &Authority,
    fence: &Fence,
    attachment: Attachment,
) -> Proof {
    let bytes = keys
        .sign_schnorr(&nostr::secp256k1::Message::from_digest(
            fence.message(authority, attachment).unwrap(),
        ))
        .serialize();
    Proof {
        transfer: fence.transfer,
        nonce: fence.nonce,
        fence: fence.digest(authority).unwrap(),
        signature: [
            bytes[..32].try_into().unwrap(),
            bytes[32..].try_into().unwrap(),
        ],
    }
}

#[test]
fn ordered_park_canonical_domains_match_independent_fixed_layout_goldens() {
    let (_, authority, fence, attachment) = fixture();
    // Independently calculated from the documented byte layouts, protocol96,
    // and the secp256k1 generator's public x coordinate (secret scalar1). Protocol100
    // (living world Tier 2 S6): the fence and message domains hash the protocol
    // version, so those goldens were recomputed with 100 (the same independent
    // computation, a short script over the layouts in `park.rs`, reproduced the
    // protocol-99 values before the bump); the authority and attachment digests do not hash it and are
    // unchanged.
    assert_eq!(
        hex::encode(authority_digest(&authority)),
        "30a50bb9c25270bddc7157141c13c3fbd7efc722c1cf66b8b043feee033b62c1"
    );
    assert_eq!(
        hex::encode(fence.digest(&authority).unwrap()),
        "6408e1f69df21176e39d3661fec55a554d6be207aa37eb766aca43e46943f4c8"
    );
    assert_eq!(
        hex::encode(attachment.digest()),
        "e1f19b214aa4ed0b8240c4f0cde1ba2083cea8cb1312a1c45be90f09c6abe80e"
    );
    assert_eq!(
        hex::encode(fence.message(&authority, attachment).unwrap()),
        "34223023a08f7bc51a7b8b5b1ce1aa60e9f68e80722f84121e144ca198572e88"
    );
    let imported = Attachment {
        context: GameplayContext::Imported(crate::personal_value::PersonalContext {
            participant: authority.persona(),
            lineage: [0x77; 16],
            generation: 0x1020_3040_5060_7080,
            transfer: [0x88; 16],
        }),
        ..attachment
    };
    assert_eq!(
        hex::encode(imported.digest()),
        "01108bf32ba2800f216fd37c08b1a9af6e9cfae0a760cb67db5513331b14eb3e"
    );
    assert_eq!(
        hex::encode(fence.message(&authority, imported).unwrap()),
        "7b815dc142e95f8d1db1a8ad5a7f1647c33ff5dd2ca4e5034eb63370e6197cd4"
    );
}

#[test]
fn ordered_park_fixed_codec_pins_append_only_tags_lengths_and_caps() {
    let (keys, authority, fence, attachment) = fixture();
    let proof = signed_claim(&keys, &authority, &fence, attachment);
    for (frame, tag, tagged_length) in [
        (Frame::FreezeFence { fence }, 13u32, 137),
        (Frame::Parked { proof }, 14, 133),
    ] {
        let bytes = frame.encode().unwrap();
        assert_eq!(&bytes[..4], &tag.to_le_bytes());
        assert_eq!(bytes.len() + 1, tagged_length);
        assert_eq!(Frame::decode(&bytes).unwrap(), frame);
        assert_eq!(frame.transfer(), fence.transfer);
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(Frame::decode(&trailing).is_err());
        assert!(Frame::decode(&bytes[..bytes.len() - 1]).is_err());
        let mut oversized = bytes;
        oversized.resize(WIRE_CAP, 0);
        assert!(Frame::decode(&oversized).is_err());
    }
}

#[test]
fn ordered_park_signature_reconstructs_context_life_nonce_authority_and_role() {
    let (keys, authority, fence, attachment) = fixture();
    let proof = signed_claim(&keys, &authority, &fence, attachment);
    proof.verify(&fence, &authority, attachment).unwrap();
    for changed in [
        Attachment {
            transfer: [1; 16],
            ..attachment
        },
        Attachment {
            player_index: 4,
            ..attachment
        },
        Attachment {
            session: [2; 16],
            ..attachment
        },
        Attachment {
            life: 6,
            observed_life: 6,
            ..attachment
        },
        Attachment {
            context: GameplayContext::WorldLocal {
                scope: [4; 16],
                player: WorldPlayer::Npub(authority.persona()),
            },
            ..attachment
        },
        Attachment {
            context: GameplayContext::Imported(crate::personal_value::PersonalContext {
                participant: authority.persona(),
                lineage: [0x77; 16],
                generation: 2,
                transfer: [0x88; 16],
            }),
            ..attachment
        },
    ] {
        assert!(proof.verify(&fence, &authority, changed).is_err());
    }
    for changed in [
        Fence {
            transfer: [3; 16],
            ..fence
        },
        Fence {
            nonce: [4; 16],
            ..fence
        },
        Fence {
            generation: 8,
            ..fence
        },
        Fence {
            authority: [5; 32],
            ..fence
        },
        Fence {
            participant: [6; 32],
            ..fence
        },
        Fence {
            runtime: [7; 32],
            ..fence
        },
    ] {
        assert!(proof.verify(&changed, &authority, attachment).is_err());
    }
    let mut bad = proof;
    bad.signature[0][0] ^= 1;
    assert!(bad.verify(&fence, &authority, attachment).is_err());
    let other = Keys::new(SecretKey::from_slice(&[2; 32]).unwrap());
    assert!(
        signed_claim(&other, &authority, &fence, attachment)
            .verify(&fence, &authority, attachment)
            .is_err()
    );
    // Even a valid signature of the fence digest has the wrong role/domain.
    let sig = keys
        .sign_schnorr(&nostr::secp256k1::Message::from_digest(
            fence.digest(&authority).unwrap(),
        ))
        .serialize();
    bad = proof;
    bad.signature = [sig[..32].try_into().unwrap(), sig[32..].try_into().unwrap()];
    assert!(bad.verify(&fence, &authority, attachment).is_err());
}

#[test]
fn ordered_park_projection_rejects_unsigned_missing_namespace_and_unapplied_life() {
    let (_, _, _, a) = fixture();
    for context in [
        GameplayContext::Guest {
            session: a.session,
            generation: 1,
        },
        GameplayContext::WorldLocal {
            scope: [3; 16],
            player: WorldPlayer::LegacyLocal(0),
        },
    ] {
        assert!(Attachment::checked(a.transfer, a.player_index, a.session, context, 5, 5).is_err());
    }
    assert!(Attachment::checked(a.transfer, a.player_index, [0; 16], a.context, 5, 5).is_err());
    assert!(Attachment::checked(a.transfer, a.player_index, a.session, a.context, 5, 6).is_err());
}

#[test]
fn ordered_park_counts_actual_overlapping_allocations_before_both_encodings() {
    let (_, _, fence, _) = fixture();
    let frame = Frame::FreezeFence { fence };
    let encoded = EncodedControl::new(&frame, 0).unwrap();
    let wire = encoded.tagged(0).unwrap();
    assert!(encoded.0.capacity() <= WIRE_CAP && wire.capacity() <= WIRE_CAP);
    let simultaneous = control_bytes(0, encoded.0.capacity(), wire.capacity());
    assert!(simultaneous <= CONTROL_CAP);
    assert!(EncodedControl::new(&frame, CONTROL_CAP).is_err());
    assert!(encoded.tagged(CONTROL_CAP - simultaneous + 1).is_err());
    assert!(encoded.tagged(CONTROL_CAP - simultaneous).is_ok());
    assert!(encoded.tagged(usize::MAX).is_err());
    assert!(
        EncodedControl::new(
            &Frame::Notice {
                transfer: fence.transfer,
                paused: true
            },
            0
        )
        .is_err()
    );
}

#[test]
fn ordered_park_incoming_census_counts_actual_inner_outer_and_outgoing_overlap() {
    let (_, _, fence, _) = fixture();
    let encoded = EncodedControl::new(&Frame::FreezeFence { fence }, 0).unwrap();
    for count in [16, 32] {
        let mut batch = Vec::new();
        batch.try_reserve_exact(count).unwrap();
        for _ in 0..count {
            let mut wire = encoded.tagged(0).unwrap();
            wire.try_reserve_exact(WIRE_CAP - wire.len()).unwrap();
            assert_eq!(wire.len(), 137);
            assert_eq!(wire.capacity(), WIRE_CAP);
            check_incoming_control(&wire[1..], wire.capacity()).unwrap();
            batch.push(wire);
        }
        let actual_inner = batch.iter().map(Vec::capacity).sum();
        let actual_outer = batch.capacity() * size_of::<Vec<u8>>();
        assert_eq!(
            check_incoming_batch(512, actual_inner, actual_outer).is_ok(),
            count == 16
        );
    }
    let mut short = encoded.tagged(0).unwrap();
    short.try_reserve_exact(WIRE_CAP + 1 - short.len()).unwrap();
    assert!(
        Frame::decode(&short[1..]).is_ok(),
        "valid short encoding alone is insufficient"
    );
    assert!(check_incoming_control(&short[1..], short.capacity()).is_err());
    assert!(check_incoming_batch(usize::MAX, 0, 0).is_err());
}
