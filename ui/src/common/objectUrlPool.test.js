import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const { acquireObjectUrl, releaseObjectUrl, resetObjectUrlPool } = await import(
	'./objectUrlPool'
)

describe('objectUrlPool', () => {
	let createSpy
	let revokeSpy
	let urlCounter

	beforeEach(() => {
		vi.useFakeTimers()
		urlCounter = 0
		// jsdom does not implement these, so there is nothing to spy on yet —
		// stub them onto URL first.
		if (!URL.createObjectURL) URL.createObjectURL = () => ''
		if (!URL.revokeObjectURL) URL.revokeObjectURL = () => {}
		createSpy = vi
			.spyOn(URL, 'createObjectURL')
			.mockImplementation(() => `blob:mock-${urlCounter++}`)
		revokeSpy = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {})
	})

	afterEach(() => {
		resetObjectUrlPool()
		vi.useRealTimers()
		createSpy.mockRestore()
		revokeSpy.mockRestore()
	})

	it('reuses the same URL while a key is still referenced', () => {
		const blob = new Blob(['a'])
		const first = acquireObjectUrl('scope:path', blob)
		const second = acquireObjectUrl('scope:path', blob)
		expect(second).toBe(first)
		expect(createSpy).toHaveBeenCalledTimes(1)
	})

	it('revokes the URL only after the last release and its grace window elapses', () => {
		const blob = new Blob(['a'])
		acquireObjectUrl('scope:path', blob)
		acquireObjectUrl('scope:path', blob)

		releaseObjectUrl('scope:path')
		expect(revokeSpy).not.toHaveBeenCalled()

		// Still one live reference; must not revoke.
		vi.advanceTimersByTime(10000)
		expect(revokeSpy).not.toHaveBeenCalled()

		releaseObjectUrl('scope:path')
		expect(revokeSpy).not.toHaveBeenCalled()

		vi.advanceTimersByTime(5000)
		expect(revokeSpy).toHaveBeenCalledTimes(1)
	})

	it('does not revoke when re-acquired within the deferred window', () => {
		const blob = new Blob(['a'])
		const url = acquireObjectUrl('scope:path', blob)
		releaseObjectUrl('scope:path')

		vi.advanceTimersByTime(2000)
		const reacquired = acquireObjectUrl('scope:path', blob)
		expect(reacquired).toBe(url)
		expect(revokeSpy).not.toHaveBeenCalled()

		// The old timer must have been cancelled, not just outraced.
		vi.advanceTimersByTime(5000)
		expect(revokeSpy).not.toHaveBeenCalled()

		releaseObjectUrl('scope:path')
		vi.advanceTimersByTime(5000)
		expect(revokeSpy).toHaveBeenCalledTimes(1)
	})

	it('creates a fresh URL per distinct key', () => {
		const blob = new Blob(['a'])
		const a = acquireObjectUrl('scope:a', blob)
		const b = acquireObjectUrl('scope:b', blob)
		expect(a).not.toBe(b)
		expect(createSpy).toHaveBeenCalledTimes(2)
	})
})
