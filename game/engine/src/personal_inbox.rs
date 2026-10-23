//! Portable common progression intake and exact physical-window application.
//! No sign-in, file store or native handover authority is invoked by this client.
use crate::protocol::{PersonalAck, PersonalSnapshot};
use sha2::Digest;
mod prepared;
pub(crate) use prepared::{AppliedPersonalBatch, PersonalAdmission};
#[cfg(any(test, not(target_arch = "wasm32")))]
pub(crate) use prepared::{PersonalFenceIdentity, PreparedInboxApply, PreparedPersonalPush};
#[cfg(test)]
mod prepared_tests;

pub(crate) fn acknowledgement(snapshot: &PersonalSnapshot) -> PersonalAck {
    acknowledgement_parts(
        snapshot.session,
        snapshot.life,
        snapshot.revision,
        snapshot.window_event,
        snapshot.context,
        &snapshot.bytes,
    )
}

pub(crate) fn acknowledgement_parts(
    session: [u8; 16],
    life: u32,
    revision: u64,
    window_event: u32,
    context: crate::progression::GameplayContext,
    bytes: &[u8],
) -> PersonalAck {
    #[derive(serde::Serialize)]
    struct BorrowedSnapshot<'a> {
        session: [u8; 16],
        life: u32,
        revision: u64,
        window_event: u32,
        context: crate::progression::GameplayContext,
        bytes: &'a [u8],
    }
    let snapshot = BorrowedSnapshot {
        session,
        life,
        revision,
        window_event,
        context,
        bytes,
    };
    // Stream the identical fixed bincode layout into SHA-256 without retaining
    // another snapshot buffer during bounded admission/application.
    struct HashWriter(sha2::Sha256);
    impl std::io::Write for HashWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = HashWriter(sha2::Sha256::new());
    bincode::serialize_into(&mut writer, &snapshot).expect("fixed personal snapshot codec");
    PersonalAck {
        session: snapshot.session,
        life: snapshot.life,
        revision: snapshot.revision,
        window_event: snapshot.window_event,
        snapshot: writer.0.finalize().into(),
    }
}

pub(crate) struct Inbox {
    #[cfg(any(test, not(target_arch = "wasm32")))]
    origin: std::sync::Arc<()>,
    session: [u8; 16],
    pub life: u32,
    observed_life: u32,
    pending: std::collections::VecDeque<PersonalSnapshot>,
    last: Option<PersonalAck>,
    context: Option<crate::progression::GameplayContext>,
    started: web_time::Instant,
}
impl Inbox {
    pub fn new(session: [u8; 16], life: u32) -> Self {
        Self {
            #[cfg(any(test, not(target_arch = "wasm32")))]
            origin: std::sync::Arc::new(()),
            session,
            life,
            observed_life: life,
            pending: Default::default(),
            last: None,
            context: None,
            started: web_time::Instant::now(),
        }
    }
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
    pub fn owned_bytes(&self) -> usize {
        self.pending
            .capacity()
            .saturating_mul(std::mem::size_of::<PersonalSnapshot>())
            .saturating_add(
                self.pending
                    .iter()
                    .map(|snapshot| snapshot.bytes.capacity())
                    .fold(0usize, usize::saturating_add),
            )
    }
    pub fn has_pending_fence(&self, event: u32, life: u32) -> bool {
        self.pending
            .iter()
            .any(|snapshot| snapshot.window_event == event && snapshot.life == life)
    }
    pub(crate) fn applied_through(&self, event: u32, life: u32) -> bool {
        self.last
            .is_some_and(|ack| ack.life == life && ack.window_event >= event)
    }
    pub fn observe_respawn(&mut self, respawns: u32) -> Result<(), String> {
        if respawns <= self.observed_life {
            return Ok(());
        }
        if self.observed_life.checked_add(1) != Some(respawns) {
            return Err("personal life skipped".into());
        }
        self.observed_life = respawns;
        Ok(())
    }
    pub fn timed_out(&self) -> bool {
        !self.initialised() && self.started.elapsed() >= std::time::Duration::from_secs(30)
    }
    pub fn initialised(&self) -> bool {
        self.last.is_some()
    }
    #[cfg(test)]
    pub fn push(&mut self, value: PersonalSnapshot) -> Result<Option<PersonalAck>, String> {
        Ok(match self.prepare_push(value)?.commit() {
            PersonalAdmission::DuplicateApplied(batch) => batch.last_ack(),
            PersonalAdmission::Admitted(_) | PersonalAdmission::Ignored => None,
        })
    }
    #[cfg(test)]
    pub fn apply(
        &mut self,
        events: u32,
        slot: &mut crate::player_slot::PlayerSlot,
    ) -> Result<Vec<PersonalAck>, String> {
        if self.pending.is_empty() {
            return Ok(Vec::new());
        }
        // The public return vector and live map overlap are charged before installation.
        let mut acks = Vec::new();
        acks.try_reserve_exact(self.pending.len())
            .map_err(|_| "Personal ACK allocation failed")?;
        let external = acks.capacity() * std::mem::size_of::<PersonalAck>();
        let prepared = self.prepare_apply(events, slot, (0, external))?;
        let batch = prepared.commit();
        acks.extend(batch.acks());
        Ok(acks)
    }
}

