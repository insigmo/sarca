/**
 * Photo zoom + gesture model shared by the web UI, the desktop client and the
 * mobile app. All of them render the same viewer, so the behaviour lives here
 * once: pinch/double-tap/drag on touch, wheel and the magnifier buttons with a
 * pointer, and the horizontal swipe that walks the folder on a phone.
 *
 * The transform the caller applies is `translate(x, y) scale(s)` around the
 * element's centre, so an offset is measured in screen pixels from that centre
 * and a content point `c` lands at `offset + c * scale`.
 */

export const ZOOM_MIN = 1
export const ZOOM_MAX = 8
/** One press of the magnifier buttons. */
export const ZOOM_BUTTON_FACTOR = 1.5
/** Where a double tap / double click lands when starting from 1x. */
export const DOUBLE_TAP_SCALE = 2.5
/** Wheel notch to scale factor (a notch is ~100 deltaY). */
export const WHEEL_FACTOR = 1.0015
/** A tap is a press that neither moved nor lasted. */
export const TAP_SLOP_PX = 12
export const TAP_MAX_MS = 350
export const DOUBLE_TAP_MS = 320
/** Horizontal swipe that steps to the neighbouring file. */
export const SWIPE_DISTANCE_PX = 56
/**
 * Below this a gesture is a sloppy tap, not a flick. Without it a 13px slip
 * over a few milliseconds clears the velocity gate and jumps a photo.
 */
export const SWIPE_MIN_PX = TAP_SLOP_PX * 2
export const SWIPE_VELOCITY_PX_PER_MS = 0.35
/** Vertical swipe that dismisses the viewer. */
export const DISMISS_DISTANCE_PX = 120

/**
 * @param {number} scale
 * @returns {number}
 */
export function clampScale(scale) {
	if (!Number.isFinite(scale)) return ZOOM_MIN
	return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, scale))
}

/**
 * How far the image may travel from the centre before its edge would leave the
 * viewport. Zero while the photo still fits, which is what keeps an unzoomed
 * one pinned. `content` is the painted size at 1x — for a letterboxed photo
 * that is smaller than the viewport, and using it stops a pan from dragging
 * the picture into the empty bars.
 * @param {number} scale
 * @param {{ width: number, height: number }} viewport
 * @param {{ width: number, height: number }} [content] Defaults to the viewport.
 * @returns {{ x: number, y: number }}
 */
export function panBounds(scale, viewport, content) {
	const s = clampScale(scale)
	const vw = Math.max(0, viewport?.width || 0)
	const vh = Math.max(0, viewport?.height || 0)
	const cw = Math.max(0, content?.width || vw)
	const ch = Math.max(0, content?.height || vh)
	return {
		x: Math.max(0, (s * cw - vw) / 2),
		y: Math.max(0, (s * ch - vh) / 2),
	}
}

/**
 * @param {{ x: number, y: number }} offset
 * @param {number} scale
 * @param {{ width: number, height: number }} viewport
 * @param {{ width: number, height: number }} [content]
 * @returns {{ x: number, y: number }}
 */
export function clampOffset(offset, scale, viewport, content) {
	const bounds = panBounds(scale, viewport, content)
	const x = Number.isFinite(offset?.x) ? offset.x : 0
	const y = Number.isFinite(offset?.y) ? offset.y : 0
	// `|| 0` also folds the -0 that clamping a negative offset to zero leaves
	// behind, so equal states compare equal.
	return {
		x: Math.min(bounds.x, Math.max(-bounds.x, x)) || 0,
		y: Math.min(bounds.y, Math.max(-bounds.y, y)) || 0,
	}
}

/**
 * Scale around a focal point so whatever sits under the fingers (or the
 * cursor) stays under them.
 * @param {{
 *   scale: number,
 *   offset: { x: number, y: number },
 *   nextScale: number,
 *   viewport: { width: number, height: number },
 *   content?: { width: number, height: number },
 *   focus?: { x: number, y: number } Screen offset from the element's centre.
 * }} args
 * @returns {{ scale: number, offset: { x: number, y: number } }}
 */
export function zoomTo({ scale, offset, nextScale, viewport, content, focus }) {
	const from = clampScale(scale)
	const to = clampScale(nextScale)
	const fx = Number.isFinite(focus?.x) ? focus.x : 0
	const fy = Number.isFinite(focus?.y) ? focus.y : 0
	const ratio = to / from
	const next = {
		x: fx - (fx - (offset?.x || 0)) * ratio,
		y: fy - (fy - (offset?.y || 0)) * ratio,
	}
	return { scale: to, offset: clampOffset(next, to, viewport, content) }
}

