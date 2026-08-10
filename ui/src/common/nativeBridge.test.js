import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

import { nativeInvoke, describeNativeError } from './nativeBridge'

describe('nativeInvoke bridge warm-up', () => {
	beforeEach(() => {
		delete window.__sarcaInvoke
		delete window.__TAURI_INTERNALS__
	})

	afterEach(() => {
		vi.useRealTimers()
		delete window.__sarcaInvoke
		delete window.__TAURI_INTERNALS__
	})

	// Regression: opening a server flashed "Command … not allowed by ACL".
	// The native side grants the origin its capability immediately before
	// navigating, but the grant is not live the instant the page's onMount
	// handlers fire, so the first command could be refused and every caller
	// had to grow its own retry loop.
	it('retries an ACL refusal until the capability lands', async () => {
		let calls = 0
		window.__sarcaInvoke = vi.fn(() => {
			calls += 1
			if (calls < 3) {
				return Promise.reject(new Error('Command get_client_prefs not allowed by ACL'))
			}
			return Promise.resolve('ok')
		})

		await expect(nativeInvoke('get_client_prefs')).resolves.toBe('ok')
		expect(calls).toBe(3)
	})

	it('surfaces a real command error on the first attempt', async () => {
		window.__sarcaInvoke = vi.fn(() => Promise.reject(new Error('PIN is incorrect')))

		await expect(nativeInvoke('verify_app_lock_pin')).rejects.toThrow('PIN is incorrect')
		expect(window.__sarcaInvoke).toHaveBeenCalledTimes(1)
	})
})

describe('describeNativeError', () => {
	it('hides bridge noise behind an actionable message', () => {
		const acl = new Error('Command disconnect not allowed by ACL')
		expect(describeNativeError(acl)).not.toContain('ACL')
	})

	it('passes real errors through untouched', () => {
		expect(describeNativeError(new Error('Folder is not readable'))).toBe(
			'Folder is not readable',
		)
	})
})