impl crate::GameState {
    pub(crate) fn apply_personal_inbox(&mut self) {
        // Socket receipt cannot acknowledge progression through a held pack
        // window. Its own numbered fence must first reach actual application.
        if self.remote_client.as_ref().is_some_and(|client| {
            client.is_parked()
                || client.pack_window_inbox_held()
                || client.initial_personal_window_waits()
        }) {
            return;
        }
        let Some(client) = self.remote_client.as_mut() else {
            return;
        };
        let events = client.applied_window_event();
        let retained = client.personal_control_retained_usage();
        let Some(inbox) = &mut client.personal_inbox else {
            return;
        };
        let Some(slot) = self.players.first_mut() else {
            return;
        };
        if inbox.pending_count() == 0 {
            client.retire_applied_receipts();
            return;
        }
        let window = self.window_inbox.retained_usage();
        let external = (
            retained
                .0
                .saturating_sub(inbox.pending_count())
                .saturating_add(window.0),
            retained
                .1
                .saturating_sub(inbox.owned_bytes())
                .saturating_add(window.1)
                .saturating_add(self.chunk_intake.carried_metadata_bytes()),
        );
        let result = inbox
            .prepare_apply(events, slot, external)
            .map(|prepared| prepared.commit());
        match result {
            Ok(batch) => {
                for wire in batch.wires() {
                    client.send_raw(wire);
                }
                // The actual prebuilt PersonalACK leaves before a quest
                // receipt releases its flight, feedback or later authority.
                client.retire_applied_receipts();
                // The personal state becomes ready only here: a Mirror
                // window's settings are logged now, with every op logged
                // before dropped, so nothing made before ready goes out and
                // they precede any later click. A death fence still pending
                // on the server drops them, as it drops any Mirror op while
                // dead.
                slot.crafting_ui.settle_initial_settings(client, &slot.inventory, &slot.armour_slots);
            }
            Err(error) => client.state = crate::remote_client::ConnectionState::Failed(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn d2c3_portable_personal_ack_preserves_exact_snapshot_bytes_and_sha256() {
        // Pin the established wire layout independently of the native proof
        // adapter. Opaque body bytes suffice for this hash/codec golden;
        // Inbox admission separately requires a fully valid progression body.
        let snapshot = PersonalSnapshot {
            session: [1; 16],
            life: 0,
            revision: 1,
            window_event: 2,
            context: crate::progression::GameplayContext::Guest {
                session: [1; 16],
                generation: 1,
            },
            bytes: vec![9, 8],
        };
        let bytes = bincode::serialize(&snapshot).unwrap();
        assert_eq!(bytes.len(), 66);
        assert_eq!(&bytes[..16], &[1; 16]);
        assert_eq!(
            &bytes[16..32],
            &[0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0]
        );
        let ack = acknowledgement(&snapshot);
        assert_eq!(
            hex::encode(ack.snapshot),
            "10ce8ed7525b4c02341d0dc5af686193aa4b169a237b4d23e610916962947ca8"
        );
        #[cfg(not(target_arch = "wasm32"))]
        assert_eq!(
            ack.snapshot,
            crate::handover::hash(&bytes),
            "portable and established native acknowledgements use identical bytes"
        );
    }
}
