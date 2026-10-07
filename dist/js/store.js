// A small key-value store in IndexedDB, for things too big for localStorage (your likes, track
// lists). Everything here is a cache: if it's missing or unreadable, the app just fetches again.

let storeDb = null;

function storeOpen() {
  if (storeDb) return storeDb;
  storeDb = new Promise((resolve) => {
    try {
      const req = indexedDB.open("sc-client", 1);
      req.onupgradeneeded = () => req.result.createObjectStore("kv");
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => resolve(null);
      req.onblocked = () => resolve(null);
    } catch (e) {
      resolve(null);
    }
  });
  return storeDb;
}

async function storeGet(key) {
  const db = await storeOpen();
  if (!db) return null;
  return new Promise((resolve) => {
    try {
      const req = db.transaction("kv").objectStore("kv").get(key);
      req.onsuccess = () => resolve(req.result || null);
      req.onerror = () => resolve(null);
    } catch (e) {
      resolve(null);
    }
  });
}

async function storeSet(key, value) {
  const db = await storeOpen();
  if (!db) return;
  try { db.transaction("kv", "readwrite").objectStore("kv").put(value, key); } catch (e) {}
}

// Cached tracks back into track objects (fresh copies, so changes don't touch the cache)
const fromSnap = (s) => Object.assign({}, s, { user: Object.assign({}, s.user) });

// A track list kept under a key: { at, tracks }
async function cacheTracks(key, tracks) {
  await storeSet(key, { at: Date.now(), tracks: tracks.map(xplSnap) });
}
async function cachedTracks(key) {
  const c = await storeGet(key);
  return c && Array.isArray(c.tracks) ? { at: c.at || 0, tracks: c.tracks.map(fromSnap) } : null;
}
