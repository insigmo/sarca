import { render, fireEvent, waitFor } from '@solidjs/testing-library'
import { describe, it, expect, vi, beforeEach } from 'vitest'

const { navigate, addAlert, createBackup, restoreBackup } = vi.hoisted(() => ({
	navigate: vi.fn(),
	addAlert: vi.fn(),
	createBackup: vi.fn(),
	restoreBackup: vi.fn(),
}))

vi.mock('@solidjs/router', () => ({
	useNavigate: () => navigate,
}))

vi.mock('../common/nativeBridge', () => ({
	nativeInvoke: vi.fn(),
	pickLocalFolder: vi.fn(),
	isMobileNativePlatform: () => false,
	formatBytes: (n) => String(n),
	describeNativeError: (e) => String(e?.message || e || ''),
}))

vi.mock('../common/nativeClient', () => ({
	nativeClientStore: { isNative: () => false, refresh: () => false },
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
			createBackup: (...args) => createBackup(...args),
			restoreBackup: (...args) => restoreBackup(...args),
		},
	},
}))

import { settingsStore } from '../common/settings'
import SettingsModal from './SettingsModal'

/** @param {HTMLElement} container */
const restoreInput = (container) =>
	/** @type {HTMLInputElement} */ (container.querySelector('.settings-backup__file'))

/**
 * The panel is superuser-only and rendered after `meSilent` resolves.
 * @param {HTMLElement} container
 */
const waitForPanel = (container) =>
	waitFor(() => {
		const el = container.querySelector('.settings-backup')
		if (!el) throw new Error('backup panel not rendered yet')
		return el
	})

/** @param {HTMLElement} container @param {string} label */
const buttonByText = (container, text) =>
	/** @type {HTMLButtonElement} */ (
		[...container.querySelectorAll('button')].find((b) =>
			b.textContent?.includes(text),
		)
	)

/** @param {HTMLElement} container @param {string} text */
const navByText = (container, text) =>
	[...container.querySelectorAll('.settings-nav__title')].find(
		(el) => el.textContent === text,
	)

const fakeBackupFile = () =>
	new File([new Uint8Array([1, 2, 3])], 'sarca-backup.sarcabak')

describe('SettingsModal backup panel', () => {
	beforeEach(() => {
		vi.clearAllMocks()
		createBackup.mockResolvedValue({
			blob: new Blob(['x']),
			filename: 'sarca-backup-20260826-120000.sarcabak',
		})
		restoreBackup.mockResolvedValue({
			tables: 12,
			rows: 340,
			skipped_tables: [],
			safety_copy: '/work/backups/pre-restore.sqlite',
		})
		globalThis.URL.createObjectURL = vi.fn(() => 'blob:backup')
		globalThis.URL.revokeObjectURL = vi.fn()
		settingsStore.openSettings('backup')
	})

	it('sends the typed password with the download request', async () => {
		const { container } = render(() => <SettingsModal />)
		await waitForPanel(container)

		const password = container.querySelector('.settings-backup input[type="password"]')
		fireEvent.input(password, { target: { value: 'rock solid' } })
		fireEvent.click(buttonByText(container, 'Download backup'))

		await waitFor(() => expect(createBackup).toHaveBeenCalledWith('rock solid'))
	})

	// Restore is irreversible, so the button must open the confirmation rather
	// than start wiping the database on the first click.
	it('does not restore until the confirmation is accepted', async () => {
		const { container, baseElement } = render(() => <SettingsModal />)
		await waitForPanel(container)

		const input = restoreInput(container)
		Object.defineProperty(input, 'files', { value: [fakeBackupFile()] })
		fireEvent.change(input)

		await waitFor(() =>
			expect(buttonByText(container, 'Restore from backup').disabled).toBe(false),
		)
		fireEvent.click(buttonByText(container, 'Restore from backup'))
		expect(restoreBackup).not.toHaveBeenCalled()

		const confirm = buttonByText(baseElement, 'Confirm')
		expect(confirm).toBeTruthy()
		fireEvent.click(confirm)

		await waitFor(() => expect(restoreBackup).toHaveBeenCalled())
	})

	// The restored database has its own accounts: staying signed in with a token
	// minted by the replaced one just 401s on the next request.
	it('signs the user out once a restore lands', async () => {
		const { container, baseElement } = render(() => <SettingsModal />)
		await waitForPanel(container)

		const input = restoreInput(container)
		Object.defineProperty(input, 'files', { value: [fakeBackupFile()] })
		fireEvent.change(input)

		await waitFor(() =>
			expect(buttonByText(container, 'Restore from backup').disabled).toBe(false),
		)
		fireEvent.click(buttonByText(container, 'Restore from backup'))
		fireEvent.click(buttonByText(baseElement, 'Confirm'))

		await waitFor(() => expect(navigate).toHaveBeenCalled())
		expect(settingsStore.isOpen()).toBe(false)
	})

	it('cannot be reached by a non-superuser', async () => {
		const api = (await import('../api')).default
		// Not `...Once`: opening the modal and landing on the tab each ask the
		// server who you are, and both answers have to say "not a superuser".
		api.auth.meSilent.mockResolvedValue({
			email: 'someone@example.com',
			is_superuser: false,
		})

		const { container } = render(() => <SettingsModal />)
		// The tab itself is superuser-only: a non-superuser landing on it (deep
		// link, demotion) gets bounced back to General instead of an empty pane.
		await waitFor(() => expect(settingsStore.tab()).toBe('general'))
		expect(navByText(container, 'Backup')).toBeFalsy()
		expect(container.querySelector('.settings-backup')).toBeNull()
	})
})
