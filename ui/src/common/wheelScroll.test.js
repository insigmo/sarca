import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'

import {
	deltaToPixels,
	scrollableAncestor,
	isLinuxWebKit,
	installWheelScrollFix,
	LINE_HEIGHT_PX,
	TWEEN_MS,
} from './wheelScroll'

describe('deltaToPixels', () => {
	// WebKitGTK is the only build that reports line deltas; treating a "3" as
	// three pixels is what made the file list crawl on Linux.
	it('scales line deltas up to pixels', () => {
		expect(deltaToPixels({ deltaY: 3, deltaMode: 1 }, 800)).toBe(3 * LINE_HEIGHT_PX)
	})

	it('scales page deltas against the viewport', () => {
		expect(deltaToPixels({ deltaY: 1, deltaMode: 2 }, 1000)).toBe(900)
	})

	it('leaves pixel deltas untouched', () => {
		expect(deltaToPixels({ deltaY: 42, deltaMode: 0 }, 800)).toBe(42)
	})
})

describe('isLinuxWebKit', () => {
	it('matches the Linux desktop build only', () => {
		expect(isLinuxWebKit('Mozilla/5.0 (X11; Linux x86_64)')).toBe(true)
		expect(isLinuxWebKit('Mozilla/5.0 (Linux; Android 14)')).toBe(false)
		expect(isLinuxWebKit('Mozilla/5.0 (Macintosh; Intel Mac OS X)')).toBe(false)
	})
})

describe('scrollableAncestor / installWheelScrollFix', () => {
	let root
	let scroller

	const makeScrollable = (el, { scrollHeight = 1000, clientHeight = 300 } = {}) => {
		Object.defineProperty(el, 'scrollHeight', { value: scrollHeight, configurable: true })
		Object.defineProperty(el, 'clientHeight', { value: clientHeight, configurable: true })
		el.style.overflowY = 'auto'
	}

	beforeEach(() => {
		vi.useFakeTimers()
		vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Mozilla/5.0 (X11; Linux x86_64)')
		root = document.createElement('div')
		scroller = document.createElement('div')
		const child = document.createElement('span')
		scroller.appendChild(child)
		root.appendChild(scroller)
		document.body.appendChild(root)
		makeScrollable(scroller)
	})

	afterEach(() => {
		root.remove()
		vi.useRealTimers()
		vi.restoreAllMocks()
	})

	it('finds the scrollable ancestor of a deep target', () => {
		const child = scroller.firstChild
		expect(scrollableAncestor(child, root)).toBe(scroller)
	})

	it('returns null when nothing can scroll', () => {
		const plain = document.createElement('div')
		root.appendChild(plain)
		expect(scrollableAncestor(plain, root)).toBe(null)
	})

	it('tweens a line-mode wheel to a pixel scroll instead of snapping', () => {
		const stop = installWheelScrollFix(root)
		const event = new WheelEvent('wheel', { deltaY: 3, bubbles: true, cancelable: true })
		Object.defineProperty(event, 'deltaMode', { value: 1 })
		scroller.firstChild.dispatchEvent(event)

		// Jumping straight to the target on the same tick is the jerky
		// behaviour this replaces — the tween should still be in flight.
		expect(scroller.scrollTop).toBe(0)
		expect(event.defaultPrevented).toBe(true)

		vi.advanceTimersByTime(TWEEN_MS / 2)
		expect(scroller.scrollTop).toBeGreaterThan(0)
		expect(scroller.scrollTop).toBeLessThan(3 * LINE_HEIGHT_PX)

		vi.advanceTimersByTime(TWEEN_MS)
		expect(scroller.scrollTop).toBe(3 * LINE_HEIGHT_PX)
		stop()
	})

	it('leaves pixel-mode wheels to the browser', () => {
		const stop = installWheelScrollFix(root)
		const event = new WheelEvent('wheel', { deltaY: 50, bubbles: true, cancelable: true })
		Object.defineProperty(event, 'deltaMode', { value: 0 })
		scroller.firstChild.dispatchEvent(event)

		expect(scroller.scrollTop).toBe(0)
		expect(event.defaultPrevented).toBe(false)
		stop()
	})

	it('lets the event chain when already at the edge', () => {
		const stop = installWheelScrollFix(root)
		const event = new WheelEvent('wheel', { deltaY: -3, bubbles: true, cancelable: true })
		Object.defineProperty(event, 'deltaMode', { value: 1 })
		scroller.firstChild.dispatchEvent(event)

		expect(scroller.scrollTop).toBe(0)
		expect(event.defaultPrevented).toBe(false)
		stop()
	})

	it('is a no-op off Linux', () => {
		vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Mozilla/5.0 (Macintosh)')
		const stop = installWheelScrollFix(root)
		const event = new WheelEvent('wheel', { deltaY: 3, bubbles: true, cancelable: true })
		Object.defineProperty(event, 'deltaMode', { value: 1 })
		scroller.firstChild.dispatchEvent(event)

		expect(scroller.scrollTop).toBe(0)
		stop()
	})
})
