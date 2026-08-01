import { describe, it, expect, vi, afterEach } from 'vitest'

import { createRafBatcher } from './rafBatch'

describe('createRafBatcher', () => {
	afterEach(() => {
		vi.useRealTimers()
	})

	// Regression: marquee-selection hit-testing in Files/index.jsx used to run
	// pathsIntersectingClientRect() — a querySelectorAll + getBoundingClientRect
	// pass over every visible tile — on every raw mousemove event. In a large
	// (photo) folder this fired dozens of times per second and made the drag
	// selection visibly janky ("интерфейс тормознутый"). It must run at most
	// once per animation frame, using only the most recent position.
	it('collapses many schedule() calls within a frame into a single fn call using the latest args', () => {
		vi.stubGlobal('requestAnimationFrame', (cb) => {
			queueMicrotask(cb)
			return 1
		})
		const fn = vi.fn()
		const batcher = createRafBatcher(fn)

		batcher.schedule(1, 1)
		batcher.schedule(2, 2)
		batcher.schedule(3, 3)

		expect(fn).not.toHaveBeenCalled()
		return Promise.resolve().then(() => {
			expect(fn).toHaveBeenCalledTimes(1)
			expect(fn).toHaveBeenCalledWith(3, 3)
		})
	})

	it('cancel() prevents a pending call from firing', async () => {
		let scheduledCb = null
		vi.stubGlobal('requestAnimationFrame', (cb) => {
			scheduledCb = cb
			return 1
		})
		vi.stubGlobal('cancelAnimationFrame', () => {
			scheduledCb = null
		})
		const fn = vi.fn()
		const batcher = createRafBatcher(fn)

		batcher.schedule(1, 1)
		batcher.cancel()
		if (scheduledCb) scheduledCb()

		expect(fn).not.toHaveBeenCalled()
	})

	it('schedules a fresh frame after a previous batch already ran', async () => {
		const callbacks = []
		vi.stubGlobal('requestAnimationFrame', (cb) => {
			callbacks.push(cb)
			return callbacks.length
		})
		const fn = vi.fn()
		const batcher = createRafBatcher(fn)

		batcher.schedule(1, 1)
		callbacks.shift()()
		expect(fn).toHaveBeenCalledTimes(1)

		batcher.schedule(2, 2)
		expect(callbacks).toHaveLength(1)
		callbacks.shift()()
		expect(fn).toHaveBeenCalledTimes(2)
		expect(fn).toHaveBeenLastCalledWith(2, 2)
	})
})
