import { describe, it, expect } from 'vitest'
import {
	enqueueThumbFetch,
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
})
