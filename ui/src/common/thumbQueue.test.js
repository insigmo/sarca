import { describe, it, expect, vi } from 'vitest'
import {
	enqueueThumbFetch,
	clearThumbQueue,
	pauseThumbQueue,
	resumeThumbQueue,
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

	it('retries a 503 with jittered backoff instead of failing the tile', async () => {
		vi.useFakeTimers()
		// Zero the jitter so the exponential base (1s, 2s, …) is exact and the
		// timer advances below land precisely on a retry.
		const randomSpy = vi.spyOn(Math, 'random').mockReturnValue(0)
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
			randomSpy.mockRestore()
			vi.useRealTimers()
		}
	})

	it('honours a server Retry-After instead of guessing the delay', async () => {
		vi.useFakeTimers()
		const randomSpy = vi.spyOn(Math, 'random').mockReturnValue(0)
		try {
			let calls = 0
			const promise = enqueueThumbFetch(() => {
				calls += 1
				if (calls < 2) {
					const err = new Error('storage is busy, retry shortly')
					err.status = 503
					err.retryAfter = 5
					return Promise.reject(err)
				}
				return Promise.resolve('ok')
			})

			// Not due yet — the server asked for 5s, not our ~1s default guess.
			await vi.advanceTimersByTimeAsync(4000)
			expect(calls).toBe(1)

			// Due by 5s, matching Retry-After exactly (no jitter, mocked to 0).
			await vi.advanceTimersByTimeAsync(1000)
			await expect(promise).resolves.toBe('ok')
			expect(calls).toBe(2)
		} finally {
			randomSpy.mockRestore()
			vi.useRealTimers()
		}
	})

	it('gives up after exhausting 503 retries', async () => {
		vi.useFakeTimers()
		const randomSpy = vi.spyOn(Math, 'random').mockReturnValue(0)
		try {
			let calls = 0
			const err = new Error('storage is busy, retry shortly')
			err.status = 503
			const promise = enqueueThumbFetch(() => {
				calls += 1
				return Promise.reject(err)
			})
			promise.catch(() => {})

			// 8 attempts total: base ~1s doubling each time, capped at ~30s.
			for (const ms of [1000, 2000, 4000, 8000, 16000, 30000, 30000]) {
				await vi.advanceTimersByTimeAsync(ms)
			}

			await expect(promise).rejects.toBe(err)
			expect(calls).toBe(8)
		} finally {
			randomSpy.mockRestore()
			vi.useRealTimers()
		}
	})

	it('releases the concurrency slot while a tile backs off, instead of holding it', async () => {
		// One tile that will 503 and go into backoff.
		let flakyCalls = 0
		const flaky = enqueueThumbFetch(() => {
			flakyCalls += 1
			const err = new Error('storage is busy, retry shortly')
			err.status = 503
			return Promise.reject(err)
		})
		flaky.catch(() => {})

		// Enough hanging tiles to fill every slot on their own. Together with
		// the flaky one that is seven entries for six slots, so the last one
		// can only run if the backing-off tile gave its slot back.
		let hangingStarts = 0
		/** @type {Array<() => void>} */
		const releases = []
		const hanging = Array.from({ length: THUMB_MAX_CONCURRENT }, () =>
			enqueueThumbFetch(
				() =>
					new Promise((resolve) => {
						hangingStarts += 1
						releases.push(() => resolve('ok'))
					}),
			),
		)

		// A macrotask boundary rather than a fixed number of microtask ticks:
		// the rejection has to walk the queue's internal promise chain before
		// the freed slot is handed on, and counting the exact number of ticks
		// that takes would break on any refactor of it.
		await new Promise((resolve) => setTimeout(resolve, 0))

		// The flaky tile's first attempt has already failed and it is now
		// sleeping before its retry — that must not be counted against the
		// concurrency limit, so the seventh task gets to run instead of
		// sitting in the queue behind a tile that is doing nothing.
		expect(flakyCalls).toBe(1)
		expect(hangingStarts).toBe(THUMB_MAX_CONCURRENT)
		expect(thumbQueueStats().active).toBe(THUMB_MAX_CONCURRENT)
		expect(thumbQueueStats().waiting).toBe(0)

		clearThumbQueue()
		for (const release of releases) release()
		await Promise.allSettled([flaky, ...hanging])
	})

	it('hands out no new slots while paused, and catches up on resume', async () => {
		let started = 0
		/** @type {Array<() => void>} */
		const releases = []
		const task = () =>
			enqueueThumbFetch(
				() =>
					new Promise((resolve) => {
						started += 1
						releases.push(() => resolve('ok'))
					}),
			)

		pauseThumbQueue()
		const tasks = Array.from({ length: 3 }, task)
		await new Promise((resolve) => setTimeout(resolve, 0))

		// Queued, not running: a video is holding the connections.
		expect(started).toBe(0)
		expect(thumbQueueStats().active).toBe(0)
		expect(thumbQueueStats().waiting).toBe(3)

		resumeThumbQueue()
		await new Promise((resolve) => setTimeout(resolve, 0))
		expect(started).toBe(3)
		expect(thumbQueueStats().waiting).toBe(0)

		for (const release of releases) release()
		await Promise.allSettled(tasks)
	})

	it('needs every hold released before it resumes', async () => {
		let started = 0
		/** @type {Array<() => void>} */
		const releases = []

		pauseThumbQueue()
		pauseThumbQueue()
		const task = enqueueThumbFetch(
			() =>
				new Promise((resolve) => {
					started += 1
					releases.push(() => resolve('ok'))
				}),
		)

		resumeThumbQueue()
		await new Promise((resolve) => setTimeout(resolve, 0))
		// One holder let go, the other did not — still held.
		expect(started).toBe(0)

		resumeThumbQueue()
		await new Promise((resolve) => setTimeout(resolve, 0))
		expect(started).toBe(1)

		for (const release of releases) release()
		await task
	})
})
