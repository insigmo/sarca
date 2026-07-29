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
})
