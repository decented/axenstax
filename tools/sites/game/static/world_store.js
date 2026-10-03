// Axe'n'Stax — IndexedDB world store (Phase 1α local-first worlds).
//
// Stores packed world blobs (tar+gzip, produced by pack_world on the Rust
// side) keyed by "<pubkey>:<world_name>". Per-pubkey scoping is the only
// isolation — no encryption at rest for alpha.
//
// Exposes six async functions on `window`:
//   axenstax_save_world(pubkey, name, u8array, meta_json)  → Promise<void>
//   axenstax_load_world(pubkey, name)                      → Promise<Uint8Array | null>
//   axenstax_list_worlds(pubkey)                           → Promise<Array<meta>>
//   axenstax_delete_world(pubkey, name)                    → Promise<void>
//   axenstax_export_world(pubkey, name)                    → Promise<void>  (triggers a <name>.axeworld download)
//   axenstax_profile_download(name, u8array)               → Promise<void>  (triggers a <name> .axeprofile download)
//   axenstax_pick_world_file()                             → Promise<{name, bytes} | null>
//   axenstax_pick_skin_file()                              → Promise<{name, bytes} | null>  (a 64×64 PNG skin)

(function () {
    'use strict';

    const DB_NAME = 'axenstax_worlds';
    const DB_VERSION = 1;
    const STORE = 'worlds';

    function openDb() {
        return new Promise((resolve, reject) => {
            const req = indexedDB.open(DB_NAME, DB_VERSION);
            req.onupgradeneeded = () => {
                const db = req.result;
                if (!db.objectStoreNames.contains(STORE)) {
                    const store = db.createObjectStore(STORE, { keyPath: 'key' });
                    store.createIndex('by_pubkey', 'pubkey', { unique: false });
                }
            };
            req.onsuccess = () => resolve(req.result);
            req.onerror = () => reject(req.error || new Error('open failed'));
        });
    }

    function key(pubkey, name) {
        return pubkey + ':' + name;
    }

    function validPubkey(pk) {
        // "local" is the guest namespace for the login-free web sandbox — the
        // engine's `save::wasm_storage_key()` returns it when there's no pubkey.
        // Accept it alongside a 64-hex persona key so guest worlds save/load/list/
        // delete/export instead of throwing "bad pubkey".
        return pk === 'local' || (typeof pk === 'string' && /^[0-9a-f]{64}$/i.test(pk));
    }

    async function save(pubkey, name, u8array, meta_json) {
        if (!validPubkey(pubkey)) throw new Error('bad pubkey');
        if (!name) throw new Error('bad name');
        let meta;
        try { meta = JSON.parse(meta_json); }
        catch (e) { throw new Error('meta parse: ' + e.message); }
        const db = await openDb();
        try {
            await new Promise((resolve, reject) => {
                const tx = db.transaction(STORE, 'readwrite');
                tx.oncomplete = resolve;
                tx.onerror = () => reject(tx.error || new Error('tx failed'));
                tx.onabort = () => reject(tx.error || new Error('tx aborted — likely QuotaExceeded'));
                tx.objectStore(STORE).put({
                    key: key(pubkey, name),
                    pubkey,
                    name,
                    blob: u8array,
                    meta,
                    last_saved: Math.floor(Date.now() / 1000),
                    size: u8array.byteLength,
                });
            });
        } catch (e) {
            if (e && e.name === 'QuotaExceededError') {
                throw new Error('quota');
            }
            throw e;
        } finally {
            db.close();
        }
    }

    async function load(pubkey, name) {
        if (!validPubkey(pubkey)) throw new Error('bad pubkey');
        const db = await openDb();
        try {
            return await new Promise((resolve, reject) => {
                const tx = db.transaction(STORE, 'readonly');
                const req = tx.objectStore(STORE).get(key(pubkey, name));
                req.onsuccess = () => {
                    const rec = req.result;
                    if (!rec) { resolve(null); return; }
                    // blob may be ArrayBuffer or Uint8Array depending on how it was stored
                    const blob = rec.blob instanceof Uint8Array
                        ? rec.blob
                        : new Uint8Array(rec.blob);
                    resolve(blob);
                };
                req.onerror = () => reject(req.error || new Error('get failed'));
            });
        } finally {
            db.close();
        }
    }

    async function list(pubkey) {
        if (!validPubkey(pubkey)) throw new Error('bad pubkey');
        const db = await openDb();
        try {
            return await new Promise((resolve, reject) => {
                const tx = db.transaction(STORE, 'readonly');
                const store = tx.objectStore(STORE);
                const index = store.index('by_pubkey');
                const req = index.openCursor(IDBKeyRange.only(pubkey));
                const out = [];
                req.onsuccess = () => {
                    const cursor = req.result;
                    if (!cursor) { resolve(out); return; }
                    const r = cursor.value;
                    out.push({
                        name: r.name,
                        size: r.size,
                        last_saved: r.last_saved,
                        game_mode: (r.meta && r.meta.game_mode) || 'survival',
                        display_name: (r.meta && r.meta.display_name) || r.name,
                        description: (r.meta && r.meta.description) || '',
                        difficulty: (r.meta && r.meta.difficulty) || 'normal',
                        cloud_save: !!(r.meta && r.meta.cloud_save),
                    });
                    cursor.continue();
                };
                req.onerror = () => reject(req.error || new Error('cursor failed'));
            });
        } finally {
            db.close();
        }
    }

    async function del(pubkey, name) {
        if (!validPubkey(pubkey)) throw new Error('bad pubkey');
        const db = await openDb();
        try {
            await new Promise((resolve, reject) => {
                const tx = db.transaction(STORE, 'readwrite');
                tx.oncomplete = resolve;
                tx.onerror = () => reject(tx.error || new Error('tx failed'));
                tx.objectStore(STORE).delete(key(pubkey, name));
            });
        } finally {
            db.close();
        }
    }

    // Export a world's packed blob as a `<name>.axeworld` file download. Reads
    // the IDB record at key(pubkey, name), wraps its blob in a Blob, and drives
    // a temporary <a download> click. Rejects if the record is missing.
    async function exportWorld(pubkey, name) {
        if (!validPubkey(pubkey)) throw new Error('bad pubkey');
        if (!name) throw new Error('bad name');
        const db = await openDb();
        let rec;
        try {
            rec = await new Promise((resolve, reject) => {
                const tx = db.transaction(STORE, 'readonly');
                const req = tx.objectStore(STORE).get(key(pubkey, name));
                req.onsuccess = () => resolve(req.result);
                req.onerror = () => reject(req.error || new Error('get failed'));
            });
        } finally {
            db.close();
        }
        if (!rec) throw new Error('world not found: ' + name);
        const blob = new Blob([rec.blob], { type: 'application/octet-stream' });
        const url = URL.createObjectURL(blob);
        try {
            const a = document.createElement('a');
            a.href = url;
            a.download = name + '.axeworld';
            a.style.display = 'none';
            document.body.appendChild(a);
            a.click();
            document.body.removeChild(a);
        } finally {
            // Revoke after a tick so the download has a chance to start.
            setTimeout(() => URL.revokeObjectURL(url), 0);
        }
    }

    // "Take your worlds to native" — download a whole-profile bundle the
    // desktop app can import. `bytes` is the packed `.axeprofile` container
    // built engine-side (profile_bundle::pack); this only wraps it in a Blob
    // and drives the same temporary <a download> click as exportWorld. Purely
    // local: nothing is uploaded, nothing leaves the machine.
    async function profileDownload(name, bytes) {
        const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
        const blob = new Blob([u8], { type: 'application/octet-stream' });
        const url = URL.createObjectURL(blob);
        try {
            const a = document.createElement('a');
            a.href = url;
            a.download = name || 'axenstax-profile.axeprofile';
            a.style.display = 'none';
            document.body.appendChild(a);
            a.click();
            document.body.removeChild(a);
        } finally {
            // Revoke after a tick so the download has a chance to start.
            setTimeout(() => URL.revokeObjectURL(url), 0);
        }
    }

    // Open a file picker for `.axeworld` files. Resolves with
    // { name, bytes } (basename without extension + the file as a Uint8Array)
    // once a file is chosen, or `null` if the picker is cancelled/closed.
    function pickWorldFile() {
        return new Promise((resolve, reject) => {
            const input = document.createElement('input');
            input.type = 'file';
            input.accept = '.axeworld';
            input.style.display = 'none';
            let settled = false;

            const cleanup = () => {
                if (input.parentNode) input.parentNode.removeChild(input);
            };

            input.addEventListener('change', () => {
                if (settled) return;
                settled = true;
                const file = input.files && input.files[0];
                if (!file) { cleanup(); resolve(null); return; }
                const basename = file.name.replace(/\.axeworld$/i, '');
                const reader = new FileReader();
                reader.onload = () => {
                    cleanup();
                    resolve({ name: basename, bytes: new Uint8Array(reader.result) });
                };
                reader.onerror = () => {
                    cleanup();
                    reject(reader.error || new Error('read failed'));
                };
                reader.readAsArrayBuffer(file);
            });

            // `cancel` fires when the picker is dismissed without a selection
            // (supported in modern Chromium). Treated as a no-op (null).
            input.addEventListener('cancel', () => {
                if (settled) return;
                settled = true;
                cleanup();
                resolve(null);
            });

            document.body.appendChild(input);
            input.click();
        });
    }

    // Open a file picker for a PNG skin (the player's 64×64 avatar texture).
    // Resolves with { name, bytes } (the original filename + the file as a
    // Uint8Array), or `null` if the picker is cancelled/closed. Unlike
    // pickWorldFile this needs no pubkey — it only reads a local file; the
    // caller decodes/validates the PNG and persists it via the cosmetic bridge.
    function pickSkinFile() {
        return new Promise((resolve, reject) => {
            const input = document.createElement('input');
            input.type = 'file';
            input.accept = '.png,image/png';
            input.style.display = 'none';
            let settled = false;

            const cleanup = () => {
                if (input.parentNode) input.parentNode.removeChild(input);
            };

            input.addEventListener('change', () => {
                if (settled) return;
                settled = true;
                const file = input.files && input.files[0];
                if (!file) { cleanup(); resolve(null); return; }
                const reader = new FileReader();
                reader.onload = () => {
                    cleanup();
                    resolve({ name: file.name, bytes: new Uint8Array(reader.result) });
                };
                reader.onerror = () => {
                    cleanup();
                    reject(reader.error || new Error('read failed'));
                };
                reader.readAsArrayBuffer(file);
            });

            // `cancel` fires when the picker is dismissed without a selection
            // (supported in modern Chromium). Treated as a no-op (null).
            input.addEventListener('cancel', () => {
                if (settled) return;
                settled = true;
                cleanup();
                resolve(null);
            });

            // Window-focus fallback: the `cancel` event isn't reliably delivered
            // on every dismiss path (OS dialog quirks), which would leave this
            // promise pending forever and the engine's `picking` flag stuck true
            // — disabling Upload for the rest of the session. The window regains
            // focus when the dialog closes, so on the first focus after opening,
            // give `change` a short grace period to win, then settle as cancelled
            // if no file was chosen. Guarded by `settled` so a real `change`
            // (or `cancel`) still takes precedence.
            const onFocus = () => {
                setTimeout(() => {
                    if (settled) return;
                    if (!input.files || input.files.length === 0) {
                        settled = true;
                        cleanup();
                        resolve(null);
                    }
                }, 400);
            };
            window.addEventListener('focus', onFocus, { once: true });

            document.body.appendChild(input);
            input.click();
        });
    }

    // Exhibits (Creator Gallery) — fetch one authored exhibit image, served
    // same-origin from the world's exhibit set. Sanitised ref; rejects on a bad
    // ref / HTTP error so the engine skips that exhibit and keeps walking. (The
    // server route is finalised in Phase 3/4; this is the client contract.)
    async function loadExhibitArt(world, imageRef) {
        const safe = String(imageRef).replace(/[^a-z0-9_.-]/gi, '');
        if (!safe || safe.indexOf('..') !== -1) throw new Error('bad exhibit ref');
        const resp = await fetch('/exhibits/' + safe, { cache: 'force-cache' });
        if (!resp.ok) throw new Error('exhibit ' + safe + ' HTTP ' + resp.status);
        const buf = await resp.arrayBuffer();
        return new Uint8Array(buf);
    }

    window.axenstax_save_world = save;
    window.axenstax_load_world = load;
    window.axenstax_load_exhibit_art = loadExhibitArt;
    window.axenstax_list_worlds = list;
    window.axenstax_delete_world = del;
    window.axenstax_export_world = exportWorld;
    window.axenstax_profile_download = profileDownload;
    window.axenstax_pick_world_file = pickWorldFile;
    // Campaign G — pick an .axeghost file (a shared race ghost). Same
    // input-element flow as pickSkinFile, different accept filter.
    function pickGhostFile() {
        return new Promise((resolve, reject) => {
            const input = document.createElement('input');
            input.type = 'file';
            input.accept = '.axeghost,application/json';
            input.style.display = 'none';
            let settled = false;

            const cleanup = () => {
                if (input.parentNode) input.parentNode.removeChild(input);
            };

            input.addEventListener('change', () => {
                if (settled) return;
                settled = true;
                const file = input.files && input.files[0];
                if (!file) { cleanup(); resolve(null); return; }
                const reader = new FileReader();
                reader.onload = () => {
                    cleanup();
                    resolve({ name: file.name, bytes: new Uint8Array(reader.result) });
                };
                reader.onerror = () => {
                    cleanup();
                    reject(reader.error || new Error('read failed'));
                };
                reader.readAsArrayBuffer(file);
            });

            input.addEventListener('cancel', () => {
                if (settled) return;
                settled = true;
                cleanup();
                resolve(null);
            });

            document.body.appendChild(input);
            input.click();
        });
    }

    window.axenstax_pick_skin_file = pickSkinFile;
    window.axenstax_pick_ghost_file = pickGhostFile;
})();
