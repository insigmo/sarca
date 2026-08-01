/**
 * Best-effort snapshot of the last-seen file list per storage/path, so a full
 * page reload can repaint instantly instead of flashing an empty folder while
 * the fresh listing round-trips.
 */

const PREFIX = 'sarca.fsLayerCache.'
const MAX_ITEMS = 1000

/**
 * @param {string} storageId
 * @param {string} path
 * @returns {string}
 */
export const fsLayerCacheKey = (storageId, path) => `${PREFIX}${storageId}::${path || ''}`

/**
 * @param {string} key
 * @returns {import("../api").FSElement[] | null}
 */
export const readFsLayerCache = (key) => {
	try {
		const raw = localStorage.getItem(key)
		if (!raw) return null
		const parsed = JSON.parse(raw)
		return Array.isArray(parsed) ? parsed : null
	} catch {
		return null
	}
}

/**
 * @param {string} key
 * @param {import("../api").FSElement[]} items
 */
export const writeFsLayerCache = (key, items) => {
	try {
		localStorage.setItem(key, JSON.stringify((items || []).slice(0, MAX_ITEMS)))
	} catch {
		/* quota exceeded or storage unavailable — cache is best-effort */
	}
}
