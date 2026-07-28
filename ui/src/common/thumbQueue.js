/**
 * Limit concurrent thumbnail HTTP fetches so opening a large photo folder
 * does not stampede the browser/WebView and Telegram download path.
 */

const MAX_CONCURRENT = 6

/** @type {Array<{ run: () => Promise<any>, resolve: (v: any) => void, reject: (e: any) => void, signal?: AbortSignal }>} */
const waiters = []
let active = 0

function pump() {
	while (active < MAX_CONCURRENT && waiters.length) {
		const entry = waiters.shift()
		if (!entry) break
		if (entry.signal?.aborted) {
			entry.reject(new DOMException('Aborted', 'AbortError'))
			continue
		}
		active += 1
		Promise.resolve()
			.then(() => entry.run())
			.then(entry.resolve, entry.reject)
			.finally(() => {
				active -= 1
				pump()
			})
	}
}

/**
 * @template T
 * @param {() => Promise<T>} run
 * @param {{ signal?: AbortSignal }} [opts]
 * @returns {Promise<T>}
 */
export function enqueueThumbFetch(run, opts = {}) {
	const signal = opts.signal
	if (signal?.aborted) {
		return Promise.reject(new DOMException('Aborted', 'AbortError'))
	}
	return new Promise((resolve, reject) => {
		/** @type {{ run: () => Promise<any>, resolve: (v: any) => void, reject: (e: any) => void, signal?: AbortSignal }} */
		const entry = { run, resolve, reject, signal }
		const onAbort = () => {
			const idx = waiters.indexOf(entry)
			if (idx >= 0) {
				waiters.splice(idx, 1)
				reject(new DOMException('Aborted', 'AbortError'))
			}
		}
		if (signal) {
			signal.addEventListener('abort', onAbort, { once: true })
		}
		waiters.push(entry)
		pump()
	})
}

/** @returns {{ active: number, waiting: number }} */
export function thumbQueueStats() {
	return { active, waiting: waiters.length }
}

export const THUMB_MAX_CONCURRENT = MAX_CONCURRENT
