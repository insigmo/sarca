import { shouldRefreshOnVisibilityEvent } from './visibilityRefresh'

/** Inclusive lower bound of the gap between two silent refreshes. */
export const MIN_DELAY_MS = 5_000
/** Inclusive upper bound of the gap between two silent refreshes. */
export const MAX_DELAY_MS = 15_000

/**
 * A fresh delay for every tick rather than one fixed interval.
 *
 * Jitter matters here: every open tab and every running client polls the same
 * endpoints, and a fixed cadence lines them all up into a burst the server has
 * to absorb at once.
 * @returns {number}
 */
export const nextDelay = () =>
	MIN_DELAY_MS + Math.floor(Math.random() * (MAX_DELAY_MS - MIN_DELAY_MS + 1))

/**
 * Re-run `run` every 5-15 seconds for as long as the page is actually being
 * looked at.
 *
 * Silent by contract: `run` must update state in place and must not raise a
 * loading flag, or the list flickers every few seconds. Ticks never overlap and
 * never queue up — a slow or failed run just costs its own tick.
 *
 * @param {{
 *   run: () => unknown | Promise<unknown>,
 *   isPaused?: () => boolean,
 *   doc?: Document,
 *   win?: Window,
 * }} options
 * @returns {() => void} stop function; safe to call more than once
 */
export function startAutoRefresh(options) {
	const { run } = options
	const isPaused = options.isPaused || (() => false)
	const doc = options.doc || (typeof document === 'undefined' ? null : document)
	const win = options.win || (typeof window === 'undefined' ? null : window)
	if (!doc || !win) return () => {}

	let timer = null
	let running = false
	let stopped = false

	const visible = () => shouldRefreshOnVisibilityEvent(doc.visibilityState)

	const clear = () => {
		if (timer !== null) {
			win.clearTimeout(timer)
			timer = null
		}
	}

	const schedule = () => {
		if (stopped || !visible()) return
		clear()
		timer = win.setTimeout(tick, nextDelay())
	}

	async function tick() {
		timer = null
		if (stopped) return
		// A tick that arrives while the previous run is still in flight is
		// dropped, not queued: stacking them turns one slow response into a
		// pile-up of identical requests.
		if (running || !visible() || isPaused()) {
			schedule()
			return
		}
		running = true
		try {
			await run()
		} catch {
			// Background refresh failures stay silent; the next tick retries and
			// the user's own actions still surface their own errors.
		} finally {
			running = false
			schedule()
		}
	}

	const onVisibility = () => {
		if (visible()) {
			// Coming back into view restarts the cadence rather than firing at
			// once, so alt-tabbing repeatedly cannot be used as a request pump.
			schedule()
		} else {
			clear()
		}
	}

	doc.addEventListener('visibilitychange', onVisibility)
	win.addEventListener('focus', onVisibility)
	schedule()

	return () => {
		if (stopped) return
		stopped = true
		clear()
		doc.removeEventListener('visibilitychange', onVisibility)
		win.removeEventListener('focus', onVisibility)
	}
}
