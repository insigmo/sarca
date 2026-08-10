import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

import { startAutoRefresh, nextDelay, MIN_DELAY_MS, MAX_DELAY_MS } from './autoRefresh'

/** Minimal Document/Window stand-ins so the timers stay under our control. */
const makeEnv = (visibilityState = 'visible') => {
	const listeners = {}
	const add = (type, fn) => {
		listeners[type] = listeners[type] || new Set()
		listeners[type].add(fn)
	}
	const remove = (type, fn) => listeners[type]?.delete(fn)
	const emit = (type) => listeners[type]?.forEach((fn) => fn())

	const doc = {
		visibilityState,
		addEventListener: add,
		removeEventListener: remove,
	}
	const win = {
		addEventListener: add,
		removeEventListener: remove,
		setTimeout: (fn, ms) => setTimeout(fn, ms),
		clearTimeout: (id) => clearTimeout(id),
	}
	return { doc, win, emit, listenerCount: () => Object.values(listeners).reduce((n, s) => n + s.size, 0) }
}

describe('nextDelay', () => {
	afterEach(() => {
		vi.restoreAllMocks()
	})

	it('stays inside the 5-15s window at both extremes', () => {
		vi.spyOn(Math, 'random').mockReturnValue(0)
		expect(nextDelay()).toBe(MIN_DELAY_MS)
		vi.spyOn(Math, 'random').mockReturnValue(0.999999)
		expect(nextDelay()).toBe(MAX_DELAY_MS)
	})

	it('varies between calls instead of returning one fixed interval', () => {
		const sequence = [0, 0.5, 0.9]
		let i = 0
		vi.spyOn(Math, 'random').mockImplementation(() => sequence[i++ % sequence.length])
		const delays = [nextDelay(), nextDelay(), nextDelay()]
		expect(new Set(delays).size).toBe(3)
		for (const d of delays) {
			expect(d).toBeGreaterThanOrEqual(MIN_DELAY_MS)
			expect(d).toBeLessThanOrEqual(MAX_DELAY_MS)
		}
	})
})

describe('startAutoRefresh', () => {
	beforeEach(() => {
		vi.useFakeTimers()
	})

	afterEach(() => {
		vi.useRealTimers()
		vi.restoreAllMocks()
	})

	it('re-runs on a fresh random delay each tick', async () => {
		vi.spyOn(Math, 'random').mockReturnValue(0)
		const env = makeEnv()
		const run = vi.fn()
		const stop = startAutoRefresh({ run, doc: env.doc, win: env.win })

		expect(run).not.toHaveBeenCalled()
		await vi.advanceTimersByTimeAsync(MIN_DELAY_MS)
		expect(run).toHaveBeenCalledTimes(1)
		await vi.advanceTimersByTimeAsync(MIN_DELAY_MS)
		expect(run).toHaveBeenCalledTimes(2)

		stop()
	})

	it('never fires below the lower bound', async () => {
		vi.spyOn(Math, 'random').mockReturnValue(0)
		const env = makeEnv()
		const run = vi.fn()
		const stop = startAutoRefresh({ run, doc: env.doc, win: env.win })

		await vi.advanceTimersByTimeAsync(MIN_DELAY_MS - 1)
		expect(run).not.toHaveBeenCalled()

		stop()
	})

	it('does not tick while the document is hidden', async () => {
		vi.spyOn(Math, 'random').mockReturnValue(0)
		const env = makeEnv('hidden')
		const run = vi.fn()
		const stop = startAutoRefresh({ run, doc: env.doc, win: env.win })

		await vi.advanceTimersByTimeAsync(MAX_DELAY_MS * 3)
		expect(run).not.toHaveBeenCalled()

		// Becoming visible again restarts the cadence.
		env.doc.visibilityState = 'visible'
		env.emit('visibilitychange')
		await vi.advanceTimersByTimeAsync(MIN_DELAY_MS)
		expect(run).toHaveBeenCalledTimes(1)

		stop()
	})

	it('skips a tick instead of re-entering a run that has not settled', async () => {
		vi.spyOn(Math, 'random').mockReturnValue(0)
		const env = makeEnv()
		let release
		const pending = new Promise((resolve) => {
			release = resolve
		})
		const run = vi.fn(() => pending)
		const stop = startAutoRefresh({ run, doc: env.doc, win: env.win })

		await vi.advanceTimersByTimeAsync(MIN_DELAY_MS)
		expect(run).toHaveBeenCalledTimes(1)

		// The first run is still in flight: further time must not stack calls.
		await vi.advanceTimersByTimeAsync(MAX_DELAY_MS * 3)
		expect(run).toHaveBeenCalledTimes(1)

		release()
		await vi.advanceTimersByTimeAsync(MIN_DELAY_MS)
		expect(run).toHaveBeenCalledTimes(2)

		stop()
	})

	it('honours the isPaused veto without stopping the loop', async () => {
		vi.spyOn(Math, 'random').mockReturnValue(0)
		const env = makeEnv()
		let paused = true
		const run = vi.fn()
		const stop = startAutoRefresh({
			run,
			isPaused: () => paused,
			doc: env.doc,
			win: env.win,
		})

		await vi.advanceTimersByTimeAsync(MIN_DELAY_MS * 2)
		expect(run).not.toHaveBeenCalled()

		paused = false
		await vi.advanceTimersByTimeAsync(MIN_DELAY_MS)
		expect(run).toHaveBeenCalledTimes(1)

		stop()
	})

	it('a failing run does not kill the loop', async () => {
		vi.spyOn(Math, 'random').mockReturnValue(0)
		const env = makeEnv()
		const run = vi.fn().mockRejectedValue(new Error('offline'))
		const stop = startAutoRefresh({ run, doc: env.doc, win: env.win })

		await vi.advanceTimersByTimeAsync(MIN_DELAY_MS)
		expect(run).toHaveBeenCalledTimes(1)
		await vi.advanceTimersByTimeAsync(MIN_DELAY_MS)
		expect(run).toHaveBeenCalledTimes(2)

		stop()
	})

	it('stop removes every timer and listener', async () => {
		vi.spyOn(Math, 'random').mockReturnValue(0)
		const env = makeEnv()
		const run = vi.fn()
		const stop = startAutoRefresh({ run, doc: env.doc, win: env.win })
		expect(env.listenerCount()).toBeGreaterThan(0)

		stop()
		stop() // idempotent

		expect(env.listenerCount()).toBe(0)
		await vi.advanceTimersByTimeAsync(MAX_DELAY_MS * 3)
		expect(run).not.toHaveBeenCalled()
	})
})
