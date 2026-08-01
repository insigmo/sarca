/**
 * IndexedDB-backed cache for full-size photo previews on the web UI.
 *
 * Mirrors the native client's on-disk preview cache (see
 * client/src-tauri/src/commands.rs: cache_get_preview/cache_put_preview):
 * content-addressed by (scope, path) rather than by request URL, since the
 * preview URL embeds a short-lived access token and would otherwise defeat
 * plain HTTP caching. Size-capped with LRU eviction by last-access time.
 *
 * localStorage cannot hold this much binary data (most browsers cap it at
 * 5-10MB total per origin) so this uses IndexedDB instead, which supports a
 * much larger quota.
 */

const DB_NAME = 'sarca-preview-cache'
const STORE = 'previews'
const DB_VERSION = 1
export const PREVIEW_CACHE_LIMIT_BYTES = 100 * 1024 * 1024

/** @type {Promise<IDBDatabase> | null} */
let dbPromise = null

const openDb = () => {
	if (typeof indexedDB === 'undefined') return Promise.reject(new Error('indexedDB unavailable'))
	if (dbPromise) return dbPromise
	dbPromise = new Promise((resolve, reject) => {
		const req = indexedDB.open(DB_NAME, DB_VERSION)
		req.onupgradeneeded = () => {
			const db = req.result
			if (!db.objectStoreNames.contains(STORE)) {
				const store = db.createObjectStore(STORE, { keyPath: 'key' })
				store.createIndex('ts', 'ts')
			}
		}
		req.onsuccess = () => resolve(req.result)
		req.onerror = () => reject(req.error || new Error('indexedDB open failed'))
	})
	return dbPromise
}

const cacheKey = (scope, path) => `${scope}:${path}`

/**
 * @param {string} scope
 * @param {string} path
 * @returns {Promise<Blob | null>}
 */
export const getCachedPreview = async (scope, path) => {
	try {
		const db = await openDb()
		const entry = await new Promise((resolve, reject) => {
			const tx = db.transaction(STORE, 'readonly')
			const req = tx.objectStore(STORE).get(cacheKey(scope, path))
			req.onsuccess = () => resolve(req.result || null)
			req.onerror = () => reject(req.error || new Error('indexedDB get failed'))
		})
		if (!entry) return null
		// Touch last-access time for LRU (best-effort, don't block the read on it).
		touchEntry(db, entry).catch(() => {})
		return entry.blob
	} catch {
		return null
	}
}

const touchEntry = (db, entry) =>
	new Promise((resolve, reject) => {
		const tx = db.transaction(STORE, 'readwrite')
		tx.objectStore(STORE).put({ ...entry, ts: Date.now() })
		tx.oncomplete = () => resolve()
		tx.onerror = () => reject(tx.error || new Error('indexedDB touch failed'))
	})

/**
 * @param {string} scope
 * @param {string} path
 * @param {Blob} blob
 * @returns {Promise<void>}
 */
export const putCachedPreview = async (scope, path, blob) => {
	try {
		const db = await openDb()
		await new Promise((resolve, reject) => {
			const tx = db.transaction(STORE, 'readwrite')
			tx.objectStore(STORE).put({
				key: cacheKey(scope, path),
				scope,
				path,
				blob,
				size: blob.size,
				ts: Date.now(),
			})
			tx.oncomplete = () => resolve()
			tx.onerror = () => reject(tx.error || new Error('indexedDB put failed'))
		})
		await evictIfNeeded(db)
	} catch {
		/* cache write is best-effort */
	}
}

const evictIfNeeded = (db) =>
	new Promise((resolve, reject) => {
		const tx = db.transaction(STORE, 'readonly')
		const req = tx.objectStore(STORE).getAll()
		req.onsuccess = async () => {
			try {
				const entries = req.result || []
				let total = entries.reduce((sum, e) => sum + (e.size || 0), 0)
				if (total <= PREVIEW_CACHE_LIMIT_BYTES) {
					resolve()
					return
				}
				entries.sort((a, b) => a.ts - b.ts)
				const evictTx = db.transaction(STORE, 'readwrite')
				const store = evictTx.objectStore(STORE)
				for (const entry of entries) {
					if (total <= PREVIEW_CACHE_LIMIT_BYTES) break
					store.delete(entry.key)
					total -= entry.size || 0
				}
				evictTx.oncomplete = () => resolve()
				evictTx.onerror = () => reject(evictTx.error || new Error('indexedDB evict failed'))
			} catch (e) {
				reject(e)
			}
		}
		req.onerror = () => reject(req.error || new Error('indexedDB getAll failed'))
	})
