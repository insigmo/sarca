/**
 * Limit concurrent thumbnail HTTP fetches so opening a large photo folder
 * does not stampede the browser/WebView and Telegram download path.
 *
 * In-flight work is abortable: folder navigation must call {@link clearThumbQueue}
 * so listing requests are not stuck behind thumbs (browser ~6 connections/origin).
 */

const MAX_CONCURRENT = 6

/** @type {Array<{ run: (signal: AbortSignal) => Promise<any>, resolve: (v: any) => void, reject: (e: any) => void, ac: AbortController }>} */
const waiters = []
/** @type {Set<AbortController>} */
const activeControllers = new Set()
let active = 0

function abortError() {
	return new DOMException('Aborted', 'AbortError')
}

function pump() {
	while (active < MAX_CONCURRENT && waiters.length) {
		const entry = waiters.shift()
		if (!entry) break
		if (entry.ac.signal.aborted) {
			entry.reject(abortError())
			continue
		}
		active += 1
		activeControllers.add(entry.ac)
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
			.then(entry.resolve, entry.reject)
			.finally(() => {
				activeControllers.delete(entry.ac)
				active -= 1
				pump()
			})
	}
}

/**
 * Drop queued thumbs and abort in-flight ones so navigation/list fetches
 * are not blocked on the shared HTTP connection pool.
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
		entry.reject(err)
	}
	for (const ac of [...activeControllers]) {
		try {
			ac.abort()
		} catch {
			/* ignore */
		}
	}
	activeControllers.clear()
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
		/** @type {{ run: (signal: AbortSignal) => Promise<any>, resolve: (v: any) => void, reject: (e: any) => void, ac: AbortController }} */
		const entry = { run, resolve, reject, ac }
		const onAbort = () => {
			const idx = waiters.indexOf(entry)
			if (idx >= 0) {
				waiters.splice(idx, 1)
				reject(abortError())
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
