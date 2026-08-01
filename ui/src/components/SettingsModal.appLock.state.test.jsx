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

/** @param {{ app_lock_enabled?: boolean }} [prefs] */
function mockNativeInvoke(prefs = {}) {
	nativeInvoke.mockReset()
	nativeInvoke.mockImplementation(async (cmd) => {
		switch (cmd) {
			case 'get_client_prefs':
				return { app_lock_enabled: false, app_lock_pin: null, ...prefs }
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
		mockNativeInvoke({ app_lock_enabled: true })
		const { getByRole } = render(() => <SettingsModal />)
		const sw = await waitFor(() => getByRole('switch'))
		await waitFor(() => expect(sw).toHaveAttribute('aria-checked', 'true'))
	})
})
