//! Client-local, content-addressed Plan bodies. Markers never imply body transfer.
use crate::plan::PlanData;

thread_local! { static REMEMBERED: std::cell::RefCell<std::collections::HashSet<String>> = std::cell::RefCell::new(Default::default()); }
fn cache_key(marker: [u8; 32]) -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        root()
            .join(hex::encode(marker))
            .to_string_lossy()
            .into_owned()
    }
    #[cfg(target_arch = "wasm32")]
    {
        hex::encode(marker)
    }
}
fn builtin(marker: [u8; 32]) -> Option<PlanData> {
    [
        crate::satoshi::starter_hut_plan(),
        PlanData::debug_3x3_stone(),
        PlanData::small_hut(),
    ]
    .into_iter()
    .find(|p| crate::plan::marker(p) == marker)
}
fn decode(bytes: &[u8], marker: [u8; 32]) -> Option<PlanData> {
    use bincode::Options;
    let body: PlanData = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(16 * 1024 * 1024)
        .reject_trailing_bytes()
        .deserialize(bytes)
        .ok()?;
    (body.marker.is_none() && crate::plan::marker(&body) == marker).then_some(body)
}
pub(crate) fn remember_item(item: &crate::item::Item) {
    if let crate::item::Item::Plan(body) = item {
        remember(body);
    }
}
pub(crate) fn remember(body: &PlanData) {
    // Run 4 (run-3 review L2) — Mirror's single-body store is main's: the
    // 16 MiB ENCODED limit only, no 64 MiB residency cap (that bounds Strict
    // journal residency, `remember_checked` with a real resident count).
    if let Err(error) = remember_checked(body, 0) { log::warn!("Could not remember local Plan: {error}"); }
}

