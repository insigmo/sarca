import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

import {
	DISMISS_DISTANCE_PX,
	DOUBLE_TAP_MS,
	DOUBLE_TAP_SCALE,
	SWIPE_DISTANCE_PX,
	SWIPE_MIN_PX,
	ZOOM_BUTTON_FACTOR,
	ZOOM_MAX,
	ZOOM_MIN,
	clampOffset,
	clampScale,
	createImageZoom,
	panBounds,
	swipeIntent,
	zoomTo,
} from './imageZoom'

const VIEWPORT = { width: 800, height: 600 }

/** A stand-in for the gesture surface: jsdom has no layout. */
const makeHost = (rect = { left: 0, top: 0, width: 800, height: 600 }) => {
	const el = document.createElement('div')
	el.getBoundingClientRect = () => ({
		left: rect.left,
		top: rect.top,
		right: rect.left + rect.width,
		bottom: rect.top + rect.height,
		width: rect.width,
		height: rect.height,
		x: rect.left,
		y: rect.top,
		toJSON: () => ({}),
	})
	el.setPointerCapture = () => {}
	el.releasePointerCapture = () => {}
	document.body.appendChild(el)
	return el
}

/** jsdom has no PointerEvent, so carry the fields the model reads. */
const pointer = (type, { id = 1, x = 0, y = 0, pointerType = 'touch' } = {}) => {
	const e = new Event(type, { bubbles: true, cancelable: true })
	Object.assign(e, { pointerId: id, clientX: x, clientY: y, pointerType, button: 0 })
	return e
}

const wheel = ({ x = 0, y = 0, deltaY = -100, deltaMode = 0 } = {}) => {
	const e = new Event('wheel', { bubbles: true, cancelable: true })
	Object.assign(e, { clientX: x, clientY: y, deltaY, deltaMode, ctrlKey: false })
	return e
}

describe('zoom maths', () => {
	it('never leaves the supported range', () => {
		expect(clampScale(0.1)).toBe(ZOOM_MIN)
		expect(clampScale(99)).toBe(ZOOM_MAX)
		expect(clampScale(Number.NaN)).toBe(ZOOM_MIN)
		expect(clampScale(2.5)).toBe(2.5)
	})

	it('pins an unzoomed photo', () => {
		expect(panBounds(1, VIEWPORT)).toEqual({ x: 0, y: 0 })
		expect(clampOffset({ x: 120, y: -80 }, 1, VIEWPORT)).toEqual({ x: 0, y: 0 })
	})

	it('allows exactly the overflow once zoomed', () => {
		expect(panBounds(2, VIEWPORT)).toEqual({ x: 400, y: 300 })
		expect(clampOffset({ x: 999, y: -999 }, 2, VIEWPORT)).toEqual({ x: 400, y: -300 })
	})

	it('measures bounds from the painted photo, not the letterbox', () => {
		// A 400x600 photo inside an 800x600 stage: at 2x it is 800 wide, which
		// exactly fills the stage, so there is nothing to pan horizontally.
		const content = { width: 400, height: 600 }
		expect(panBounds(2, VIEWPORT, content)).toEqual({ x: 0, y: 300 })
	})

	it('keeps the focal point under the finger', () => {
		// Focus 200px right of centre, zooming 1x -> 2x.
		const next = zoomTo({
			scale: 1,
			offset: { x: 0, y: 0 },
			nextScale: 2,
			viewport: VIEWPORT,
			focus: { x: 200, y: 0 },
		})
		expect(next.scale).toBe(2)
		expect(next.offset.x).toBe(-200)
		// Content point under the focus before: (200 - 0) / 1 = 200.
		// After: offset + 200 * 2 = -200 + 400 = 200. Unmoved.
	})

	it('clamps a zoom-out back to centre', () => {
		const next = zoomTo({
			scale: 4,
			offset: { x: 900, y: -400 },
			nextScale: 1,
			viewport: VIEWPORT,
		})
		expect(next).toEqual({ scale: 1, offset: { x: 0, y: 0 } })
	})
})

