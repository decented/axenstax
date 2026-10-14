//! Bounded ordered direct-transfer stream, with no archive-sized queue.
use super::{Error, Hash, Id, Result};
use sha2::{Digest, Sha256};
use std::{collections::VecDeque, io::Write};
pub const CHUNK_LIMIT: usize = 16 * 1024;
pub const FLIGHT_LIMIT: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub transfer: Id,
    pub offset: u64,
    pub bytes: Vec<u8>,
}
impl Chunk {
    pub fn validate(&self) -> Result<()> {
        if self.bytes.is_empty()
            || self.bytes.len() > CHUNK_LIMIT
            || self.bytes.capacity() > CHUNK_LIMIT
        {
            return Err(Error::Limit("transfer chunk"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
struct Receipt {
    offset: u64,
    length: usize,
    hash: Hash,
}
/// Caller supplies monotonically increasing elapsed seconds, independent of
/// wall-clock certificate expiry. Invalid packets never refresh progress.
pub struct Receiver<W: Write> {
    writer: W,
    transfer: Id,
    declared: u64,
    expected: Hash,
    offset: u64,
    hasher: Sha256,
    recent: VecDeque<Receipt>,
    started: u64,
    progress: u64,
    failed: bool,
    finished: bool,
}
impl<W: Write> Receiver<W> {
    pub fn new(writer: W, transfer: Id, declared: u64, expected: Hash, now: u64) -> Result<Self> {
        if transfer == [0; 16]
            || declared == 0
            || declared > super::manifest::COMPRESSED_LIMIT
            || expected == [0; 32]
        {
            return Err(Error::Invalid("stream declaration"));
        }
        Ok(Self {
            writer,
            transfer,
            declared,
            expected,
            offset: 0,
            hasher: Sha256::new(),
            recent: VecDeque::new(),
            started: now,
            progress: now,
            failed: false,
            finished: false,
        })
    }
    pub fn offset(&self) -> u64 {
        self.offset
    }
    pub fn check_deadline(&self, now: u64) -> Result<()> {
        if now < self.started
            || now < self.progress
            || now - self.started >= 300
            || now - self.progress >= 15
        {
            return Err(Error::Deadline);
        }
        Ok(())
    }
    pub fn accept(&mut self, chunk: &Chunk, now: u64) -> Result<bool> {
        if self.failed || self.finished {
            return Err(Error::State("closed transfer"));
        }
        self.check_deadline(now)?;
        chunk.validate()?;
        if chunk.transfer != self.transfer {
            return Err(Error::Invalid("transfer id"));
        }
        let hash = super::hash(&chunk.bytes);
        if chunk.offset < self.offset {
            if self.recent.iter().any(|r| {
                r.offset == chunk.offset && r.length == chunk.bytes.len() && r.hash == hash
            }) {
                return Ok(false);
            }
            return Err(Error::Invalid("conflicting/expired duplicate chunk"));
        }
        if chunk.offset != self.offset {
            return Err(Error::Invalid("chunk order"));
        }
        let end = self
            .offset
            .checked_add(chunk.bytes.len() as u64)
            .ok_or(Error::Limit("stream offset"))?;
        if end > self.declared {
            return Err(Error::Limit("stream total"));
        }
        // Partial disk writes poison this receiver. Never acknowledge or append
        // again to an uncertain temp file; cleanup/restart owns the file.
        if self.writer.write_all(&chunk.bytes).is_err() {
            self.failed = true;
            return Err(Error::Io);
        }
        self.hasher.update(&chunk.bytes);
        self.offset = end;
        self.progress = now;
        self.recent.push_back(Receipt {
            offset: chunk.offset,
            length: chunk.bytes.len(),
            hash,
        });
        if self.recent.len() > FLIGHT_LIMIT {
            self.recent.pop_front();
        }
        Ok(true)
    }
    pub fn finish(mut self, now: u64) -> Result<W> {
        self.check_deadline(now)?;
        if self.failed || self.finished || self.offset != self.declared {
            return Err(Error::Invalid("incomplete stream"));
        }
        let digest: Hash = self.hasher.finalize().into();
        if digest != self.expected {
            return Err(Error::Invalid("stream hash"));
        }
        self.writer.flush().map_err(|_| Error::Io)?;
        self.finished = true;
        Ok(self.writer)
    }
}

/// Only four chunks can be owned/enqueued before acknowledgement. No allowance
/// for an attacker/caller's oversized Vec capacity hidden behind a short slice.
pub struct Sender {
    transfer: Id,
    declared: u64,
    next: u64,
    acked: u64,
    inflight: VecDeque<Chunk>,
}
impl Sender {
    pub fn new(transfer: Id, declared: u64) -> Result<Self> {
        if transfer == [0; 16] || declared == 0 || declared > super::manifest::COMPRESSED_LIMIT {
            return Err(Error::Invalid("sender declaration"));
        }
        Ok(Self {
            transfer,
            declared,
            next: 0,
            acked: 0,
            inflight: VecDeque::new(),
        })
    }
    pub fn enqueue(&mut self, bytes: Vec<u8>) -> Result<&Chunk> {
        if self.inflight.len() >= FLIGHT_LIMIT {
            return Err(Error::Limit("in-flight chunks"));
        }
        let c = Chunk {
            transfer: self.transfer,
            offset: self.next,
            bytes,
        };
        c.validate()?;
        let next = self
            .next
            .checked_add(c.bytes.len() as u64)
            .ok_or(Error::Limit("stream offset"))?;
        if next > self.declared {
            return Err(Error::Limit("stream total"));
        }
        self.inflight.push_back(c);
        self.next = next;
        self.inflight
            .back()
            .ok_or(Error::State("missing enqueued chunk"))
    }
    pub fn acknowledge(&mut self, transfer: Id, through: u64) -> Result<bool> {
        if transfer != self.transfer || through > self.next {
            return Err(Error::Invalid("chunk acknowledgement"));
        }
        if through <= self.acked {
            return Ok(false);
        }
        if !self
            .inflight
            .iter()
            .any(|c| c.offset + c.bytes.len() as u64 == through)
        {
            return Err(Error::Invalid("partial acknowledgement"));
        }
        while self
            .inflight
            .front()
            .is_some_and(|c| c.offset + c.bytes.len() as u64 <= through)
        {
            self.inflight.pop_front();
        }
        self.acked = through;
        Ok(true)
    }
    pub fn complete(&self) -> bool {
        self.acked == self.declared
    }
    pub fn allocated_chunk_bytes(&self) -> usize {
        self.inflight.iter().map(|c| c.bytes.capacity()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handover_stream_hash_order_duplicates_and_deadlines() {
        let data = b"actual checkpoint bytes";
        let mut r = Receiver::new(
            Vec::new(),
            [1; 16],
            data.len() as u64,
            super::super::hash(data),
            0,
        )
        .unwrap();
        let c = Chunk {
            transfer: [1; 16],
            offset: 0,
            bytes: data.to_vec(),
        };
        assert!(r.accept(&c, 1).unwrap());
        assert!(!r.accept(&c, 2).unwrap());
        let mut bad = c.clone();
        bad.bytes[0] ^= 1;
        assert!(r.accept(&bad, 3).is_err());
        assert_eq!(r.finish(3).unwrap(), data);
        let mut r = Receiver::new(Vec::new(), [1; 16], data.len() as u64, [8; 32], 0).unwrap();
        assert!(r.accept(&c, 1).unwrap());
        assert!(r.finish(2).is_err());
        let mut r = Receiver::new(Vec::new(), [1; 16], data.len() as u64, [8; 32], 0).unwrap();
        assert!(r.accept(&c, 15).is_err());
        assert_eq!(r.offset(), 0);
    }
    #[test]
    fn handover_stream_bounds_prevent_enqueue_and_partial_ack() {
        let mut s = Sender::new([1; 16], (CHUNK_LIMIT * 5) as u64).unwrap();
        for _ in 0..4 {
            s.enqueue(vec![1; CHUNK_LIMIT]).unwrap();
        }
        assert_eq!(s.allocated_chunk_bytes(), 64 * 1024);
        assert!(s.enqueue(vec![1; 1]).is_err());
        assert!(s.acknowledge([1; 16], 1).is_err());
        s.acknowledge([1; 16], CHUNK_LIMIT as u64).unwrap();
        s.enqueue(vec![1; CHUNK_LIMIT]).unwrap();
        assert!(s.acknowledge([2; 16], (CHUNK_LIMIT * 5) as u64).is_err());
        s.acknowledge([1; 16], (CHUNK_LIMIT * 5) as u64).unwrap();
        assert!(s.complete());
        let mut v = Vec::with_capacity(CHUNK_LIMIT + 1);
        v.push(1);
        assert!(Sender::new([1; 16], 1).unwrap().enqueue(v).is_err());
    }
    #[test]
    fn handover_stream_write_failure_is_terminal() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("fixture"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut r = Receiver::new(Broken, [1; 16], 1, [1; 32], 0).unwrap();
        let c = Chunk {
            transfer: [1; 16],
            offset: 0,
            bytes: vec![1],
        };
        assert_eq!(r.accept(&c, 1), Err(Error::Io));
        assert_eq!(r.offset(), 0);
        assert!(matches!(r.accept(&c, 2), Err(Error::State(_))));
    }
}
