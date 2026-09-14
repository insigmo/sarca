import { render, fireEvent, waitFor } from '@solidjs/testing-library'
import { describe, it, expect, vi, beforeEach } from 'vitest'

const { navigate, addAlert, listUsers, deleteUser } = vi.hoisted(() => ({
	navigate: vi.fn(),
	addAlert: vi.fn(),
	listUsers: vi.fn(),
	deleteUser: vi.fn(),
}))

// The Access tab pulls in GrantAccess, which reads route params of its own.
vi.mock('@solidjs/router', () => ({
	useNavigate: () => navigate,
	useParams: () => ({}),
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
		access: { listUsersWithAccess: vi.fn().mockResolvedValue([]) },
		auth: {
			meSilent: vi
				.fn()
				.mockResolvedValue({ email: 'root@example.com', is_superuser: true }),
			logout: vi.fn(),
		},
		users: {
			listUsers: (...args) => listUsers(...args),
			deleteUser: (...args) => deleteUser(...args),
		},
		settings: {
			getTrashSettings: vi.fn().mockResolvedValue({ retention_days: 30 }),
		},
	},
}))

import { settingsStore } from '../common/settings'
import SettingsModal from './SettingsModal'

/** @param {HTMLElement} root @param {string} text */
const buttonByText = (root, text) =>
	/** @type {HTMLButtonElement} */ (
		[...root.querySelectorAll('button')].find((b) => b.textContent?.includes(text))
	)

/** @param {HTMLElement} root @param {string|RegExp} label */
const buttonByLabel = (root, label) =>
	/** @type {HTMLButtonElement} */ (
		[...root.querySelectorAll('button')].find((b) =>
			label instanceof RegExp
				? label.test(b.getAttribute('aria-label') || '')
				: b.getAttribute('aria-label') === label,
		)
	)

/** The account list only renders once `meSilent` confirms a superuser. */
const waitForRows = (container) =>
	waitFor(() => {
		const rows = container.querySelectorAll('.settings-users__row')
		if (!rows.length) throw new Error('account rows not rendered yet')
		return rows
	})

describe('SettingsModal account deletion', () => {
	beforeEach(() => {
		vi.clearAllMocks()
		localStorage.clear()
		localStorage.setItem(
			'user',
			JSON.stringify({ email: 'root@example.com', is_superuser: true }),
		)
		listUsers.mockResolvedValue({
			users: [
				{
					id: 'root-id',
					email: 'root@example.com',
					email_verified: true,
					is_superuser: true,
					disabled: false,
				},
				{
					id: 'user-id',
					email: 'alice@example.com',
					email_verified: true,
					is_superuser: false,
					disabled: false,
				},
			],
		})
		deleteUser.mockResolvedValue(undefined)
		settingsStore.openSettings('access')
	})

	// Deleting an account also wipes storages nobody else can reach, so the
	// first click must only ask, never delete.
	it('asks for confirmation before deleting, then deletes on confirm', async () => {
		const { container, baseElement } = render(() => <SettingsModal />)
		await waitForRows(container)

		fireEvent.click(buttonByLabel(container, 'Delete the account alice@example.com'))
		expect(deleteUser).not.toHaveBeenCalled()

		const confirm = buttonByText(baseElement, 'Confirm')
		expect(confirm).toBeTruthy()
		fireEvent.click(confirm)

		await waitFor(() => expect(deleteUser).toHaveBeenCalledWith('user-id'))
		// The list is reloaded so the deleted row disappears.
		await waitFor(() => expect(listUsers).toHaveBeenCalledTimes(2))
	})

	it('deletes nothing when the confirmation is cancelled', async () => {
		const { container, baseElement } = render(() => <SettingsModal />)
		await waitForRows(container)

		fireEvent.click(buttonByLabel(container, 'Delete the account alice@example.com'))
		fireEvent.click(buttonByText(baseElement, 'Cancel'))

		await waitFor(() =>
			expect(buttonByText(baseElement, 'Confirm')).toBeFalsy(),
		)
		expect(deleteUser).not.toHaveBeenCalled()
	})

	// The server answers 403 for both, so the buttons must not invite the click.
	it('disables delete for the superuser row and for the caller', async () => {
		const { container } = render(() => <SettingsModal />)
		await waitForRows(container)

		expect(
			buttonByLabel(container, 'Delete the account root@example.com').disabled,
		).toBe(true)
		expect(
			buttonByLabel(container, 'Delete the account alice@example.com').disabled,
		).toBe(false)
	})
})
