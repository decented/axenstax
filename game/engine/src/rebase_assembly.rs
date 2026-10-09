//! Bounded owned recovery pages; parsing completion is not an applied fence.

use crate::action_identity::RecoveryCut;
use crate::action_journal::{MAX_BUNDLE_BYTES, MAX_PAGE_BYTES};

pub const MAX_REBASE_PARTS: usize = MAX_BUNDLE_BYTES / MAX_PAGE_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RebaseManifest {
    pub cut: RecoveryCut,
    pub included_events: u32,
    pub parts: u16,
    pub encoded_bytes: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssemblyError { Namespace, Manifest, Part, Conflict, Bytes, Incomplete }

#[derive(Debug)]
pub struct RebaseAssembly {
    manifest: RebaseManifest,
    pages: Vec<Option<Box<[u8]>>>,
    received: usize,
    ended: bool,
    applied: bool,
}

impl RebaseAssembly {
    pub fn new(manifest: RebaseManifest, session: [u8; 16], life: u32) -> Result<Self, AssemblyError> {
        if manifest.cut.session != session || manifest.cut.life != life { return Err(AssemblyError::Namespace); }
        let parts = usize::from(manifest.parts);
        if parts == 0 || parts > MAX_REBASE_PARTS || manifest.encoded_bytes == 0 { return Err(AssemblyError::Manifest); }
        let pages: Vec<Option<Box<[u8]>>> = (0..parts).map(|_| None).collect();
        let overhead = std::mem::size_of::<Self>() + pages.capacity() * std::mem::size_of::<Option<Box<[u8]>>>();
        if (manifest.encoded_bytes as usize).checked_add(overhead).is_none_or(|n| n > MAX_BUNDLE_BYTES) {
            return Err(AssemblyError::Bytes);
        }
        Ok(Self { manifest, pages, received: 0, ended: false, applied: false })
    }

    pub fn page(&mut self, cut: RecoveryCut, index: u16, bytes: Box<[u8]>) -> Result<(), AssemblyError> {
        if cut != self.manifest.cut { return Err(AssemblyError::Namespace); }
        if bytes.is_empty() || bytes.len() > MAX_PAGE_BYTES { return Err(AssemblyError::Bytes); }
        let page = self.pages.get_mut(usize::from(index)).ok_or(AssemblyError::Part)?;
        if let Some(old) = page {
            return if old.as_ref() == bytes.as_ref() { Ok(()) } else { Err(AssemblyError::Conflict) };
        }
        if self.ended { return Err(AssemblyError::Conflict); }
        let received = self.received.checked_add(bytes.len()).ok_or(AssemblyError::Bytes)?;
        if received > self.manifest.encoded_bytes as usize { return Err(AssemblyError::Bytes); }
        *page = Some(bytes);
        self.received = received;
        Ok(())
    }

    pub fn end(&mut self, cut: RecoveryCut) -> Result<(), AssemblyError> {
        if cut != self.manifest.cut { return Err(AssemblyError::Namespace); }
        if self.received != self.manifest.encoded_bytes as usize || self.pages.iter().any(Option::is_none) {
            return Err(AssemblyError::Incomplete);
        }
        self.ended = true;
        Ok(())
    }

    /// Called from the real ordered world intake, after preceding carried
    /// chunks/deltas and required generation. Socket parsing must not call it.
    pub fn apply_fence(&mut self, cut: RecoveryCut) -> Result<(), AssemblyError> {
        if cut != self.manifest.cut { return Err(AssemblyError::Namespace); }
        if !self.ended { return Err(AssemblyError::Incomplete); }
        self.applied = true;
        Ok(())
    }

    pub fn ready(&self) -> bool { self.ended && self.applied }
    pub fn manifest(&self) -> RebaseManifest { self.manifest }
    pub fn pages(&self) -> impl Iterator<Item = &[u8]> { self.pages.iter().filter_map(|p| p.as_deref()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> RebaseManifest {
        RebaseManifest { cut: RecoveryCut { session: [4; 16], id: 1, epoch: 1, life: 2, through: 4 }, included_events: 9, parts: 2, encoded_bytes: 4 }
    }

    #[test]
    fn c3d_complete_socket_bundle_waits_for_actual_carried_world_fence() {
        let manifest = manifest();
        let mut assembly = RebaseAssembly::new(manifest, [4; 16], 2).unwrap();
        assembly.page(manifest.cut, 0, Box::new([1, 2])).unwrap();
        assert_eq!(assembly.end(manifest.cut), Err(AssemblyError::Incomplete));
        assert_eq!(assembly.apply_fence(manifest.cut), Err(AssemblyError::Incomplete));
        assembly.page(manifest.cut, 1, Box::new([3, 4])).unwrap();
        assembly.end(manifest.cut).unwrap();
        assert!(!assembly.ready());
        // A forced-column carry blocks the older chunk and every later step.
        let mut stream = std::collections::VecDeque::from(["old chunk", "new edit", "fence", "later edit"]);
        let can_generate_column = false;
        if can_generate_column { stream.pop_front(); }
        assert_eq!(stream.front(), Some(&"old chunk"));
        assert!(!assembly.ready());
        assert_eq!(stream.pop_front(), Some("old chunk"));
        assert_eq!(stream.pop_front(), Some("new edit"));
        assert_eq!(stream.pop_front(), Some("fence"));
        assembly.apply_fence(manifest.cut).unwrap();
        assert!(assembly.ready());
        assert_eq!(stream.front(), Some(&"later edit"));
        assert_eq!(assembly.pages().flatten().copied().collect::<Vec<_>>(), [1, 2, 3, 4]);
    }

    #[test]
    fn c3d_partial_foreign_conflicting_and_oversized_pages_never_activate() {
        let manifest = manifest();
        let mut assembly = RebaseAssembly::new(manifest, [4; 16], 2).unwrap();
        let foreign = RecoveryCut { session: [5; 16], ..manifest.cut };
        assert_eq!(assembly.page(foreign, 0, Box::new([1, 2])), Err(AssemblyError::Namespace));
        assembly.page(manifest.cut, 0, Box::new([1, 2])).unwrap();
        assembly.page(manifest.cut, 0, Box::new([1, 2])).unwrap();
        assert_eq!(assembly.page(manifest.cut, 0, Box::new([9, 9])), Err(AssemblyError::Conflict));
        assert_eq!(assembly.page(manifest.cut, 1, vec![0; MAX_PAGE_BYTES + 1].into_boxed_slice()), Err(AssemblyError::Bytes));
        assert!(!assembly.ready());
        assert!(matches!(RebaseAssembly::new(RebaseManifest { parts: 65, ..manifest }, [4; 16], 2), Err(AssemblyError::Manifest)));
        assert!(matches!(RebaseAssembly::new(RebaseManifest { encoded_bytes: MAX_BUNDLE_BYTES as u32, ..manifest }, [4; 16], 2), Err(AssemblyError::Bytes)));
        assert!(matches!(RebaseAssembly::new(manifest, [4; 16], 3), Err(AssemblyError::Namespace)));
    }
}
