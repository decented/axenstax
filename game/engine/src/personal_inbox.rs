//! Portable common progression intake and exact physical-window application.
//! No sign-in, file store or native handover authority is invoked by this client.
use crate::{
    progression::GameplayRecord,
    protocol::{PersonalAck, PersonalSnapshot},
};
use sha2::Digest;

pub(crate) fn acknowledgement(snapshot: &PersonalSnapshot) -> PersonalAck {
    PersonalAck {
        session: snapshot.session,
        life: snapshot.life,
        revision: snapshot.revision,
        window_event: snapshot.window_event,
        snapshot: sha2::Sha256::digest(
            bincode::serialize(snapshot).expect("fixed personal snapshot codec"),
        )
        .into(),
    }
}

pub(crate) struct Inbox {
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
    pub fn push(&mut self, value: PersonalSnapshot) -> Result<Option<PersonalAck>, String> {
        if value.session != self.session
            || value.life > self.observed_life
            || value.revision == 0
            || value.window_event == 0
        {
            return Err("foreign or future personal snapshot".into());
        }
        if value.life < self.life {
            return Ok(None);
        }
        GameplayRecord::decode(&value.bytes, value.context)?;
        if matches!(value.context, crate::progression::GameplayContext::Guest { session, .. } if session != self.session)
        {
            return Err("foreign guest progression attachment".into());
        }
        if self.context.is_some_and(|c| c != value.context) {
            return Err("personal snapshot identity changed".into());
        }
        let ack = acknowledgement(&value);
        if let Some(last) = self.last
            && value.revision <= last.revision
        {
            return if ack == last {
                Ok(Some(last))
            } else {
                Ok(None)
            };
        }
        if self.pending.iter().any(|s| acknowledgement(s) == ack) {
            return Ok(None);
        }
        if self.pending.len() >= 8 {
            return Err("personal snapshot queue full".into());
        }
        if self
            .pending
            .back()
            .is_some_and(|s| value.revision <= s.revision)
        {
            return Err("personal snapshot order changed".into());
        }
        let bytes = self
            .owned_bytes()
            .saturating_add(value.bytes.capacity())
            .saturating_add(std::mem::size_of::<PersonalSnapshot>());
        if bytes > crate::action_journal::MAX_RETAINED_BYTES {
            return Err("personal snapshots exceed their retained byte limit".into());
        }
        self.pending
            .try_reserve_exact(1)
            .map_err(|_| "personal snapshot allocation failed")?;
        if self.owned_bytes().saturating_add(value.bytes.capacity())
            > crate::action_journal::MAX_RETAINED_BYTES
        {
            return Err("personal snapshot allocation exceeds its byte limit".into());
        }
        self.context = Some(value.context);
        self.pending.push_back(value);
        Ok(None)
    }
    pub fn apply(
        &mut self,
        events: u32,
        slot: &mut crate::player_slot::PlayerSlot,
    ) -> Result<Vec<PersonalAck>, String> {
        let mut applied = Vec::new();
        while let Some(snapshot) = self.pending.front() {
            if snapshot.life < self.life {
                self.pending.pop_front();
                continue;
            }
            if snapshot.life != self.life || snapshot.window_event > events {
                break;
            }
            let value = self.pending.pop_front().unwrap();
            let record = GameplayRecord::decode(&value.bytes, value.context)?;
            record.install_progression(slot);
            let ack = acknowledgement(&value);
            self.last = Some(ack);
            applied.push(ack);
        }
        Ok(applied)
    }
}

impl crate::GameState {
    pub(crate) fn apply_personal_inbox(&mut self) {
        // Socket receipt cannot acknowledge progression through a held pack
        // window. Its own numbered fence must first reach actual application.
        if self.remote_client.as_ref().is_some_and(|client| {
            client.pack_window_inbox_held() || client.initial_personal_window_waits()
        }) {
            return;
        }
        let Some(client) = self.remote_client.as_mut() else {
            return;
        };
        let events = client.applied_window_event();
        let Some(inbox) = &mut client.personal_inbox else {
            return;
        };
        let Some(slot) = self.players.first_mut() else {
            return;
        };
        match inbox.apply(events, slot) {
            Ok(acks) => {
                for ack in acks {
                    client.send_raw(&crate::protocol::serialize_packet(
                        crate::protocol::PacketType::PersonalSync,
                        &crate::protocol::PersonalSyncPacket::Applied(ack),
                    ));
                }
                // The exact PersonalACK leaves before a quest receipt can
                // release its flight, overflow feedback or later authority.
                client.retire_applied_receipts();
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