describe('swipeIntent', () => {
	it('steps forward on a leftward flick', () => {
		expect(
			swipeIntent({ dx: -SWIPE_DISTANCE_PX, dy: 4, elapsedMs: 200, scale: 1 }),
		).toBe('next')
	})

	it('steps back on a rightward flick', () => {
		expect(
			swipeIntent({ dx: SWIPE_DISTANCE_PX, dy: -4, elapsedMs: 200, scale: 1 }),
		).toBe('prev')
	})

	it('takes a short but fast flick', () => {
		expect(swipeIntent({ dx: -30, dy: 0, elapsedMs: 60, scale: 1 })).toBe('next')
	})

	it('ignores a slow nudge', () => {
		expect(swipeIntent({ dx: -20, dy: 0, elapsedMs: 900, scale: 1 })).toBe(null)
	})

	it('ignores a tap that slipped, however fast it was', () => {
		// Just past the tap slop, over a few frames: a shaky finger, not a flick.
		expect(swipeIntent({ dx: -13, dy: 0, elapsedMs: 30, scale: 1 })).toBe(null)
		expect(swipeIntent({ dx: SWIPE_MIN_PX - 1, dy: 0, elapsedMs: 20, scale: 1 })).toBe(
			null,
		)
	})

	it('dismisses on a downward swipe', () => {
		expect(
			swipeIntent({ dx: 0, dy: DISMISS_DISTANCE_PX, elapsedMs: 300, scale: 1 }),
		).toBe('close')
	})

	it('does not dismiss on a swipe up', () => {
		expect(swipeIntent({ dx: 0, dy: -300, elapsedMs: 300, scale: 1 })).toBe(null)
	})

	it('never fires while zoomed — that drag was a pan', () => {
		expect(swipeIntent({ dx: -200, dy: 0, elapsedMs: 100, scale: 2 })).toBe(null)
		expect(swipeIntent({ dx: 0, dy: 300, elapsedMs: 300, scale: 2 })).toBe(null)
	})
})

