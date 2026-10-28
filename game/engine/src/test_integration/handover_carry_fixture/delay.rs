//! Bounded network delay below the real recipient, including its parked link.
use crate::{
    protocol::{self, PacketType},
    transport::{ClientTransport, Packet},
};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

const PACKETS: usize = 64;
const OWNED_BYTES: usize = 1024 * 1024;
const EVENTS: usize = 256;
const METADATA_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Sent,
    Forwarded,
    Held,
    Released,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Wire {
    tag: Option<PacketType>,
    length: usize,
    capacity: Option<usize>,
    allocation: Option<usize>,
    hash: [u8; 32],
}

impl Wire {
    fn of(packet: &[u8], capacity: Option<usize>) -> Self {
        assert!(
            packet.len() <= protocol::MAX_WIRE_PACKET_LEN,
            "delay wire work bound exhausted"
        );
        Self {
            tag: protocol::deserialize_header(packet).map(|(tag, _)| tag),
            length: packet.len(),
            capacity,
            allocation: capacity.map(|_| packet.as_ptr() as usize),
            hash: Sha256::digest(packet).into(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Event {
    ordinal: usize,
    kind: Kind,
    wire: Wire,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Forward,
    Hold,
    Release,
}

struct State {
    phase: Phase,
    armed: bool,
    delayed: bool,
    held: VecDeque<Packet>,
    packet_capacity: usize,
    events: Vec<Event>,
}

impl State {
    fn record(&mut self, kind: Kind, wire: Wire) {
        if !self.armed {
            return;
        }
        assert!(
            self.events.len() < EVENTS && self.events.len() < self.events.capacity(),
            "delay metadata exhausted"
        );
        self.events.push(Event {
            ordinal: self.events.len(),
            kind,
            wire,
        });
    }

    fn hold(&mut self, packet: Packet) {
        let capacity = packet.capacity();
        let outer = self
            .held
            .capacity()
            .checked_mul(std::mem::size_of::<Packet>())
            .unwrap();
        let next = self.packet_capacity.checked_add(capacity).unwrap();
        assert!(
            self.held.len() < PACKETS && self.held.len() < self.held.capacity(),
            "delay packet bound exhausted"
        );
        assert!(
            next.checked_add(outer).unwrap() <= OWNED_BYTES,
            "delay owned-capacity bound exhausted"
        );
        self.record(Kind::Held, Wire::of(&packet, Some(capacity)));
        self.packet_capacity = next;
        self.held.push_back(packet);
    }
}

#[derive(Clone)]
pub(in crate::test_integration) struct DelayControl(Arc<Mutex<State>>);

struct Delayed<T> {
    inner: T,
    control: DelayControl,
}

pub(in crate::test_integration) fn wrap<T: ClientTransport>(
    inner: T,
) -> (impl ClientTransport, DelayControl) {
    let mut held = VecDeque::new();
    held.try_reserve_exact(PACKETS)
        .expect("delay queue allocation");
    assert!(held.capacity() <= PACKETS, "delay queue over-allocation");
    let mut events = Vec::new();
    events
        .try_reserve_exact(EVENTS)
        .expect("delay metadata allocation");
    assert!(
        events
            .capacity()
            .checked_mul(std::mem::size_of::<Event>())
            .unwrap()
            <= METADATA_BYTES
    );
    let control = DelayControl(Arc::new(Mutex::new(State {
        phase: Phase::Forward,
        armed: false,
        delayed: false,
        held,
        packet_capacity: 0,
        events,
    })));
    (
        Delayed {
            inner,
            control: control.clone(),
        },
        control,
    )
}

impl DelayControl {
    pub(in crate::test_integration) fn arm(&self, delayed: bool) {
        let mut state = self.0.lock().expect("delay state poisoned");
        assert!(!state.armed && state.events.is_empty() && state.held.is_empty());
        state.delayed = delayed;
        state.armed = true;
        state.phase = if delayed { Phase::Hold } else { Phase::Forward };
    }

    pub(in crate::test_integration) fn assert_prefix(&self) {
        let state = self.0.lock().expect("delay state poisoned");
        let kind = if state.delayed {
            Kind::Held
        } else {
            Kind::Forwarded
        };
        assert!(
            state
                .events
                .iter()
                .any(|event| event.kind == kind && event.wire.tag == Some(PacketType::StateUpdate)),
            "a complete source tick must provide a real ordinary prefix"
        );
        assert_eq!(state.held.is_empty(), !state.delayed);
        let mut ordinary = state
            .events
            .iter()
            .filter(|event| event.kind == kind && event.wire.tag != Some(PacketType::Handover));
        assert_eq!(
            ordinary.clone().next().unwrap().wire.tag,
            Some(PacketType::StateUpdate),
            "the genuine still-raw prefix must start with ordinary state"
        );
        assert!(
            ordinary.all(|event| event.wire.tag != Some(PacketType::PersonalSync)),
            "this control cannot demand already-admitted Personal bodies in raw abort replay"
        );
    }

    pub(in crate::test_integration) fn release(&self) {
        let mut state = self.0.lock().expect("delay state poisoned");
        assert!(state.armed);
        if state.delayed {
            assert_eq!(state.phase, Phase::Hold);
            assert!(!state.held.is_empty());
            state.phase = Phase::Release;
        }
    }

    pub(in crate::test_integration) fn assert_released(&self) {
        let mut state = self.0.lock().expect("delay state poisoned");
        assert!(state.armed);
        eprintln!(
            "actual carry delay: delayed={} held={} owned_packet_capacity={} events={:?}",
            state.delayed,
            state.held.len(),
            state.packet_capacity,
            state.events
        );
        assert!(
            state.held.is_empty(),
            "the real peer pump did not receive the entire delayed prefix"
        );
        assert_eq!(state.packet_capacity, 0);
        let held = state.events.iter().filter(|event| event.kind == Kind::Held);
        let released = state
            .events
            .iter()
            .filter(|event| event.kind == Kind::Released);
        assert_eq!(held.clone().count(), released.clone().count());
        for (held, released) in held.zip(released) {
            assert_eq!(
                held.wire, released.wire,
                "actual owned packets must retain capacity, bytes and FIFO order"
            );
            assert!(released.ordinal > held.ordinal);
        }
        if state.delayed {
            let last_release = state
                .events
                .iter()
                .rfind(|event| event.kind == Kind::Released)
                .unwrap()
                .ordinal;
            assert!(
                state
                    .events
                    .iter()
                    .filter(|event| event.kind == Kind::Forwarded)
                    .all(|event| event.ordinal > last_release),
                "later host output overtook an original held packet"
            );
        }
        assert!(
            state
                .events
                .iter()
                .any(|event| event.kind == Kind::Sent
                    && event.wire.tag == Some(PacketType::Handover)),
            "the real consent control must travel on the opposite link"
        );
        state.armed = false;
    }


}

impl<T: ClientTransport> ClientTransport for Delayed<T> {
    fn send_to_server(&self, bytes: &[u8]) {
        // No delay or substitution on the opposite direction, including Accept.
        let mut state = self.control.0.lock().expect("delay state poisoned");
        // Validate the armed observation's work before delegating this send.
        let wire = state.armed.then(|| Wire::of(bytes, None));
        self.inner.send_to_server(bytes);
        if let Some(wire) = wire {
            state.record(Kind::Sent, wire);
        }
    }

    fn try_recv_from_server(&self) -> Option<Packet> {
        let mut state = self.control.0.lock().expect("delay state poisoned");
        if state.phase == Phase::Hold {
            // A deliberate network delay can appear empty to its recipient.
            // Capacity/work exhaustion instead fails loudly, never as empty.
            for _ in 0..PACKETS {
                let packet = self.inner.try_recv_from_server()?;
                state.hold(packet);
            }
            panic!(
                "delay receive-work bound exhausted before observing the underlying queue empty"
            );
        }
        if state.phase == Phase::Release {
            if let Some(packet) = state.held.pop_front() {
                state.packet_capacity = state
                    .packet_capacity
                    .checked_sub(packet.capacity())
                    .unwrap();
                state.record(Kind::Released, Wire::of(&packet, Some(packet.capacity())));
                return Some(packet);
            }
            state.phase = Phase::Forward;
        }
        let packet = self.inner.try_recv_from_server()?;
        if state.armed {
            state.record(Kind::Forwarded, Wire::of(&packet, Some(packet.capacity())));
        }
        Some(packet)
    }

    fn channel_binding(&self) -> Option<[u8; 32]> {
        self.inner.channel_binding()
    }

    fn ws_host(&self) -> Option<String> {
        self.inner.ws_host()
    }

    fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    fn close_text(&self) -> Option<String> {
        self.inner.close_text()
    }

    fn host_closed_link(&self) -> bool {
        self.inner.host_closed_link()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn proven_host(&self) -> Option<crate::transport::ProvenHost> {
        self.inner.proven_host()
    }
}
