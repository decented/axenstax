use super::*;
use crate::handover::{certificate::Core, crypto::Seal, relay_fixture::Relay};
use nostr::{Event, Keys, Timestamp, ToBech32};

struct Actor {
    runtime: Keys,
    persona: Keys,
    attestation: Event,
}
impl Actor {
    async fn new() -> Self {
        let runtime = Keys::generate();
        let persona = Keys::generate();
        let attestation = crate::runtime_identity::mint_player_attestation(
            &persona,
            &runtime.public_key(),
            Timestamp::from(100),
            1,
        )
        .await
        .unwrap();
        Self {
            runtime,
            persona,
            attestation,
        }
    }
}
struct Fixture {
    relay: Relay,
    actors: Vec<Actor>,
    root: RootContext,
}
impl Fixture {
    async fn new() -> Self {
        let relay = Relay::start();
        let actors = vec![Actor::new().await, Actor::new().await, Actor::new().await];
        let root = RootContext::new(
            actors[0].runtime.public_key().to_bytes(),
            &actors[0].persona.public_key().to_bech32().unwrap(),
            900,
            &[relay.url()],
            100,
        )
        .unwrap();
        Self {
            relay,
            actors,
            root,
        }
    }
    async fn commit(
        &self,
        source: usize,
        successor: usize,
        epoch: u32,
        predecessor: [u8; 32],
        transfer: u8,
    ) -> (Event, Event, VerifiedCommit) {
        let a = &self.actors[source];
        let b = &self.actors[successor];
        let core = Core {
            context: self.root.digest(),
            root_runtime: self.root.runtime(),
            root_persona: self.root.persona(),
            root_expiry: 900,
            lineage: root_lineage(&self.root),
            snapshot_world: [2; 16],
            manifest: [3; 32],
            generation: u64::from(epoch),
            epoch,
            predecessor,
            transfer: [transfer; 16],
            source_runtime: a.runtime.public_key().to_bytes(),
            source_persona: a.persona.public_key().to_bytes(),
            successor_runtime: b.runtime.public_key().to_bytes(),
            successor_persona: b.persona.public_key().to_bytes(),
            issued_at: 100,
            not_before: 100,
            expires_at: 800,
            remaining_hops: (4 - epoch) as u8,
        };
        let keys =
            crypto::recipient(&self.root, &[1; 16], (epoch > 1).then_some(predecessor)).unwrap();
        let ready = crypto::sign_ready(&b.runtime, &core, [8; 16])
            .await
            .unwrap();
        let prepare = crypto::seal(
            Seal {
                source: &a.runtime,
                recipient: &keys,
                core: &core,
                ready: &ready,
                challenge: [8; 16],
                source_attestation: &a.attestation,
                successor_attestation: &b.attestation,
                prepared: None,
            },
            &self.root,
            100,
        )
        .await
        .unwrap();
        let Verified::Prepared(vp) =
            crypto::open(&prepare, &keys, core.source_runtime, &self.root, 100, None).unwrap()
        else {
            panic!("Prepared")
        };
        let commit = crypto::seal(
            Seal {
                source: &a.runtime,
                recipient: &keys,
                core: &core,
                ready: &ready,
                challenge: [8; 16],
                source_attestation: &a.attestation,
                successor_attestation: &b.attestation,
                prepared: Some(&vp),
            },
            &self.root,
            100,
        )
        .await
        .unwrap();
        let Verified::Committed(proof) = crypto::open(
            &commit,
            &keys,
            core.source_runtime,
            &self.root,
            100,
            Some(&vp),
        )
        .unwrap() else {
            panic!("Committed")
        };
        (prepare, commit, proof)
    }
    async fn resolve(&self, known: Option<&Prefix>) -> Result<Resolved> {
        lookup(
            self.root.clone(),
            Bearer::new([1; 16]),
            known,
            100,
            &AtomicBool::new(false),
        )
        .await
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn handover_original_invitation_resolves_handback_and_later_epoch_without_cache() {
    runtime().block_on(async {
        let f = Fixture::new().await;
        let invite = crate::invite::Invite {
            host_runtime: f.root.runtime(),
            host_persona: f.actors[0].persona.public_key().to_bech32().unwrap(),
            relays: f.root.relays().to_vec(),
            bearer: [1; 16],
            expires_at: 900,
            world_name: "Original world".into(),
        };
        let link = invite.to_link();
        assert_eq!(crate::invite::Invite::parse(&link, 100).unwrap(), invite);
        assert_eq!(
            f.resolve(None).await.unwrap().authority().unwrap().epoch(),
            0
        );
        let mut previous = [0; 32];
        for (epoch, source, successor) in [(1, 0, 1), (2, 1, 0), (3, 0, 2)] {
            let (prepare, commit, proof) = f
                .commit(source, successor, epoch, previous, epoch as u8 + 10)
                .await;
            f.relay.insert(prepare);
            f.relay.insert(commit);
            previous = proof.event_hash();
            // A cold reconnect starts with the exact ordinary original link.
            let parsed = crate::invite::Invite::parse(&link, 100).unwrap();
            let root = RootContext::new(
                parsed.host_runtime,
                &parsed.host_persona,
                parsed.expires_at,
                &parsed.relays,
                100,
            )
            .unwrap();
            let resolved = lookup(
                root,
                Bearer::new(parsed.bearer),
                None,
                100,
                &AtomicBool::new(false),
            )
            .await
            .unwrap();
            let authority = resolved.authority().unwrap();
            assert_eq!(
                (authority.epoch(), authority.runtime(), authority.persona()),
                (
                    epoch,
                    f.actors[successor].runtime.public_key().to_bytes(),
                    f.actors[successor].persona.public_key().to_bytes()
                )
            );
            assert_eq!(resolved.root, f.root);
            assert_eq!(authority.committed(), proof.event_hash());
        }
    });
}

#[test]
fn handover_original_invitation_prepared_only_cannot_route_and_known_commit_cannot_roll_back() {
    runtime().block_on(async {
        let f = Fixture::new().await;
        let (prepare, commit, _) = f.commit(0, 1, 1, [0; 32], 20).await;
        f.relay.insert(prepare);
        assert_eq!(
            f.resolve(None).await.unwrap().authority().unwrap().epoch(),
            0
        );
        f.relay.insert(commit);
        let known = f.resolve(None).await.unwrap().prefix;
        f.relay.withhold(true);
        assert!(
            f.resolve(Some(&known)).await.is_err(),
            "a withheld known commit downgraded to the root"
        );
        f.relay.withhold(false);
        assert_eq!(
            f.resolve(Some(&known))
                .await
                .unwrap()
                .authority()
                .unwrap()
                .epoch(),
            1
        );
    });
}

#[test]
fn handover_original_invitation_conflicting_commits_never_choose_relay_order() {
    runtime().block_on(async {
        let f = Fixture::new().await;
        for target in [1, 2] {
            let (prepare, commit, _) = f.commit(0, target, 1, [0; 32], target as u8 + 30).await;
            f.relay.insert(prepare);
            f.relay.insert(commit);
        }
        assert!(
            f.resolve(None).await.is_err(),
            "one competing authority was selected"
        );
    });
}

#[test]
fn handover_returned_runtime_refuses_stale_invitation_epoch_before_kith() {
    use crate::rendezvous::{
        payload::{Candidate, Offer, PAYLOAD_VERSION, new_session_id, npub_of, seal_offer},
        relay_client::{FakeRelayHub, RendezvousRelay},
    };
    runtime().block_on(async {
        let f = Fixture::new().await;
        let stale = f.resolve(None).await.unwrap().admission().unwrap();
        let mut previous = [0; 32];
        for (epoch, source, successor) in [(1, 0, 1), (2, 1, 0)] {
            let (prepare, commit, proof) = f
                .commit(source, successor, epoch, previous, epoch as u8 + 40)
                .await;
            f.relay.insert(prepare);
            f.relay.insert(commit);
            previous = proof.event_hash();
        }
        let returned = f.resolve(None).await.unwrap();
        let current = returned.admission().unwrap();
        let hub = FakeRelayHub::new();
        let owner = &f.actors[0];
        let caller = &f.actors[2];
        let book = vec![crate::contacts::Contact {
            pubkey: caller.persona.public_key().to_bytes(),
            display_name: None,
            tier: crate::comms::Tier::Kith,
            is_child: false,
            runtime_pubkey: None,
            added_via: crate::contacts::AddedVia::Kenspeckle,
            added_at: 0,
            last_joined: None,
        }];
        let identity = crate::runtime_identity::RuntimeIdentity::from_parts(
            owner.runtime.clone(),
            Some(owner.attestation.clone()),
        );
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let mut host = crate::online_host::OnlineHost::start(
            identity,
            owner.persona.public_key().to_bytes(),
            Box::new(hub.client()),
            socket,
            Vec::<Candidate>::new(),
            "Returned".into(),
            f.root.relays().to_vec(),
            book,
            100,
        )
        .map_err(|(error, _)| error)
        .unwrap();
        host.install_continuation(returned, 100).unwrap();
        let original = host.invite_link(100).unwrap();
        for time in [101, 120, 200] {
            assert_eq!(
                host.mint_fresh_invite(time).unwrap().to_link(),
                original,
                "the actual repeated hosting action changed the original capability"
            );
        }
        let relay = hub.client();
        for expected in [Some(stale), None, Some(current)] {
            let offer = Offer {
                v: PAYLOAD_VERSION,
                session: new_session_id(),
                persona: npub_of(&caller.persona.public_key()),
                attestation: caller.attestation.clone(),
                bearer: Some(hex::encode([1; 16])),
                protocol: crate::protocol::PROTOCOL_VERSION,
                candidates: Vec::new(),
                sent_at: 100,
                handover: expected,
            };
            relay
                .publish(
                    &seal_offer(&caller.runtime, &owner.runtime.public_key(), &offer)
                        .await
                        .unwrap(),
                )
                .unwrap();
            let events = host.poll(100, 1, 4);
            let admitted = events
                .iter()
                .any(|event| matches!(event, crate::online_host::HostEvent::Admitted { .. }));
            assert_eq!(
                admitted,
                expected == Some(current),
                "wrong admission for {expected:?}: {events:?}"
            );
        }
        host.rotate_bearer();
        assert_eq!(
            host.invite().to_link(),
            original,
            "abort created a hybrid link"
        );
        assert!(host.invite_link(200).is_err());
        assert!(host.mint_fresh_invite(200).is_err());
        assert!(host.live_bearer(200).is_none() && host.root_context(200).is_err());
        let offer = Offer {
            v: PAYLOAD_VERSION,
            session: new_session_id(),
            persona: npub_of(&caller.persona.public_key()),
            attestation: caller.attestation.clone(),
            bearer: Some(hex::encode([1; 16])),
            protocol: crate::protocol::PROTOCOL_VERSION,
            candidates: Vec::new(),
            sent_at: 200,
            handover: Some(current),
        };
        relay
            .publish(
                &seal_offer(&caller.runtime, &owner.runtime.public_key(), &offer)
                    .await
                    .unwrap(),
            )
            .unwrap();
        assert!(
            !host
                .poll(200, 1, 4)
                .iter()
                .any(|event| matches!(event, crate::online_host::HostEvent::Admitted { .. })),
            "a Kith shortcut revived the revoked original invitation"
        );
    });
}
