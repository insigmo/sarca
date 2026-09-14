import { render, fireEvent, waitFor } from '@solidjs/testing-library'
import { describe, it, expect, vi, beforeEach } from 'vitest'

const { navigate, addAlert, nativeInvoke } = vi.hoisted(() => ({
	navigate: vi.fn(),
	addAlert: vi.fn(),
	nativeInvoke: vi.fn(),
}))

vi.mock('@solidjs/router', () => ({
	useNavigate: () => navigate,
}))

vi.mock('../common/nativeBridge', () => ({
	nativeInvoke: (...args) => nativeInvoke(...args),
	pickLocalFolder: vi.fn(),
	isMobileNativePlatform: () => false,
	formatBytes: (n) => `${n} B`,
	describeNativeError: (e) => String(e?.message || e || ''),
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
	alertStore: { addAlert },
}))

vi.mock('../api', () => ({
	default: {
		storages: { listStorages: vi.fn().mockResolvedValue({ storages: [] }) },
		auth: {
			meSilent: vi
				.fn()
				.mockResolvedValue({ email: 'root@example.com', is_superuser: true }),
			logout: vi.fn(),
		},
		settings: {
			getTrashSettings: vi.fn().mockResolvedValue({ retention_days: 30 }),
			getServerVersion: vi.fn().mockResolvedValue({ version: '0.0.170' }),
			checkServerUpdate: vi.fn(),
			applyServerUpdate: vi.fn(),
		},
	},
}))

import { settingsStore } from '../common/settings'
import SettingsModal from './SettingsModal'

/** @param {HTMLElement} container @param {string} text */
const buttonByText = (container, text) =>
	/** @type {HTMLButtonElement} */ (
		[...container.querySelectorAll('button')].find((b) =>
			b.textContent?.includes(text),
		)
	)

/** @param {HTMLElement} container @param {string} id */
const switchById = (container, id) =>
	/** @type {HTMLButtonElement} */ (container.querySelector(`#${id}`))

/** Prefs as the native side would answer them, plus whatever the test overrides. */
const prefs = (over = {}) => ({
	wifi_only: true,
	app_lock_enabled: true,
	app_lock_pin_set: true,
	enable_logs: true,
	log_level: 'info',
	cache_limit_bytes: 1_073_741_824,
	...over,
})

describe('SettingsModal logs tab', () => {
	beforeEach(() => {
		vi.clearAllMocks()
		nativeInvoke.mockImplementation(async (cmd) => {
			switch (cmd) {
				case 'get_client_prefs':
					return prefs()
				case 'get_log_status':
					return {
						enabled: true,
						level: 'info',
						path: '/data/logs/sarca-client.log',
						size_bytes: 2048,
					}
				case 'set_client_prefs':
					return prefs()
				case 'clear_logs':
					return {
						enabled: true,
						level: 'info',
						path: '/data/logs/sarca-client.log',
						size_bytes: 0,
					}
				default:
					return null
			}
		})
		settingsStore.openSettings('logs')
	})

	it('shows the log file, its size and the current level', async () => {
		const { container } = render(() => <SettingsModal />)

		await waitFor(() =>
			expect(container.textContent).toContain('/data/logs/sarca-client.log'),
		)
		expect(container.textContent).toContain('2048 B on disk')
		// The level is spelled out rather than only implied by the switch: the
		// user asked for INFO by default, and "by default" is only visible if
		// the screen says which level is in force.
		expect(container.textContent).toContain('INFO')
	})

	// The point of the checkbox. `set_client_prefs` replaces the whole object,
	// so the write has to carry every other pref forward — sending only
	// `log_level` would turn the app lock off as a side effect of asking for
	// debug logs.
	it('advanced logging switches the level to debug without dropping other prefs', async () => {
		const { container } = render(() => <SettingsModal />)
		await waitFor(() =>
			expect(switchById(container, 'settings-advanced-logging-switch')).toBeTruthy(),
		)

		fireEvent.click(switchById(container, 'settings-advanced-logging-switch'))

		await waitFor(() =>
			expect(nativeInvoke).toHaveBeenCalledWith('set_client_prefs', {
				prefs: expect.objectContaining({
					log_level: 'debug',
					app_lock_enabled: true,
					enable_logs: true,
					wifi_only: true,
				}),
			}),
		)
	})

	it('defaults to info when the native side reports an unknown level', async () => {
		nativeInvoke.mockImplementation(async (cmd) =>
			cmd === 'get_log_status'
				? { enabled: true, level: 'shout', path: '/x.log', size_bytes: 0 }
				: prefs(),
		)
		const { container } = render(() => <SettingsModal />)

		await waitFor(() => expect(container.textContent).toContain('INFO'))
		expect(
			switchById(container, 'settings-advanced-logging-switch').getAttribute(
				'aria-checked',
			),
		).toBe('false')
	})

	it('clears the log and reports the new size', async () => {
		const { container } = render(() => <SettingsModal />)
		await waitFor(() => expect(buttonByText(container, 'Clear log')).toBeTruthy())

		fireEvent.click(buttonByText(container, 'Clear log'))

		await waitFor(() => expect(nativeInvoke).toHaveBeenCalledWith('clear_logs'))
		await waitFor(() => expect(container.textContent).toContain('0 B on disk'))
	})
})

