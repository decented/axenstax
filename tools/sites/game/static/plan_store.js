/* Anonymous local Plan bodies: IndexedDB persistence and a synchronous cache. */
(() => {
  const cache = new Map();
  const ready = new Promise((resolve, reject) => {
    const request = indexedDB.open('axenstax-plans', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('bodies');
    request.onerror = () => reject(request.error);
    request.onsuccess = () => resolve(request.result);
  });
  // Engine boot waits for this preload, so synchronous resolution sees saved bodies.
  window.axenstax_plan_store_ready = ready.then(db => new Promise((resolve, reject) => {
    const transaction = db.transaction('bodies', 'readonly');
    const cursor = transaction.objectStore('bodies').openCursor();
    cursor.onsuccess = () => {
      const row = cursor.result;
      if (row) { cache.set(row.key, new Uint8Array(row.value)); row.continue(); }
    };
    transaction.oncomplete = resolve;
    transaction.onerror = () => reject(transaction.error);
  })).catch(error => console.warn('Local Plan preload failed', error));
  window.axenstax_plan_resolve = marker => cache.get(marker) || null;
  window.axenstax_plan_remember = (marker, bytes) => {
    const copy = new Uint8Array(bytes);
    cache.set(marker, copy);
    ready.then(db => {
      const transaction = db.transaction('bodies', 'readwrite');
      transaction.objectStore('bodies').put(copy, marker);
      transaction.onerror = () => console.warn('Local Plan save failed', transaction.error);
    }).catch(error => console.warn('Local Plan save failed', error));
  };
})();