/**
 * What a finished one-finger gesture meant. Only ever a swipe while the photo
 * is unzoomed — once it is zoomed the same drag is a pan.
 * @param {{ dx: number, dy: number, elapsedMs: number, scale: number }} args
 * @returns {'prev' | 'next' | 'close' | null}
 */
export function swipeIntent({ dx, dy, elapsedMs, scale }) {
	if (clampScale(scale) > ZOOM_MIN) return null
	const ms = Math.max(1, elapsedMs || 0)
	const horizontal = Math.abs(dx) > Math.abs(dy)
	if (horizontal) {
		if (Math.abs(dx) < SWIPE_MIN_PX) return null
		const far = Math.abs(dx) >= SWIPE_DISTANCE_PX
		const fast = Math.abs(dx) / ms >= SWIPE_VELOCITY_PX_PER_MS
		if (!far && !fast) return null
		return dx < 0 ? 'next' : 'prev'
	}
	if (dy >= DISMISS_DISTANCE_PX) return 'close'
	return null
}

/**
 * The zoom state machine plus its pointer plumbing.
 *
 * @param {{
 *   onChange: (state: { scale: number, offset: { x: number, y: number } }) => void,
 *   onSwipe?: (direction: 'prev' | 'next') => void,
 *   onDismiss?: () => void,
 *   onTap?: (e: PointerEvent) => void,
 *   isEnabled?: () => boolean,
 *   getContentSize?: () => { width: number, height: number } | null,
 * }} opts
 */
