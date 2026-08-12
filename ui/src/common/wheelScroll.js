/**
 * Mouse-wheel scrolling repair for the Linux (WebKitGTK) build.
 *
 * WebKitGTK reports wheel deltas in lines (`deltaMode === DOM_DELTA_LINE`) or
 * pages, not pixels, and then scrolls by its own idea of what a line is. The
 * result inside the file list is the jerky, either-barely-moves-or-jumps-a-
 * screenful behaviour that does not happen on the other platforms, where the
 * same wheel produces pixel deltas.
 *
 * The fix is to take those non-pixel events over: convert the delta to pixels
 * ourselves and set `scrollTop` directly. Pixel-mode events (every other
 * platform, and touchpads on Linux) are left completely alone.
 */

/** Pixels per wheel "line". Matches the step GTK apps use for a 3-line notch. */
export const LINE_HEIGHT_PX = 40
/** Pixels per wheel "page" event, resolved against the viewport at call time. */
export const PAGE_FRACTION = 0.9
/** Below this gap (ms) between wheel ticks, the user is spinning the wheel fast. */
export const ACCEL_WINDOW_MS = 120
/** Extra multiplier added per fast tick in a row. */
export const ACCEL_STEP = 0.35
/** Multiplier ceiling so a long flick doesn't fling scrollTop across the whole list. */
export const ACCEL_MAX = 3.5

/**
 * Convert a wheel event's delta to pixels.
 * @param {{ deltaY: number, deltaMode: number }} event
 * @param {number} viewportHeight
 * @param {number} [accel] Extra speed multiplier for a fast run of ticks.
 * @returns {number}
 */
export function deltaToPixels(event, viewportHeight, accel = 1) {
	switch (event.deltaMode) {
		case 1: // DOM_DELTA_LINE
			return event.deltaY * LINE_HEIGHT_PX * accel
		case 2: // DOM_DELTA_PAGE
			return event.deltaY * viewportHeight * PAGE_FRACTION
		default:
			return event.deltaY
	}
}

/**
 * Nearest ancestor (including `start`) that can actually scroll vertically.
 * @param {Element | null} start
 * @param {Element} boundary
 * @returns {Element | null}
 */
export function scrollableAncestor(start, boundary) {
	let node = start
	while (node && node !== boundary.parentElement) {
		if (node.scrollHeight > node.clientHeight) {
			const overflowY = getComputedStyle(node).overflowY
			if (overflowY === 'auto' || overflowY === 'scroll') return node
		}
		node = node.parentElement
	}
	return null
}

/**
 * True for the WebKitGTK build, the only one that needs this.
 * @param {string} [ua]
 * @returns {boolean}
 */
export function isLinuxWebKit(ua = typeof navigator === 'undefined' ? '' : navigator.userAgent) {
	return /Linux/i.test(ua) && !/Android/i.test(ua)
}

/**
 * Install the wheel handler on `root`.
 * @param {Element} [root]
 * @returns {() => void} uninstall
 */
export function installWheelScrollFix(root) {
	const target = root || (typeof document === 'undefined' ? null : document.documentElement)
	if (!target || !isLinuxWebKit()) return () => {}

	let lastTickAt = 0
	let accel = 1

	/** @param {WheelEvent} event */
	const onWheel = (event) => {
		// Pixel deltas already behave; touchpads on Linux report pixels too and
		// taking those over would kill their smoothness.
		if (event.deltaMode === 0 || event.ctrlKey || event.defaultPrevented) return

		const scroller = scrollableAncestor(
			/** @type {Element | null} */ (event.target),
			target,
		)
		if (!scroller) return

		// Spinning the wheel fast (in either direction) ramps the speed up so
		// flicking back and forth through a long list doesn't feel glued down;
		// a pause resets it back to the normal per-notch step.
		const now = Date.now()
		accel = now - lastTickAt < ACCEL_WINDOW_MS ? Math.min(ACCEL_MAX, accel + ACCEL_STEP) : 1
		lastTickAt = now

		const pixels = deltaToPixels(event, scroller.clientHeight, accel)
		const before = scroller.scrollTop
		const max = scroller.scrollHeight - scroller.clientHeight
		const next = Math.max(0, Math.min(max, before + pixels))
		if (next === before) return // at an edge: let the event chain as usual

		scroller.scrollTop = next
		event.preventDefault()
	}

	target.addEventListener('wheel', onWheel, { passive: false })
	return () => target.removeEventListener('wheel', onWheel)
}
