/**
 * IndexedDB-backed cache for photo previews and grid thumbnails on the web UI.
 *
 * Mirrors the native client's on-disk preview cache (see
 * client/src-tauri/src/commands.rs: cache_get_preview/cache_put_preview):
 * content-addressed by (scope, path) rather than by request URL, since the
 * preview URL embeds a short-lived access token and would otherwise defeat
 * plain HTTP caching. Size-capped with LRU eviction by last-access time.
 *
 * Thumbnails live in their own store with a much smaller budget: they are
 * ~128px JPEGs, so thousands fit in the space a handful of previews take, and
 * keeping them separate stops one big photo folder from evicting the thumbs
 * that make folder repaints instant.
 *
 * localStorage cannot hold this much binary data (most browsers cap it at
 * 5-10MB total per origin) so this uses IndexedDB instead, which supports a
 * much larger quota.
 */

const DB_NAME = 'sarca-preview-cache'
const STORE = 'previews'
const THUMB_STORE = 'thumbs'
const DB_VERSION = 2
export const PREVIEW_CACHE_LIMIT_BYTES = 100 * 1024 * 1024
export const THUMB_CACHE_LIMIT_BYTES = 32 * 1024 * 1024

/** @type {Promise<IDBDatabase> | null} */
let dbPromise = null

const openDb = () => {
	if (typeof indexedDB === 'undefined') return Promise.reject(new Error('indexedDB unavailable'))
	if (dbPromise) return dbPromise
	dbPromise = new Promise((resolve, reject) => {
		const req = indexedDB.open(DB_NAME, DB_VERSION)
		req.onupgradeneeded = () => {
			const db = req.result
			for (const name of [STORE, THUMB_STORE]) {
				if (db.objectStoreNames.contains(name)) continue
				const store = db.createObjectStore(name, { keyPath: 'key' })
				store.createIndex('ts', 'ts')
			}
		}
		req.onsuccess = () => resolve(req.result)
		req.onerror = () => reject(req.error || new Error('indexedDB open failed'))
	})
	return dbPromise
}

// Bumped whenever the server's preview encode format changes, so a stale
// blob from before the change is never served from IndexedDB. The server
// invalidates its own disk cache and stored preview documents on the same
// kind of change (see PREVIEW_FORMAT_VERSION); this does the client half.
const PREVIEW_FORMAT_VERSION = 'v2-full-res-q95'

const cacheKey = (scope, path) => `${PREVIEW_FORMAT_VERSION}:${scope}:${path}`

/**
 * @param {string} store
 * @param {string} scope
 * @param {string} path
 * @returns {Promise<Blob | null>}
 */
const readEntry = async (store, scope, path) => {
	try {
		const db = await openDb()
		const entry = await new Promise((resolve, reject) => {
			const tx = db.transaction(store, 'readonly')
			const req = tx.objectStore(store).get(cacheKey(scope, path))
			req.onsuccess = () => resolve(req.result || null)
			req.onerror = () => reject(req.error || new Error('indexedDB get failed'))
		})
		if (!entry) return null
		// Touch last-access time for LRU (best-effort, don't block the read on it).
		touchEntry(db, store, entry).catch(() => {})
		return entry.blob
	} catch {
		return null
	}
}

const touchEntry = (db, store, entry) =>
	new Promise((resolve, reject) => {
		const tx = db.transaction(store, 'readwrite')
		tx.objectStore(store).put({ ...entry, ts: Date.now() })
		tx.oncomplete = () => resolve()
		tx.onerror = () => reject(tx.error || new Error('indexedDB touch failed'))
	})

/**
 * @param {string} store
 * @param {number} limit
 * @param {string} scope
 * @param {string} path
 * @param {Blob} blob
 * @returns {Promise<void>}
 */
const writeEntry = async (store, limit, scope, path, blob) => {
	try {
		const db = await openDb()
		await new Promise((resolve, reject) => {
			const tx = db.transaction(store, 'readwrite')
			tx.objectStore(store).put({
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
		await evictIfNeeded(db, store, limit)
	} catch {
		/* cache write is best-effort */
	}
}

/**
 * @param {string} scope
 * @param {string} path
 * @returns {Promise<Blob | null>}
 */
export const getCachedPreview = (scope, path) => readEntry(STORE, scope, path)

/**
 * @param {string} scope
 * @param {string} path
 * @param {Blob} blob
 * @returns {Promise<void>}
 */
export const putCachedPreview = (scope, path, blob) =>
	writeEntry(STORE, PREVIEW_CACHE_LIMIT_BYTES, scope, path, blob)

/**
 * @param {string} scope
 * @param {string} path
 * @returns {Promise<Blob | null>}
 */
export const getCachedThumb = (scope, path) => readEntry(THUMB_STORE, scope, path)

/**
 * @param {string} scope
 * @param {string} path
 * @param {Blob} blob
 * @returns {Promise<void>}
 */
export const putCachedThumb = (scope, path, blob) =>
	writeEntry(THUMB_STORE, THUMB_CACHE_LIMIT_BYTES, scope, path, blob)

const evictIfNeeded = (db, store, limit) =>
	new Promise((resolve, reject) => {
		const tx = db.transaction(store, 'readonly')
		const req = tx.objectStore(store).getAll()
		req.onsuccess = async () => {
			try {
				const entries = req.result || []
				let total = entries.reduce((sum, e) => sum + (e.size || 0), 0)
				if (total <= limit) {
					resolve()
					return
				}
				entries.sort((a, b) => a.ts - b.ts)
				const evictTx = db.transaction(store, 'readwrite')
				const objectStore = evictTx.objectStore(store)
				for (const entry of entries) {
					if (total <= limit) break
					objectStore.delete(entry.key)
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
