import { render, fireEvent, waitFor } from '@solidjs/testing-library'
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
			app_lock_pin_set: true,
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
	//
	// The retry now lives in `nativeInvoke` itself, so every caller gets it
	// (see nativeBridge.test.js) and this component issues exactly one call.
	// What must still hold here: a bridge that comes up late still ends with
	// the app locked, and the component adds no retry loop of its own on top
	// of the bridge's — nesting the two multiplied the timeouts.
	it('locks once the late-injected bridge answers, without its own retry loop', async () => {
		nativeInvoke.mockResolvedValue({ app_lock_enabled: true, app_lock_pin_set: true })

		const { findByLabelText } = render(() => (
			<AppLockGate>
				<div>app content</div>
			</AppLockGate>
		))

		expect(await findByLabelText('App lock')).toBeInTheDocument()
		expect(nativeInvoke.mock.calls.filter(([cmd]) => cmd === 'get_client_prefs')).toHaveLength(
			1,
		)
	})

	// A bridge that never comes up leaves the app usable rather than wedged
	// behind a PIN prompt it can never verify.
	it('does not wedge the app when the bridge never comes up', async () => {
		nativeInvoke.mockRejectedValue(new Error('Native bridge unavailable'))

		const { queryByLabelText } = render(() => (
			<AppLockGate>
				<div>app content</div>
			</AppLockGate>
		))

		await waitFor(() => expect(nativeInvoke).toHaveBeenCalled())
		await Promise.resolve()
		expect(queryByLabelText('App lock')).not.toBeInTheDocument()
	})

	// The PIN used to be returned by get_client_prefs and compared here, so
	// anything that could reach the bridge could read it and unlock the app.
	// Unlocking must go through the native verify command instead.
	it('verifies the PIN natively and never reads it from prefs', async () => {
		nativeInvoke.mockImplementation((cmd, args) => {
			if (cmd === 'get_client_prefs') {
				return Promise.resolve({
					app_lock_enabled: true,
					app_lock_pin_set: true,
				})
			}
			if (cmd === 'verify_app_lock_pin') {
				return Promise.resolve(args?.pin === '1234')
			}
			throw new Error(`unexpected command ${cmd}`)
		})

		const { findByLabelText, getByLabelText, getByRole, findByText, queryByLabelText } =
			render(() => (
				<AppLockGate>
					<div>app content</div>
				</AppLockGate>
			))

		await findByLabelText('App lock')
		const input = getByLabelText('PIN')
		const unlock = getByRole('button', { name: 'Unlock' })

		// SUID's InputBase listens for `input`, not `change`.
		fireEvent.input(input, { target: { value: '9999' } })
		fireEvent.click(unlock)
		expect(await findByText('Incorrect PIN')).toBeInTheDocument()

		fireEvent.input(input, { target: { value: '1234' } })
		fireEvent.click(unlock)
		await waitFor(() =>
			expect(queryByLabelText('App lock')).not.toBeInTheDocument(),
		)

		const verified = nativeInvoke.mock.calls.filter(
			([cmd]) => cmd === 'verify_app_lock_pin',
		)
		expect(verified.length).toBe(2)
		expect(sessionStorage.getItem('sarca_unlocked')).toBe('1')
	})
})