describe('createImageZoom', () => {
	/** @type {HTMLElement} */
	let host
	/** @type {() => void} */
	let detach
	let state
	let swipes
	let dismissed
	let taps

	const build = (extra = {}) => {
		state = { scale: 1, offset: { x: 0, y: 0 } }
		swipes = []
		dismissed = 0
		taps = 0
		const zoom = createImageZoom({
			onChange: (next) => {
				state = next
			},
			onSwipe: (d) => swipes.push(d),
			onDismiss: () => {
				dismissed += 1
			},
			onTap: () => {
				taps += 1
			},
			...extra,
		})
		host = makeHost()
		detach = zoom.attach(host)
		return zoom
	}

	beforeEach(() => {
		vi.useFakeTimers()
		vi.setSystemTime(new Date('2026-01-01T00:00:00Z'))
	})

	afterEach(() => {
		detach?.()
		host?.remove()
		vi.useRealTimers()
	})

	it('zooms in and out with the magnifier buttons', () => {
		const zoom = build()
		zoom.zoomIn()
		expect(state.scale).toBeCloseTo(ZOOM_BUTTON_FACTOR)
		zoom.zoomOut()
		expect(state.scale).toBeCloseTo(1)
	})

	it('stops at the ends of the range', () => {
		const zoom = build()
		for (let i = 0; i < 40; i++) zoom.zoomIn()
		expect(zoom.scale).toBe(ZOOM_MAX)
		for (let i = 0; i < 40; i++) zoom.zoomOut()
		expect(zoom.scale).toBe(ZOOM_MIN)
		expect(zoom.offset).toEqual({ x: 0, y: 0 })
	})

	it('zooms on a double tap and returns on the next one', () => {
		build()
		host.dispatchEvent(pointer('pointerdown', { x: 400, y: 300 }))
		host.dispatchEvent(pointer('pointerup', { x: 400, y: 300 }))
		vi.advanceTimersByTime(100)
		host.dispatchEvent(pointer('pointerdown', { x: 400, y: 300 }))
		host.dispatchEvent(pointer('pointerup', { x: 400, y: 300 }))
		expect(state.scale).toBe(DOUBLE_TAP_SCALE)

		vi.advanceTimersByTime(100)
		host.dispatchEvent(pointer('pointerdown', { x: 400, y: 300 }))
		host.dispatchEvent(pointer('pointerup', { x: 400, y: 300 }))
		vi.advanceTimersByTime(100)
		host.dispatchEvent(pointer('pointerdown', { x: 400, y: 300 }))
		host.dispatchEvent(pointer('pointerup', { x: 400, y: 300 }))
		expect(state.scale).toBe(ZOOM_MIN)
	})

	it('reports a lone tap instead of zooming, but only once it is lone', () => {
		build()
		host.dispatchEvent(pointer('pointerdown', { x: 10, y: 10 }))
		host.dispatchEvent(pointer('pointerup', { x: 10, y: 10 }))
		// Reporting it immediately would fire on the first half of a double tap.
		expect(taps).toBe(0)
		vi.advanceTimersByTime(DOUBLE_TAP_MS + 10)
		expect(taps).toBe(1)
		expect(state.scale).toBe(ZOOM_MIN)
	})

	it('never reports the first half of a double tap as a tap', () => {
		build()
		host.dispatchEvent(pointer('pointerdown', { x: 400, y: 300 }))
		host.dispatchEvent(pointer('pointerup', { x: 400, y: 300 }))
		vi.advanceTimersByTime(100)
		host.dispatchEvent(pointer('pointerdown', { x: 400, y: 300 }))
		host.dispatchEvent(pointer('pointerup', { x: 400, y: 300 }))
		vi.advanceTimersByTime(1000)
		expect(taps).toBe(0)
		expect(state.scale).toBe(DOUBLE_TAP_SCALE)
	})

	it('pinches to zoom around the midpoint', () => {
		build()
		host.dispatchEvent(pointer('pointerdown', { id: 1, x: 300, y: 300 }))
		host.dispatchEvent(pointer('pointerdown', { id: 2, x: 500, y: 300 }))
		// Fingers move from 200px apart to 400px apart: 2x.
		host.dispatchEvent(pointer('pointermove', { id: 1, x: 200, y: 300 }))
		host.dispatchEvent(pointer('pointermove', { id: 2, x: 600, y: 300 }))
		expect(state.scale).toBeCloseTo(2)
		host.dispatchEvent(pointer('pointerup', { id: 1, x: 200, y: 300 }))
		host.dispatchEvent(pointer('pointerup', { id: 2, x: 600, y: 300 }))
		expect(state.scale).toBeCloseTo(2)
	})

	it('snaps home when a pinch ends below 1x', () => {
		const zoom = build()
		// Start zoomed and panned off-centre, then pinch all the way back out.
		zoom.zoomToScale(4)
		zoom.panBy(300, 200)
		expect(state.offset).not.toEqual({ x: 0, y: 0 })
		host.dispatchEvent(pointer('pointerdown', { id: 1, x: 300, y: 300 }))
		host.dispatchEvent(pointer('pointerdown', { id: 2, x: 500, y: 300 }))
		host.dispatchEvent(pointer('pointermove', { id: 1, x: 398, y: 300 }))
		host.dispatchEvent(pointer('pointermove', { id: 2, x: 402, y: 300 }))
		host.dispatchEvent(pointer('pointerup', { id: 1, x: 398, y: 300 }))
		host.dispatchEvent(pointer('pointerup', { id: 2, x: 402, y: 300 }))
		expect(state.scale).toBe(ZOOM_MIN)
		expect(state.offset).toEqual({ x: 0, y: 0 })
	})

	it('survives a third finger landing and leaving mid-pinch', () => {
		build()
		host.dispatchEvent(pointer('pointerdown', { id: 1, x: 300, y: 300 }))
		host.dispatchEvent(pointer('pointerdown', { id: 2, x: 500, y: 300 }))
		host.dispatchEvent(pointer('pointermove', { id: 1, x: 200, y: 300 }))
		host.dispatchEvent(pointer('pointermove', { id: 2, x: 600, y: 300 }))
		expect(state.scale).toBeCloseTo(2)

		// A palm lands, then the first finger lifts. The pinch must re-anchor to
		// the pair that is left instead of jumping to a different distance.
		host.dispatchEvent(pointer('pointerdown', { id: 3, x: 300, y: 300 }))
		host.dispatchEvent(pointer('pointerup', { id: 1, x: 200, y: 300 }))
		expect(state.scale).toBeCloseTo(2)
		host.dispatchEvent(pointer('pointermove', { id: 3, x: 297, y: 300 }))
		// 3px on a 300px spread: a nudge, not a jump to some other pair's ratio.
		expect(state.scale).toBeCloseTo(2.02, 2)
	})

	it('forgets a gesture the browser took away', () => {
		const zoom = build()
		zoom.zoomToScale(2)
		host.dispatchEvent(pointer('pointerdown', { x: 400, y: 300 }))
		host.dispatchEvent(pointer('pointermove', { x: 450, y: 300 }))
		expect(state.offset.x).toBe(50)
		host.dispatchEvent(pointer('pointercancel', { x: 450, y: 300 }))
		// The finger is gone: further movement must not drag the photo.
		host.dispatchEvent(pointer('pointermove', { x: 700, y: 300 }))
		expect(state.offset.x).toBe(50)
	})

	it('ends the drag when the capture is lost instead of following the cursor', () => {
		const zoom = build()
		zoom.zoomToScale(2)
		host.dispatchEvent(pointer('pointerdown', { x: 400, y: 300, pointerType: 'mouse' }))
		host.dispatchEvent(pointer('pointermove', { x: 450, y: 300, pointerType: 'mouse' }))
		host.dispatchEvent(pointer('lostpointercapture', { x: 450, y: 300 }))
		host.dispatchEvent(pointer('pointermove', { x: 700, y: 300, pointerType: 'mouse' }))
		expect(state.offset.x).toBe(50)
	})

	it('keeps panning when another mouse button is released mid-drag', () => {
		const zoom = build()
		zoom.zoomToScale(2)
		host.dispatchEvent(pointer('pointerdown', { x: 400, y: 300, pointerType: 'mouse' }))
		host.dispatchEvent(pointer('pointermove', { x: 450, y: 300, pointerType: 'mouse' }))
		const rightUp = pointer('pointerup', { x: 450, y: 300, pointerType: 'mouse' })
		rightUp.button = 2
		host.dispatchEvent(rightUp)
		host.dispatchEvent(pointer('pointermove', { x: 500, y: 300, pointerType: 'mouse' }))
		expect(state.offset.x).toBe(100)
	})

	it('still ends the gesture when the button comes up outside the surface', () => {
		build()
		host.dispatchEvent(pointer('pointerdown', { x: 400, y: 300 }))
		host.dispatchEvent(pointer('pointermove', { x: 200, y: 300 }))
		window.dispatchEvent(pointer('pointerup', { x: 200, y: 300 }))
		expect(swipes).toEqual(['next'])
	})

	it('pans a zoomed photo and stops at its edge', () => {
		const zoom = build()
		zoom.zoomToScale(2)
		host.dispatchEvent(pointer('pointerdown', { x: 400, y: 300 }))
		host.dispatchEvent(pointer('pointermove', { x: 500, y: 300 }))
		expect(state.offset.x).toBe(100)
		host.dispatchEvent(pointer('pointermove', { x: 5000, y: 300 }))
		expect(state.offset.x).toBe(400)
		host.dispatchEvent(pointer('pointerup', { x: 5000, y: 300 }))
		expect(swipes).toEqual([])
	})

	it('swipes to the neighbouring file only while unzoomed', () => {
		const zoom = build()
		host.dispatchEvent(pointer('pointerdown', { x: 400, y: 300 }))
		host.dispatchEvent(pointer('pointermove', { x: 200, y: 300 }))
		host.dispatchEvent(pointer('pointerup', { x: 200, y: 300 }))
		expect(swipes).toEqual(['next'])

		zoom.zoomToScale(3)
		host.dispatchEvent(pointer('pointerdown', { x: 400, y: 300 }))
		host.dispatchEvent(pointer('pointermove', { x: 200, y: 300 }))
		host.dispatchEvent(pointer('pointerup', { x: 200, y: 300 }))
		expect(swipes).toEqual(['next'])
	})

	it('dismisses on a downward swipe', () => {
		build()
		host.dispatchEvent(pointer('pointerdown', { x: 400, y: 100 }))
		host.dispatchEvent(pointer('pointermove', { x: 400, y: 350 }))
		host.dispatchEvent(pointer('pointerup', { x: 400, y: 350 }))
		expect(dismissed).toBe(1)
	})

	it('zooms with the wheel around the cursor', () => {
		build()
		// 200px right of the 800x600 centre: that point must stay put.
		host.dispatchEvent(wheel({ x: 600, y: 300, deltaY: -300 }))
		expect(state.scale).toBeGreaterThan(1)
		const before = 200 // content point under the cursor at 1x
		expect(state.offset.x + before * state.scale).toBeCloseTo(200, 5)
		host.dispatchEvent(wheel({ x: 600, y: 300, deltaY: 3000 }))
		expect(state.scale).toBe(ZOOM_MIN)
		expect(state.offset).toEqual({ x: 0, y: 0 })
	})

	it('zooms on a wheel that reports lines instead of pixels', () => {
		build()
		// Firefox sends deltaMode 1 with ~3 lines per notch.
		host.dispatchEvent(wheel({ x: 400, y: 300, deltaY: -3, deltaMode: 1 }))
		expect(state.scale).toBeGreaterThan(1.05)
	})

	it('swallows a trackpad pinch even at the end of the range', () => {
		const zoom = build()
		zoom.zoomToScale(ZOOM_MAX)
		const e = wheel({ x: 400, y: 300, deltaY: -300 })
		e.ctrlKey = true
		let prevented = false
		e.preventDefault = () => {
			prevented = true
		}
		host.dispatchEvent(e)
		// Letting it through would zoom the whole page instead of the photo.
		expect(prevented).toBe(true)
		expect(zoom.scale).toBe(ZOOM_MAX)
	})

	it('does nothing at all when disabled', () => {
		build({ isEnabled: () => false })
		host.dispatchEvent(pointer('pointerdown', { x: 400, y: 300 }))
		host.dispatchEvent(pointer('pointermove', { x: 200, y: 300 }))
		host.dispatchEvent(pointer('pointerup', { x: 200, y: 300 }))
		host.dispatchEvent(wheel({ x: 400, y: 300, deltaY: -300 }))
		expect(state.scale).toBe(ZOOM_MIN)
		expect(swipes).toEqual([])
		expect(taps).toBe(0)
	})

	it('drops its listeners on detach', () => {
		const zoom = build()
		detach()
		host.dispatchEvent(wheel({ x: 400, y: 300, deltaY: -300 }))
		expect(zoom.scale).toBe(ZOOM_MIN)
	})

	it('keeps the pan inside the painted photo', () => {
		const zoom = build({ getContentSize: () => ({ width: 400, height: 600 }) })
		zoom.zoomToScale(2)
		host.dispatchEvent(pointer('pointerdown', { x: 400, y: 300 }))
		host.dispatchEvent(pointer('pointermove', { x: 700, y: 300 }))
		// 400 * 2 == the 800px stage: the photo fills it, nothing to pan into.
		expect(state.offset.x).toBe(0)
	})
})
