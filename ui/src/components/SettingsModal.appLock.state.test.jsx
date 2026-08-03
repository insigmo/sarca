import { render, fireEvent, waitFor } from '@solidjs/testing-library'
import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('@solidjs/router', () => ({
	useNavigate: () => () => {},
}))

vi.mock('../common/nativeBridge', () => ({
	nativeInvoke: vi.fn(),
	pickLocalFolder: vi.fn(),
	isMobileNativePlatform: () => false,
	formatBytes: (n) => String(n),
}))

vi.mock('../common/nativeClient', () => ({
	nativeClientStore: { isNative: () => true, refresh: () => true },
}))

vi.mock('../common/filesChrome', () => ({
	filesChromeStore: { storageId: () => '', storageName: () => '' },
}))

vi.mock('../common/storageSettings', () => ({
	storageSettingsStore: { open: vi.fn() },
}))

vi.mock('./AlertStack', () => ({
	alertStore: { addAlert: vi.fn() },
}))

vi.mock('../api', () => ({
	default: {
		storages: { listStorages: vi.fn().mockResolvedValue({ storages: [] }) },
		auth: { meSilent: vi.fn().mockResolvedValue(null) },
	},
}))

import { nativeInvoke } from '../common/nativeBridge'
import { settingsStore } from '../common/settings'
import SettingsModal from './SettingsModal'

/** @param {{ app_lock_enabled?: boolean, app_lock_pin_set?: boolean }} [prefs] */
function mockNativeInvoke(prefs = {}) {
	nativeInvoke.mockReset()
	nativeInvoke.mockImplementation(async (cmd) => {
		switch (cmd) {
			case 'get_client_prefs':
				// Mirrors ClientPrefsDto: the PIN itself is never readable, only
				// the flag saying one exists.
				return {
					app_lock_enabled: false,
					app_lock_pin_set: false,
					...prefs,
				}
			default:
				return null
		}
	})
}

describe('SettingsModal app lock state machine', () => {
	beforeEach(() => {
		mockNativeInvoke()
		settingsStore.openSettings('security')
	})

	it('does not call native set_client_prefs when cancelling app lock mid-PIN-entry', async () => {
		const { getByRole } = render(() => <SettingsModal />)
		const sw = await waitFor(() => getByRole('switch'))
		await waitFor(() => expect(sw).toHaveAttribute('aria-checked', 'false'))

		// Turn on: only local "entering PIN" state, nothing persisted yet.
		fireEvent.click(sw)
		expect(sw).toHaveAttribute('aria-checked', 'true')
		expect(nativeInvoke).not.toHaveBeenCalledWith(
			'set_client_prefs',
			expect.anything(),
		)

		// Cancel before saving: switch goes back off, still no native call.
		fireEvent.click(sw)
		expect(sw).toHaveAttribute('aria-checked', 'false')
		expect(nativeInvoke).not.toHaveBeenCalledWith(
			'set_client_prefs',
			expect.anything(),
		)
	})

	it('reflects a previously persisted lock as checked on load', async () => {
		mockNativeInvoke({ app_lock_enabled: true, app_lock_pin_set: true })
		const { getByRole } = render(() => <SettingsModal />)
		const sw = await waitFor(() => getByRole('switch'))
		await waitFor(() => expect(sw).toHaveAttribute('aria-checked', 'true'))
	})

	// Disabling the lock used to compare the typed PIN against a plaintext
	// `app_lock_pin` handed out by get_client_prefs. Now the current PIN goes
	// to Rust, which verifies it against a salted hash and refuses on mismatch.
	it('sends the current PIN to Rust when disabling a persisted lock', async () => {
		mockNativeInvoke({ app_lock_enabled: true, app_lock_pin_set: true })
		const { getByRole, getByLabelText, findByText } = render(() => (
			<SettingsModal />
		))
		const sw = await waitFor(() => getByRole('switch'))
		await waitFor(() => expect(sw).toHaveAttribute('aria-checked', 'true'))

		// No current PIN typed: refused locally, nothing persisted.
		fireEvent.click(sw)
		expect(await findByText('Enter your current PIN')).toBeInTheDocument()
		expect(nativeInvoke).not.toHaveBeenCalledWith(
			'set_client_prefs',
			expect.anything(),
		)

		const current = await waitFor(() => getByLabelText('Current PIN'))
		// SUID's InputBase listens for `input`, not `change`.
		fireEvent.input(current, { target: { value: '1234' } })
		fireEvent.click(sw)

		await waitFor(() =>
			expect(nativeInvoke).toHaveBeenCalledWith('set_client_prefs', {
				prefs: expect.objectContaining({
					app_lock_enabled: false,
					app_lock_pin: null,
					current_app_lock_pin: '1234',
				}),
			}),
		)
	})
})
