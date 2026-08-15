import { describe, it, expect, vi } from 'vitest'
import {
	enqueueThumbFetch,
	clearThumbQueue,
	thumbQueueStats,
	THUMB_MAX_CONCURRENT,
} from './thumbQueue'

describe('thumbQueue', () => {
	it('limits concurrent runners to THUMB_MAX_CONCURRENT', async () => {
		let inFlight = 0
		let maxInFlight = 0
		/** @type {Array<() => void>} */
		const releases = []

		const tasks = Array.from({ length: THUMB_MAX_CONCURRENT + 4 }, () =>
			enqueueThumbFetch(
				() =>
					new Promise((resolve) => {
						inFlight += 1
						maxInFlight = Math.max(maxInFlight, inFlight)
						releases.push(() => {
							inFlight -= 1
							resolve('ok')
						})
					}),
			),
		)

		await Promise.resolve()
		await Promise.resolve()

		expect(maxInFlight).toBe(THUMB_MAX_CONCURRENT)
		expect(thumbQueueStats().waiting).toBe(4)

		while (releases.length) {
			releases.shift()?.()
			await Promise.resolve()
			await Promise.resolve()
		}

		await Promise.all(tasks)
		expect(thumbQueueStats().active).toBe(0)
		expect(thumbQueueStats().waiting).toBe(0)
	})

	it('rejects immediately when signal already aborted', async () => {
		const ac = new AbortController()
		ac.abort()
		await expect(
			enqueueThumbFetch(() => Promise.resolve(1), { signal: ac.signal }),
		).rejects.toMatchObject({ name: 'AbortError' })
	})

	it('clearThumbQueue rejects waiters and aborts in-flight', async () => {
		let started = 0
		/** @type {AbortSignal[]} */
		const signals = []
		const tasks = Array.from({ length: THUMB_MAX_CONCURRENT + 3 }, () =>
			enqueueThumbFetch((signal) => {
				started += 1
				signals.push(signal)
				return new Promise(() => {})
			}),
		)

		await Promise.resolve()
		await Promise.resolve()
		expect(started).toBe(THUMB_MAX_CONCURRENT)
		expect(thumbQueueStats().waiting).toBe(3)

		clearThumbQueue()
		expect(thumbQueueStats().waiting).toBe(0)
		expect(signals.every((s) => s.aborted)).toBe(true)

		const results = await Promise.allSettled(tasks)
		expect(results.every((r) => r.status === 'rejected')).toBe(true)
	})

	it('retries a 503 with backoff instead of failing the tile', async () => {
		vi.useFakeTimers()
		try {
			let calls = 0
			const promise = enqueueThumbFetch(() => {
				calls += 1
				if (calls < 3) {
					const err = new Error('storage is busy, retry shortly')
					err.status = 503
					return Promise.reject(err)
				}
				return Promise.resolve('ok')
			})

			await vi.advanceTimersByTimeAsync(1000)
			await vi.advanceTimersByTimeAsync(2000)

			await expect(promise).resolves.toBe('ok')
			expect(calls).toBe(3)
		} finally {
			vi.useRealTimers()
		}
	})

	it('gives up after exhausting 503 retries', async () => {
		vi.useFakeTimers()
		try {
			let calls = 0
			const err = new Error('storage is busy, retry shortly')
			err.status = 503
			const promise = enqueueThumbFetch(() => {
				calls += 1
				return Promise.reject(err)
			})
			promise.catch(() => {})

			// Delays sum to ~64s — the retry budget spans the server's 1-minute
			// rate-limit window so a tile only gives up once that window can no
			// longer explain the failure.
			await vi.advanceTimersByTimeAsync(1000)
			await vi.advanceTimersByTimeAsync(2000)
			await vi.advanceTimersByTimeAsync(4000)
			await vi.advanceTimersByTimeAsync(8000)
			await vi.advanceTimersByTimeAsync(15000)
			await vi.advanceTimersByTimeAsync(15000)
			await vi.advanceTimersByTimeAsync(19000)

			await expect(promise).rejects.toBe(err)
			expect(calls).toBe(8)
		} finally {
			vi.useRealTimers()
		}
	})
})
