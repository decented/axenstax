//! Bounded first-file syncing for import extraction. Every queued handle is
//! flushed, and all workers join before directory syncs or scratch cleanup.
use super::super::{Error, Result};
use crate::fsync::Written;
use std::sync::{Arc, Mutex, mpsc};

const WORKERS: usize = 4;
const QUEUED: usize = 8;

pub(super) struct SyncPool {
    sender: Option<mpsc::SyncSender<Written>>,
    workers: Vec<std::thread::JoinHandle<std::io::Result<()>>>,
}
impl SyncPool {
    pub(super) fn new() -> Result<Self> {
        Self::start(crate::fsync::sync_written)
    }
    pub(super) fn start(
        sync: impl Fn(&Written) -> std::io::Result<()> + Send + Sync + 'static,
    ) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(QUEUED);
        let receiver = Arc::new(Mutex::new(receiver));
        let sync = Arc::new(sync);
        let mut pool = Self { sender: Some(sender), workers: Vec::with_capacity(WORKERS) };
        for _ in 0..WORKERS {
            let receiver = receiver.clone();
            let sync = sync.clone();
            let worker = std::thread::Builder::new().name("handover-fsync".into()).spawn(move || {
                let mut failure = None;
                loop {
                    // Hold the queue lock only whilst receiving, never during I/O.
                    let file = receiver.lock().map_err(|_| std::io::Error::other("sync queue poisoned"))?.recv();
                    let Ok(file) = file else { break };
                    // Drain after an error too: no later cleanup races a worker.
                    if let Err(error) = sync(&file) {
                        failure.get_or_insert(error);
                    }
                }
                failure.map_or(Ok(()), Err)
            }).map_err(|_| Error::Io)?;
            pool.workers.push(worker);
        }
        Ok(pool)
    }
    pub(super) fn submit(&mut self, file: Written) -> Result<()> {
        self.sender.as_ref().ok_or(Error::Io)?.send(file).map_err(|_| Error::Io)
    }
    fn join(&mut self) -> Result<()> {
        self.sender.take();
        let mut failed = false;
        for worker in self.workers.drain(..) {
            if !matches!(worker.join(), Ok(Ok(()))) {
                failed = true;
            }
        }
        if failed { Err(Error::Io) } else { Ok(()) }
    }
    pub(super) fn finish(mut self) -> Result<()> {
        self.join()
    }
}
impl Drop for SyncPool {
    fn drop(&mut self) {
        // Also applies to failed extraction, failed thread creation and unwind.
        let _ = self.join();
    }
}
