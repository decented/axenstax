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
    if body.marker.is_some() {
        return;
    }
    let marker = crate::plan::marker(body);
    let key = cache_key(marker);
    if REMEMBERED.with(|set| set.borrow().contains(&key)) {
        return;
    }
    let Ok(bytes) = bincode::serialize(body) else {
        return;
    };
    if bytes.len() > 16 * 1024 * 1024 {
        log::warn!("Local Plan body exceeds the store limit");
        return;
    }
    match persist(marker, bytes) {
        Ok(()) => {
            REMEMBERED.with(|set| {
                set.borrow_mut().insert(key);
            });
        }
        Err(e) => log::warn!("Could not remember local Plan: {e}"),
    }
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
fn persist(marker: [u8; 32], bytes: Vec<u8>) -> Result<(), String> {
    let dir = root();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    crate::save::write_atomic(&dir.join(hex::encode(marker)), &bytes)
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
fn persist(marker: [u8; 32], bytes: Vec<u8>) -> Result<(), String> {
    js_remember(
        hex::encode(marker),
        js_sys::Uint8Array::from(bytes.as_slice()),
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