describe('SettingsModal about tab', () => {
	beforeEach(() => {
		vi.clearAllMocks()
		nativeInvoke.mockImplementation(async (cmd) => {
			switch (cmd) {
				case 'get_about':
					return { version: '0.0.170', platform: 'Windows' }
				case 'get_client_prefs':
					return prefs()
				case 'check_client_update':
					return {
						current: '0.0.170',
						latest: 'v0.0.171',
						update_available: true,
						notes: 'Fixes auto-upload',
						asset: 'sarca_client_windows_amd64-setup.exe',
						download_url: 'https://example.invalid/setup.exe',
						can_install: true,
						reason: null,
					}
				case 'install_client_update':
					return { version: 'v0.0.171', path: 'C:/data/updates/x.exe', launched: true }
				default:
					return null
			}
		})
		settingsStore.openSettings('about')
	})

	it('shows both versions without anyone pressing a button', async () => {
		const { container } = render(() => <SettingsModal />)

		// Client version and platform come from the native bridge, the server's
		// from the API — both without a button press, because "what am I
		// running" is the question the tab exists to answer.
		await waitFor(() =>
			expect(container.textContent).toContain('0.0.170 · Windows'),
		)
		expect(container.textContent).toContain('Server')
		expect(container.textContent).toContain('0.0.170')
	})

	// Nothing is downloaded on a check, and the Update button must not exist
	// until a check has actually found something.
	it('offers the update only after a check finds one', async () => {
		const { container } = render(() => <SettingsModal />)
		await waitFor(() =>
			expect(buttonByText(container, 'Check for updates')).toBeTruthy(),
		)
		expect(buttonByText(container, 'Update')).toBeFalsy()

		fireEvent.click(buttonByText(container, 'Check for updates'))

		await waitFor(() =>
			expect(container.textContent).toContain('v0.0.171 is available'),
		)
		expect(nativeInvoke).not.toHaveBeenCalledWith('install_client_update')

		fireEvent.click(buttonByText(container, 'Update'))
		await waitFor(() =>
			expect(nativeInvoke).toHaveBeenCalledWith('install_client_update'),
		)
	})

	// A platform that cannot start its own installer (Android, iOS) must not
	// show a button that would do nothing.
	it('hides the update button when this platform cannot install it', async () => {
		nativeInvoke.mockImplementation(async (cmd) => {
			if (cmd === 'get_about') return { version: '0.0.170', platform: 'Android' }
			if (cmd === 'check_client_update') {
				return {
					current: '0.0.170',
					latest: 'v0.0.171',
					update_available: true,
					notes: '',
					asset: 'sarca_client_android_arm64.apk',
					download_url: 'https://example.invalid/app.apk',
					can_install: false,
					reason: 'install the downloaded APK from your notifications',
				}
			}
			return prefs()
		})
		const { container } = render(() => <SettingsModal />)
		await waitFor(() =>
			expect(buttonByText(container, 'Check for updates')).toBeTruthy(),
		)

		fireEvent.click(buttonByText(container, 'Check for updates'))

		await waitFor(() =>
			expect(container.textContent).toContain('v0.0.171 is available'),
		)
		expect(buttonByText(container, 'Update')).toBeFalsy()
	})
})
