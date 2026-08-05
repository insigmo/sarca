/**
 * Refcounted `blob:` object URLs, keyed by `scope:path`.
 *
 * WebKit paints the broken-image glyph the instant a mounted `<img>` points at
 * a revoked object URL. Solid effects that create/revoke their own URL on
 * every re-run (list refresh, prop churn, remount during virtualized scroll)
 * revoke a URL the still-mounted image is showing, and the tile breaks even
 * though the underlying blob was never actually gone. Routing every
 * create/revoke through this pool fixes that: a URL is only revoked once
 * nothing references it, and even then not immediately — a short grace
 * window lets an immediate remount (the common case for the bugs above)
 * reuse the same URL instead of paying for a new one.
 */

/** How long a URL survives after its last release, in case of a quick remount. */
const REVOKE_DELAY_MS = 5000

/** @typedef {{ url: string, refcount: number, revokeTimer: ReturnType<typeof setTimeout> | null }} PoolEntry */

/** @type {Map<string, PoolEntry>} */
const pool = new Map()

/**
 * Get (creating if necessary) the object URL for `key`, bumping its refcount.
 * A pending deferred revoke is cancelled, so the existing URL keeps working.
 *
 * Only the first caller for a key pays for `URL.createObjectURL`; later
 * callers for the same key reuse that URL even if they pass a different (but
 * presumably equivalent) `Blob` instance for the same logical resource.
 *
 * @param {string} key
 * @param {Blob} blob
 * @returns {string}
 */
export const acquireObjectUrl = (key, blob) => {
	const existing = pool.get(key)
	if (existing) {
		if (existing.revokeTimer != null) {
			clearTimeout(existing.revokeTimer)
			existing.revokeTimer = null
		}
		existing.refcount += 1
		return existing.url
	}
	const url = URL.createObjectURL(blob)
	pool.set(key, { url, refcount: 1, revokeTimer: null })
	return url
}

/**
 * Release one reference to `key`. Once the refcount reaches zero, the URL is
 * revoked after `REVOKE_DELAY_MS` rather than immediately, unless it is
 * re-acquired before the timer fires.
 * @param {string} key
 */
export const releaseObjectUrl = (key) => {
	const entry = pool.get(key)
	if (!entry) return
	entry.refcount -= 1
	if (entry.refcount > 0) return
	entry.revokeTimer = setTimeout(() => {
		URL.revokeObjectURL(entry.url)
		pool.delete(key)
	}, REVOKE_DELAY_MS)
}

/** Test seam: revoke everything and forget all entries. */
export const resetObjectUrlPool = () => {
	for (const entry of pool.values()) {
		if (entry.revokeTimer != null) clearTimeout(entry.revokeTimer)
		URL.revokeObjectURL(entry.url)
	}
	pool.clear()
}
