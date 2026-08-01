import { render } from '@solidjs/testing-library'
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

vi.mock('../common/nativeClient', () => ({
	nativeClientStore: { isNative: () => true },
}))

const nativeInvoke = vi.fn()
vi.mock('../common/nativeBridge', () => ({
	nativeInvoke: (...args) => nativeInvoke(...args),
}))

import AppLockGate from './AppLockGate'

describe('AppLockGate', () => {
	beforeEach(() => {
		nativeInvoke.mockReset()
		sessionStorage.clear()
	})

	afterEach(() => {
		vi.useRealTimers()
	})

	it('shows the PIN prompt once prefs report app lock enabled', async () => {
		nativeInvoke.mockResolvedValue({
			app_lock_enabled: true,
			app_lock_pin: '1234',
		})

		const { findByLabelText } = render(() => (
			<AppLockGate>
				<div>app content</div>
			</AppLockGate>
		))

		expect(await findByLabelText('App lock')).toBeInTheDocument()
	})

	it('does not lock when app lock is disabled in prefs', async () => {
		nativeInvoke.mockResolvedValue({ app_lock_enabled: false })

		const { queryByLabelText } = render(() => (
			<AppLockGate>
				<div>app content</div>
			</AppLockGate>
		))

		// Give the microtask queue a turn to resolve the mocked invoke.
		await Promise.resolve()
		await Promise.resolve()
		expect(queryByLabelText('App lock')).not.toBeInTheDocument()
	})

	// Regression: nativeClientStore.isNative() can read true from a stale
	// localStorage flag before the real native bridge is actually injected
	// (documented race in nativeClient.js — Android WebView late-inject).
	// The old code treated the *first* nativeInvoke('get_client_prefs')
	// rejection as "we must be in a plain browser, skip the lock" and gave
	// up permanently for the whole session — silently bypassing app lock.
	// It must instead retry until the bridge comes up.
	it('retries get_client_prefs instead of permanently bypassing the lock on a transient bridge failure', async () => {
		nativeInvoke
			.mockRejectedValueOnce(new Error('Native bridge unavailable'))
			.mockRejectedValueOnce(new Error('Native bridge unavailable'))
			.mockResolvedValue({ app_lock_enabled: true, app_lock_pin: '1234' })

		const { findByLabelText } = render(() => (
			<AppLockGate>
				<div>app content</div>
			</AppLockGate>
		))

		expect(await findByLabelText('App lock')).toBeInTheDocument()
		expect(nativeInvoke.mock.calls.length).toBeGreaterThanOrEqual(3)
	})
})
