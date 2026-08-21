/**
 * Limit concurrent thumbnail HTTP fetches so opening a large photo folder
 * does not stampede the browser/WebView and Telegram download path.
 *
 * In-flight work is abortable: folder navigation must call {@link clearThumbQueue}
 * so listing requests are not stuck behind thumbs (browser ~6 connections/origin).
 */

const MAX_CONCURRENT = 6

// A 503 means the storage's Telegram rate-limit bucket is temporarily
// empty, not that the thumb is missing. Back off with jittered exponential
// delay and retry before giving up, so a saturated storage (e.g. a big
// folder draining TELEGRAM_RATE_LIMIT) doesn't poison every tile after the
// budget runs out with a permanent failure.
const RETRY_BASE_MS = 1000
const RETRY_CAP_MS = 30000
const RETRY_MAX_ATTEMPTS = 8

/**
 * @typedef {Object} ThumbQueueEntry
 * @property {(signal: AbortSignal) => Promise<any>} run
 * @property {(v: any) => void} resolve
 * @property {(e: any) => void} reject
 * @property {AbortController} ac
 * @property {number} attempt 0-based count of failed tries so far.
 */

/** @type {Array<ThumbQueueEntry>} */
const waiters = []
/**
 * Every entry not yet finally settled — queued, actively fetching, *or*
 * sleeping between retry attempts. Kept populated across retries (not just
 * while a fetch is in flight) so {@link clearThumbQueue} can still reach and
 * abort a tile that is currently backing off.
 * @type {Set<ThumbQueueEntry>}
 */
const liveEntries = new Set()
let active = 0

function abortError() {
	return new DOMException('Aborted', 'AbortError')
}

function delay(ms, signal) {
	return new Promise((resolve, reject) => {
		if (signal.aborted) {
			reject(abortError())
			return
		}
		const onAbort = () => {
			clearTimeout(timer)
			reject(abortError())
		}
		const timer = setTimeout(() => {
			signal.removeEventListener('abort', onAbort)
			resolve()
		}, ms)
		signal.addEventListener('abort', onAbort, { once: true })
	})
}

/**
 * Exponential backoff (base ~1s, capped ~30s) with a little added jitter so
 * every tile thrown out by the same 503 doesn't retry in the same instant
 * and immediately re-trip the storage's rate limit. A server `Retry-After`
 * (seconds) overrides the guess entirely — it knows exactly when its
 * Telegram token bucket refills — jitter still added on top, never off, so
 * we never retry *before* the server asked us to.
 * @param {number} attempt 0-based
 * @param {number | undefined} retryAfterSeconds
 */
function backoffDelayMs(attempt, retryAfterSeconds) {
	const base =
		Number.isFinite(retryAfterSeconds) && retryAfterSeconds > 0
			? retryAfterSeconds * 1000
			: Math.min(RETRY_CAP_MS, RETRY_BASE_MS * 2 ** attempt)
	return base + Math.random() * base * 0.3
}

/** @param {ThumbQueueEntry} entry */
function settle(entry, fn, value) {
	liveEntries.delete(entry)
	fn(value)
}

function pump() {
	while (active < MAX_CONCURRENT && waiters.length) {
		const entry = waiters.shift()
		if (!entry) break
		if (entry.ac.signal.aborted) {
			settle(entry, entry.reject, abortError())
			continue
		}
		active += 1
		runAttempt(entry)
	}
}

/** @param {ThumbQueueEntry} entry */
function runAttempt(entry) {
	const signal = entry.ac.signal
	Promise.resolve()
		.then(
			() =>
				new Promise((resolve, reject) => {
					if (signal.aborted) {
						reject(abortError())
						return
					}
					const onAbort = () => reject(abortError())
					signal.addEventListener('abort', onAbort, { once: true })
					Promise.resolve()
						.then(() => entry.run(signal))
						.then(
							(v) => {
								signal.removeEventListener('abort', onAbort)
								resolve(v)
							},
							(e) => {
								signal.removeEventListener('abort', onAbort)
								reject(e)
							},
						)
				}),
		)
		.then(
			(v) => {
				active -= 1
				pump()
				settle(entry, entry.resolve, v)
			},
			(e) => {
				active -= 1
				// Release the slot *before* deciding whether to retry — a tile
				// backing off from a 503 must not sit on one of the six
				// concurrent slots while it sleeps, starving everything else
				// still queued behind it.
				pump()
				onAttemptFailed(entry, e)
			},
		)
}

/**
 * @param {ThumbQueueEntry} entry
 * @param {any} e
 */
function onAttemptFailed(entry, e) {
	if (entry.ac.signal.aborted || e?.name === 'AbortError') {
		settle(entry, entry.reject, e)
		return
	}
	if (e?.status !== 503 || entry.attempt >= RETRY_MAX_ATTEMPTS - 1) {
		settle(entry, entry.reject, e)
		return
	}
	const ms = backoffDelayMs(entry.attempt, e?.retryAfter)
	entry.attempt += 1
	delay(ms, entry.ac.signal).then(
		() => {
			// Re-enter at the back of the queue for a fresh slot, fair with
			// everything else waiting rather than jumping the line.
			waiters.push(entry)
			pump()
		},
		(abortErr) => settle(entry, entry.reject, abortErr),
	)
}

/**
 * Drop queued thumbs and abort in-flight (or currently backing-off) ones so
 * navigation/list fetches are not blocked on the shared HTTP connection pool.
 */
export function clearThumbQueue() {
	const err = abortError()
	while (waiters.length) {
		const entry = waiters.shift()
		try {
			entry.ac.abort()
		} catch {
			/* ignore */
		}
		settle(entry, entry.reject, err)
	}
	for (const entry of [...liveEntries]) {
		try {
			entry.ac.abort()
		} catch {
			/* ignore */
		}
	}
}

/**
 * @template T
 * @param {(signal: AbortSignal) => Promise<T>} run
 * @param {{ signal?: AbortSignal }} [opts]
 * @returns {Promise<T>}
 */
export function enqueueThumbFetch(run, opts = {}) {
	const parent = opts.signal
	if (parent?.aborted) {
		return Promise.reject(abortError())
	}
	const ac = new AbortController()
	if (parent) {
		parent.addEventListener(
			'abort',
			() => {
				try {
					ac.abort()
				} catch {
					/* ignore */
				}
			},
			{ once: true },
		)
	}
	return new Promise((resolve, reject) => {
		/** @type {ThumbQueueEntry} */
		const entry = { run, resolve, reject, ac, attempt: 0 }
		liveEntries.add(entry)
		const onAbort = () => {
			const idx = waiters.indexOf(entry)
			if (idx >= 0) {
				waiters.splice(idx, 1)
				settle(entry, reject, abortError())
			}
		}
		ac.signal.addEventListener('abort', onAbort, { once: true })
		waiters.push(entry)
		pump()
	})
}

/** @returns {{ active: number, waiting: number }} */
export function thumbQueueStats() {
	return { active, waiting: waiters.length }
}

export const THUMB_MAX_CONCURRENT = MAX_CONCURRENT