export function createImageZoom(opts) {
	let scale = ZOOM_MIN
	let offset = { x: 0, y: 0 }
	/** @type {HTMLElement | null} */
	let host = null

	/** @type {Map<number, { x: number, y: number }>} */
	const pointers = new Map()
	let dragging = false
	let dragOrigin = { x: 0, y: 0 }
	let dragOffset = { x: 0, y: 0 }
	let pressStart = { x: 0, y: 0, t: 0 }
	let moved = false
	let pinching = false
	let pinchStartDist = 0
	let pinchStartScale = ZOOM_MIN
	let lastTap = { x: 0, y: 0, t: 0 }
	/** A tap is only reported once it is certain it was not half of a double. */
	let tapTimer = null

	const enabled = () => (opts.isEnabled ? opts.isEnabled() : true)

	const viewport = () => {
		if (!host) return { width: 0, height: 0 }
		const rect = host.getBoundingClientRect()
		return { width: rect.width, height: rect.height }
	}

	const content = () => opts.getContentSize?.() || viewport()

	/** Screen offset from the element's centre. */
	const focusOf = (clientX, clientY) => {
		if (!host) return { x: 0, y: 0 }
		const rect = host.getBoundingClientRect()
		return {
			x: clientX - (rect.left + rect.width / 2),
			y: clientY - (rect.top + rect.height / 2),
		}
	}

	const emit = () => {
		opts.onChange({ scale, offset: { ...offset } })
	}

	const apply = (next) => {
		const changed = next.scale !== scale || next.offset.x !== offset.x || next.offset.y !== offset.y
		scale = next.scale
		offset = next.offset
		if (changed) emit()
	}

	/**
	 * @param {number} nextScale
	 * @param {{ x: number, y: number }} [focus] Screen offset from the centre.
	 */
	const zoomToScale = (nextScale, focus) => {
		apply(
			zoomTo({
				scale,
				offset,
				nextScale,
				viewport: viewport(),
				content: content(),
				focus,
			}),
		)
	}

	const reset = () => {
		apply({ scale: ZOOM_MIN, offset: { x: 0, y: 0 } })
	}

	const centreOfPointers = () => {
		let sx = 0
		let sy = 0
		for (const p of pointers.values()) {
			sx += p.x
			sy += p.y
		}
		const n = pointers.size || 1
		return { x: sx / n, y: sy / n }
	}

	const distanceOfPointers = () => {
		const [a, b] = [...pointers.values()]
		if (!a || !b) return 0
		return Math.hypot(a.x - b.x, a.y - b.y)
	}

	const beginDrag = (x, y) => {
		dragging = scale > ZOOM_MIN
		dragOrigin = { x, y }
		dragOffset = { ...offset }
	}

	/**
	 * Re-anchor the pinch to whichever two pointers are down now. A third finger
	 * (a palm) landing, or one of the first two leaving, changes which pair
	 * `distanceOfPointers` measures, and without a fresh anchor the next move
	 * compares two different pairs and the photo jumps.
	 */
	const beginPinch = () => {
		pinching = true
		dragging = false
		pinchStartDist = distanceOfPointers()
		pinchStartScale = scale
	}

	const cancelPendingTap = () => {
		if (tapTimer === null) return
		clearTimeout(tapTimer)
		tapTimer = null
	}

	const onPointerDown = (e) => {
		if (!enabled()) return
		if (e.pointerType === 'mouse' && e.button !== 0) return
		pointers.set(e.pointerId, { x: e.clientX, y: e.clientY })
		try {
			host?.setPointerCapture?.(e.pointerId)
		} catch {
			/* capture is an optimisation; the window-level listeners still fire */
		}
		if (pointers.size === 1) {
			moved = false
			pressStart = { x: e.clientX, y: e.clientY, t: Date.now() }
			beginDrag(e.clientX, e.clientY)
			return
		}
		beginPinch()
	}

	const onPointerMove = (e) => {
		if (!pointers.has(e.pointerId)) return
		pointers.set(e.pointerId, { x: e.clientX, y: e.clientY })

		if (pinching && pointers.size >= 2) {
			const dist = distanceOfPointers()
			if (pinchStartDist > 0 && dist > 0) {
				const mid = centreOfPointers()
				zoomToScale(
					pinchStartScale * (dist / pinchStartDist),
					focusOf(mid.x, mid.y),
				)
			}
			if (e.cancelable) e.preventDefault()
			return
		}

		const dx = e.clientX - pressStart.x
		const dy = e.clientY - pressStart.y
		if (Math.abs(dx) > TAP_SLOP_PX || Math.abs(dy) > TAP_SLOP_PX) moved = true

		if (!dragging) return
		apply({
			scale,
			offset: clampOffset(
				{
					x: dragOffset.x + (e.clientX - dragOrigin.x),
					y: dragOffset.y + (e.clientY - dragOrigin.y),
				},
				scale,
				viewport(),
				content(),
			),
		})
		if (e.cancelable) e.preventDefault()
	}

	const onPointerUp = (e) => {
		if (!pointers.has(e.pointerId)) return
		// A mouse button released mid-drag is not the end of the drag; only the
		// button that started it is.
		if (e.pointerType === 'mouse' && e.button !== 0) return
		pointers.delete(e.pointerId)
		try {
			host?.releasePointerCapture?.(e.pointerId)
		} catch {
			/* nothing to release */
		}

		if (pinching) {
			// The last finger of a pinch must not become a pan that jumps: start
			// a fresh drag from wherever it now is.
			if (pointers.size === 1) {
				const [remaining] = [...pointers.values()]
				pressStart = { x: remaining.x, y: remaining.y, t: Date.now() }
				moved = true
				beginDrag(remaining.x, remaining.y)
			} else if (pointers.size === 0) {
				pinching = false
				dragging = false
				// Pinching back below 1x snaps home rather than leaving a gap.
				if (scale <= ZOOM_MIN) reset()
			} else {
				// Still two or more fingers down: carry on from the pair that is
				// left instead of the pair we started with.
				beginPinch()
			}
			return
		}

		const wasDragging = dragging
		dragging = false
		const dx = e.clientX - pressStart.x
		const dy = e.clientY - pressStart.y
		const elapsed = Date.now() - pressStart.t

		// A click is a click however long the button was held; a long press with
		// a finger is the platform's own gesture (save image, context menu) and
		// must not also count as a tap.
		const isTap = !moved && (e.pointerType === 'mouse' || elapsed <= TAP_MAX_MS)
		if (isTap) {
			const now = Date.now()
			const isDouble =
				now - lastTap.t <= DOUBLE_TAP_MS &&
				Math.abs(e.clientX - lastTap.x) <= TAP_SLOP_PX * 2 &&
				Math.abs(e.clientY - lastTap.y) <= TAP_SLOP_PX * 2
			if (isDouble) {
				cancelPendingTap()
				lastTap = { x: 0, y: 0, t: 0 }
				if (scale > ZOOM_MIN) reset()
				else zoomToScale(DOUBLE_TAP_SCALE, focusOf(e.clientX, e.clientY))
				return
			}
			lastTap = { x: e.clientX, y: e.clientY, t: now }
			// Reporting the tap right away would fire on the first half of every
			// double tap, so whatever it triggers (closing the viewer) would beat
			// the zoom. Hold it until a second tap can no longer arrive.
			if (opts.onTap) {
				const at = { clientX: e.clientX, clientY: e.clientY }
				cancelPendingTap()
				tapTimer = setTimeout(() => {
					tapTimer = null
					opts.onTap?.(at)
				}, DOUBLE_TAP_MS)
			}
			return
		}

		if (wasDragging) return

		const intent = swipeIntent({ dx, dy, elapsedMs: elapsed, scale })
		if (intent === 'close') opts.onDismiss?.()
		else if (intent) opts.onSwipe?.(intent)
	}

	const onPointerCancel = (e) => {
		if (!pointers.delete(e.pointerId)) return
		if (pointers.size === 0) {
			pinching = false
			dragging = false
		} else if (pinching) {
			beginPinch()
		}
	}

	/**
	 * Capture can be lost without a `pointerup` — the browser steals it, or the
	 * button comes up outside the window. Treat it as the gesture ending, or the
	 * photo keeps following a cursor with nothing held down.
	 */
	const onLostCapture = (e) => {
		if (!pointers.has(e.pointerId)) return
		onPointerCancel(e)
	}

	const onWheel = (e) => {
		if (!enabled()) return
		// A trackpad pinch arrives as ctrl+wheel. It has to be swallowed even
		// when the photo is already at the end of its range, or the browser
		// takes it as page zoom and the whole UI grows.
		if (e.ctrlKey && e.cancelable) e.preventDefault()
		// deltaMode is lines (1) on Firefox and pages (2) in the rare case;
		// without normalising, a Firefox notch is 3 instead of ~100 and the
		// wheel does nothing visible.
		const perLine = 16
		const unit =
			e.deltaMode === 1 ? perLine : e.deltaMode === 2 ? viewport().height || 600 : 1
		const next = scale * WHEEL_FACTOR ** (-e.deltaY * unit)
		if (clampScale(next) === scale) return
		if (e.cancelable) e.preventDefault()
		zoomToScale(next, focusOf(e.clientX, e.clientY))
	}

	return {
		/** @param {HTMLElement} el */
		attach(el) {
			host = el
			el.addEventListener('pointerdown', onPointerDown)
			el.addEventListener('pointermove', onPointerMove, { passive: false })
			el.addEventListener('pointerup', onPointerUp)
			el.addEventListener('pointercancel', onPointerCancel)
			el.addEventListener('lostpointercapture', onLostCapture)
			el.addEventListener('wheel', onWheel, { passive: false })
			// The finger can leave the surface, and a mouse button can come up
			// outside the window entirely. Both handlers ignore pointers they are
			// not tracking, so the duplicate delivery from bubbling is harmless.
			const view = el.ownerDocument?.defaultView
			view?.addEventListener('pointerup', onPointerUp)
			view?.addEventListener('pointercancel', onPointerCancel)
			return () => {
				el.removeEventListener('pointerdown', onPointerDown)
				el.removeEventListener('pointermove', onPointerMove)
				el.removeEventListener('pointerup', onPointerUp)
				el.removeEventListener('pointercancel', onPointerCancel)
				el.removeEventListener('lostpointercapture', onLostCapture)
				el.removeEventListener('wheel', onWheel)
				view?.removeEventListener('pointerup', onPointerUp)
				view?.removeEventListener('pointercancel', onPointerCancel)
				cancelPendingTap()
				pointers.clear()
				pinching = false
				dragging = false
				host = null
			}
		},
		zoomIn() {
			zoomToScale(scale * ZOOM_BUTTON_FACTOR)
		},
		zoomOut() {
			zoomToScale(scale / ZOOM_BUTTON_FACTOR)
		},
		zoomToScale,
		/**
		 * Nudge a zoomed photo, used by the arrow keys.
		 * @param {number} dx
		 * @param {number} dy
		 */
		panBy(dx, dy) {
			apply({
				scale,
				offset: clampOffset(
					{ x: offset.x + dx, y: offset.y + dy },
					scale,
					viewport(),
					content(),
				),
			})
		},
		reset,
		get scale() {
			return scale
		},
		get offset() {
			return { ...offset }
		},
	}
}