/// Native joined history may release ownership only after this returns success.
/// A failed atomic write is never cached, so the same body can be retried.
pub(crate) fn remember_checked(body: &PlanData, resident_bytes: usize) -> Result<(), String> {
    if resident_bytes > crate::action_capacity::MAX_RESIDENT_BODY_BYTES { return Err("Local Plan residency exceeds its limit".into()); }
    if body.marker.is_some() { return Ok(()); }
    let marker = crate::plan::marker(body);
    let key = cache_key(marker);
    if REMEMBERED.with(|set| set.borrow().contains(&key)) { return Ok(()); }
    let encoded = usize::try_from(bincode::serialized_size(body).map_err(|e| e.to_string())?).map_err(|_| "Local Plan encoding cannot be bounded")?;
    // C3d review L3 (coordinator decision): the 16 MiB store limit counts the
    // ENCODED size, as before C3d (`decode` limits at 16 MiB), never the
    // allocated capacity: a body that encodes within it is stored.
    if encoded > crate::action_capacity::MAX_PRIVATE_BODY_BYTES {
        return Err("Local Plan body exceeds the store limit".into());
    }
    // Run 4 (run-3 review L2) — the reservation is the encoded bytes alone,
    // so exactly main's limit holds: `encoded <= 16 MiB` is stored.
    crate::action_capacity::reserve_body(resident_bytes, encoded).map_err(|e| format!("Local Plan encoding reservation failed: {e:?}"))?;
    // Fixed output cannot grow during serialisation. No owned Plan clone and no
    // second body-sized hashing buffer enters the reservation.
    let mut bytes = vec![0; encoded].into_boxed_slice();
    bincode::serialize_into(std::io::Cursor::new(bytes.as_mut()), body).map_err(|e| e.to_string())?;
    persist(marker, &bytes)?;
    REMEMBERED.with(|set| { set.borrow_mut().insert(key); });
    Ok(())
}
pub(crate) fn resolve(marker: [u8; 32]) -> Option<PlanData> {
    builtin(marker).or_else(|| load(marker).and_then(|bytes| decode(&bytes, marker)))
}
#[cfg(not(target_arch = "wasm32"))]
fn root() -> std::path::PathBuf {
    #[cfg(test)]
    {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        crate::save::worlds_root().hash(&mut hash);
        std::env::temp_dir().join(format!(
            "sidecar-plan-tests-{}-{:x}",
            std::process::id(),
            hash.finish()
        ))
    }
    #[cfg(not(test))]
    {
        crate::data_dir::data_root().join("plan_store")
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn persist(marker: [u8; 32], bytes: &[u8]) -> Result<(), String> {
    let dir = root();
    ensure_checked_dir(&dir)?;
    // Run 5 — Windows (UNCOMPILED on the development host, which can't
    // build a Windows target): the file is fsynced and renamed, with no
    // directory sync (a directory handle can't be flushed without
    // `GENERIC_WRITE`; NTFS journals the rename).
    #[cfg(windows)]
    { crate::save::write_atomic(&dir.join(hex::encode(marker)), bytes) }
    #[cfg(not(windows))]
    {
        crate::save::write_atomic(&dir.join(hex::encode(marker)), bytes)?;
        sync_checked_dir(&dir)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn ensure_checked_dir(dir: &std::path::Path) -> Result<(), String> {
    if dir.is_dir() { return Ok(()); }
    let parent = dir.parent().filter(|p| !p.as_os_str().is_empty()).ok_or("Plan-store directory has no parent")?;
    ensure_checked_dir(parent)?;
    match std::fs::create_dir(dir) {
        Ok(()) => {},
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && dir.is_dir() => {},
        Err(error) => return Err(error.to_string()),
    }
    #[cfg(windows)]
    { let _ = parent; Ok(()) }
    #[cfg(not(windows))]
    sync_checked_dir(parent)
}

/// Checked directory durability is deliberately narrower than shared save's
/// best-effort sync. Unsupported native filesystems/platforms return failure;
/// joined history keeps its owned body and pauses instead of claiming success.
#[cfg(all(not(target_arch = "wasm32"), not(windows)))]
fn sync_checked_dir(dir: &std::path::Path) -> Result<(), String> {
    crate::fsync::sync_dir(dir).map_err(|e| format!("Plan-store directory was not durably synced: {e}"))
}

#[cfg(not(target_arch = "wasm32"))]
fn load(marker: [u8; 32]) -> Option<Vec<u8>> {
    std::fs::File::open(root().join(hex::encode(marker)))
        .ok()
        .and_then(|file| crate::save::read_bounded(file, 16 * 1024 * 1024).ok())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = axenstax_plan_remember, catch)]
    fn js_remember(marker: String, bytes: js_sys::Uint8Array) -> Result<(), wasm_bindgen::JsValue>;
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = axenstax_plan_resolve, catch)]
    fn js_resolve(marker: String) -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>;
}
#[cfg(target_arch = "wasm32")]
fn persist(marker: [u8; 32], bytes: &[u8]) -> Result<(), String> {
    js_remember(
        hex::encode(marker),
        js_sys::Uint8Array::from(bytes),
    )
    .map_err(|e| format!("{e:?}"))
}
#[cfg(target_arch = "wasm32")]
fn load(marker: [u8; 32]) -> Option<Vec<u8>> {
    let value = js_resolve(hex::encode(marker)).ok()?;
    (!value.is_null() && !value.is_undefined()).then(|| js_sys::Uint8Array::new(&value).to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    // Unix (Android included) only: Windows has no directory sync to fail.
    #[cfg(unix)]
    #[test]
    fn c3d_post_rename_sync_failure_keeps_actual_held_body_and_remains_retryable() {
        let _g = crate::save::WorldsRootGuard::new("c3d-plans-sync-failure");
        let mut body = PlanData::debug_3x3_stone();
        body.name = "Held body survives checked directory sync failure".into();
        let marker = crate::plan::marker(&body);
        let mut inv = crate::inventory::Inventory::new_client();
        std::fs::create_dir_all(root()).unwrap();
        // A client inventory remembers a Plan body as it is set
        // (`remember_plans`): inject the failure first, so that store is
        // the failing one too and nothing is cached before the retain.
        let failure = crate::fsync::tests::fail_directory_on_match(&root(), 1);
        inv.set_slot(0, Some(crate::item::ItemStack { item: crate::item::Item::Plan(body), count: 1 }));
        let failed = crate::window_prediction::retain_private_window(&inv, &None, &Default::default());
        assert!(failure.failures() >= 2, "automatic remembering and checked retention both reached the shared directory helper");
        assert!(failed.unwrap_err().starts_with("Plan-store directory was not durably synced:"));
        assert!(root().join(hex::encode(marker)).is_file(), "failure is after rename, not an absent-file shortcut");
        assert!(!REMEMBERED.with(|set| set.borrow().contains(&cache_key(marker))));
        let crate::item::Item::Plan(held) = &inv.slot(0).unwrap().item else { panic!("body ownership lost") };
        assert!(!held.cells.is_empty());
        assert_eq!(crate::plan::marker(held), marker);
        drop(failure);
        crate::window_prediction::retain_private_window(&inv, &None, &Default::default()).unwrap();
        assert!(REMEMBERED.with(|set| set.borrow().contains(&cache_key(marker))));
        assert_eq!(resolve(marker).unwrap().cells, held.cells);
    }

    /// C3d review L3 — the 16 MiB store limit counts the ENCODED size, as
    /// before C3d: a body whose allocation is over 16 MiB but whose
    /// encoding is small is stored (its marker resolves after a rejoin).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn c3d_store_limit_counts_encoded_size_not_allocated_capacity() {
        let _g = crate::save::WorldsRootGuard::new("c3d-plans-encoded-limit");
        let mut body = PlanData::debug_3x3_stone();
        body.name = "Large allocation, small encoding".into();
        let element = std::mem::size_of_val(&body.cells[0]);
        body.cells.reserve_exact(crate::action_capacity::MAX_PRIVATE_BODY_BYTES / element + 1);
        assert!(crate::body_residency::allocated_body_bytes(&body).unwrap() > crate::action_capacity::MAX_PRIVATE_BODY_BYTES);
        let marker = crate::plan::marker(&body);
        remember_checked(&body, crate::body_residency::allocated_body_bytes(&body).unwrap()).expect("an encoding within 16 MiB is stored");
        assert_eq!(resolve(marker).unwrap().cells, body.cells);
    }

    /// Run 4 (run-3 review L2) — Mirror's store is main's exactly: a body
    /// whose ENCODING is exactly 16 MiB is stored and resolves (no 16-byte
    /// box in the reservation, no residency cap on the single-body store).
    #[test]
    fn c3d_mirror_store_accepts_a_body_encoded_at_exactly_the_limit() {
        let _g = crate::save::WorldsRootGuard::new("c3d-plans-exact-limit");
        let mut body = PlanData::debug_3x3_stone();
        body.name = String::new();
        let base = usize::try_from(bincode::serialized_size(&body).unwrap()).unwrap();
        body.name = "x".repeat(crate::action_capacity::MAX_PRIVATE_BODY_BYTES - base);
        assert_eq!(usize::try_from(bincode::serialized_size(&body).unwrap()).unwrap(), crate::action_capacity::MAX_PRIVATE_BODY_BYTES);
        let marker = crate::plan::marker(&body);
        remember(&body);
        assert!(REMEMBERED.with(|set| set.borrow().contains(&cache_key(marker))), "stored and cached");
        assert_eq!(resolve(marker).map(|b| b.name.len()), Some(body.name.len()), "and it resolves");
        body.name.push('x');
        let over = crate::plan::marker(&body);
        remember(&body);
        assert!(resolve(over).is_none(), "one byte over is not stored");
    }

    /// Run 11 (run-4 review LOW-5) — pins "no residency cap on Mirror":
    /// Mirror's `remember` stores a body whose allocation alone is past the
    /// 64 MiB residency cap while its encoding is small. Passing the
    /// allocated bytes as resident (instead of 0) would refuse it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn c3d_mirror_store_has_no_residency_cap() {
        let _g = crate::save::WorldsRootGuard::new("c3d-plans-no-residency-cap");
        let mut body = PlanData::debug_3x3_stone();
        body.name = "Past the residency cap in allocation, small in encoding".into();
        let element = std::mem::size_of_val(&body.cells[0]);
        body.cells.reserve_exact(crate::action_capacity::MAX_RESIDENT_BODY_BYTES / element + 1);
        assert!(crate::body_residency::allocated_body_bytes(&body).unwrap() > crate::action_capacity::MAX_RESIDENT_BODY_BYTES);
        let marker = crate::plan::marker(&body);
        remember(&body);
        assert!(REMEMBERED.with(|set| set.borrow().contains(&cache_key(marker))), "Mirror stored it: no residency cap");
        assert_eq!(resolve(marker).map(|b| b.cells), Some(body.cells.clone()), "and it resolves");
    }

    #[test]
    fn c3d_encoding_scratch_is_reserved_before_store_allocation_or_write() {
        let _g = crate::save::WorldsRootGuard::new("c3d-plans-scratch");
        let mut body = PlanData::debug_3x3_stone();
        body.name = "Scratch reservation refuses before write".into();
        let marker = crate::plan::marker(&body);
        let encoded = usize::try_from(bincode::serialized_size(&body).unwrap()).unwrap();
        let resident = crate::action_capacity::MAX_RESIDENT_BODY_BYTES - encoded + 1;
        assert!(remember_checked(&body, resident).is_err());
        assert!(!REMEMBERED.with(|set| set.borrow().contains(&cache_key(marker))));
        assert!(!root().join(hex::encode(marker)).exists());
        assert!(!body.cells.is_empty());
    }

    #[test]
    fn sidecar_plan_store_retries_after_failed_atomic_write() {
        let _g = crate::save::WorldsRootGuard::new("sidecar-plans-retry");
        let mut body = PlanData::debug_3x3_stone();
        body.name = "Retry this genuine body".into();
        let marker = crate::plan::marker(&body);
        std::fs::write(root(), b"blocks directory creation").unwrap();
        remember(&body);
        assert!(resolve(marker).is_none());
        std::fs::remove_file(root()).unwrap();
        remember(&body);
        assert_eq!(resolve(marker).unwrap().cells, body.cells);
    }
    #[test]
    fn sidecar_plan_store_keeps_spent_body_and_rejects_stub_or_wrong_hash() {
        let _g = crate::save::WorldsRootGuard::new("sidecar-plans");
        let mut body = PlanData::debug_3x3_stone();
        body.name = "A local captured Plan".into();
        let marker = crate::plan::marker(&body);
        let mut window = crate::inventory::Inventory::new_client();
        window.add_item(crate::item::ItemStack {
            item: crate::item::Item::Plan(body.clone()),
            count: 1,
        });
        // Immediately spend it before any frame/tick scan could remember it.
        window.set_slot(0, None);
        remember(&PlanData::marker_placeholder(marker, true));
        assert_eq!(resolve(marker).unwrap().cells, body.cells);
        assert!(resolve([42; 32]).is_none());
        assert!(decode(&bincode::serialize(&body).unwrap(), [42; 32]).is_none());
        std::fs::write(root().join(hex::encode(marker)), b"broken").unwrap();
        assert!(resolve(marker).is_none());
    }
}
