import { render, fireEvent, waitFor } from '@solidjs/testing-library'
import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('../common/nativeBridge', () => ({
	nativeInvoke: vi.fn(),
	pickLocalFolder: vi.fn(),
	isMobileNativePlatform: () => false,
	formatBytes: (n) => String(n),
}))

vi.mock('../common/filesChrome', () => ({
	filesChromeStore: {
		storageId: () => 'sid',
		storageName: () => 'S',
	},
}))

vi.mock('./AlertStack', () => ({
	alertStore: { addAlert: vi.fn() },
}))

import { nativeInvoke, pickLocalFolder } from '../common/nativeBridge'
import SettingsSyncPanel from './SettingsSyncPanel'

/**
 * Builds a stateful nativeInvoke mock backed by an in-memory bindings list,
 * mimicking the native command surface used by SettingsSyncPanel.
 * @param {Array<object>} initialBindings
 */
function mockNativeInvoke(initialBindings = []) {
	const state = { bindings: initialBindings.map((b) => ({ ...b })), nextId: 100 }
	nativeInvoke.mockReset()
	nativeInvoke.mockImplementation(async (cmd, args = {}) => {
		switch (cmd) {
			case 'platform_label':
				return ''
			case 'device_label':
				return 'Pixel 8'
			case 'list_bindings':
				return state.bindings.map((b) => ({ ...b }))
			case 'get_client_prefs':
				return {
					wifi_only: true,
					background_sync: true,
					app_lock_enabled: false,
					app_lock_pin: null,
				}
			case 'sync_statuses':
				return []
			case 'sync_transfer_queue':
				return { uploading: 0, downloading: 0, items: [] }
			case 'default_gallery_path':
				return '/pictures'
			case 'ensure_remote_folder':
				return args.parent ? `${args.parent}/${args.name}` : String(args.name || '')
			case 'add_binding': {
				const binding = {
					id: `new-${state.nextId++}`,
					mode: args.mode,
					local_path: args.localPath,
					remote_root: args.remoteRoot,
					enabled: true,
				}
				state.bindings.push(binding)
				return binding
			}
			case 'set_binding_enabled': {
				const b = state.bindings.find((x) => x.id === args.id)
				if (b) b.enabled = args.enabled
				return null
			}
			case 'update_binding_local_path': {
				const b = state.bindings.find((x) => x.id === args.id)
				if (b) b.local_path = args.localPath
				return b ? { ...b } : null
			}
			case 'update_binding_remote_root': {
				const b = state.bindings.find((x) => x.id === args.id)
				if (b) b.remote_root = args.remoteRoot
				return b ? { ...b } : null
			}
			case 'remove_binding': {
				state.bindings = state.bindings.filter((x) => x.id !== args.id)
				return null
			}
			case 'set_client_prefs':
				return null
			case 'sync_now':
				return null
			default:
				return null
		}
	})
	return state
}

const callsFor = (cmd) =>
	nativeInvoke.mock.calls.filter(([name]) => name === cmd)

beforeEach(() => {
	pickLocalFolder.mockReset()
})

describe('SettingsSyncPanel', () => {
	it('enabling with no camera binding adds one and turns background sync on', async () => {
		mockNativeInvoke([])
		const { container } = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		await waitFor(() => expect(callsFor('list_bindings').length).toBeGreaterThan(0))

		const sw = container.querySelector('#settings-camera-switch')
		fireEvent.click(sw)

		await waitFor(() => expect(callsFor('add_binding').length).toBe(1))
		expect(callsFor('add_binding')[0][1]).toMatchObject({
			mode: 'auto_upload',
			remoteRoot: 'Camera/Pixel 8',
		})

		await waitFor(() =>
			expect(
				callsFor('set_client_prefs').some(
					([, a]) => a.prefs?.background_sync === true,
				),
			).toBe(true),
		)
		expect(callsFor('remove_binding').length).toBe(0)
	})

	it('disabling an enabled camera binding soft-disables without removing it', async () => {
		mockNativeInvoke([
			{
				id: '1',
				mode: 'auto_upload',
				enabled: true,
				local_path: '/p',
				remote_root: 'Camera',
			},
		])
		const { container } = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		await waitFor(() => expect(callsFor('list_bindings').length).toBeGreaterThan(0))

		const sw = container.querySelector('#settings-camera-switch')
		await waitFor(() => expect(sw.getAttribute('aria-checked')).toBe('true'))
		fireEvent.click(sw)

		await waitFor(() => expect(callsFor('set_binding_enabled').length).toBe(1))
		expect(callsFor('set_binding_enabled')[0][1]).toEqual({
			id: '1',
			enabled: false,
		})
		expect(callsFor('remove_binding').length).toBe(0)
		expect(callsFor('add_binding').length).toBe(0)
	})

	it('re-enabling updates legacy Camera root to device subfolder', async () => {
		mockNativeInvoke([
			{
				id: '1',
				mode: 'auto_upload',
				enabled: false,
				local_path: '/p',
				remote_root: 'Camera',
			},
		])
		const { container } = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		await waitFor(() => expect(callsFor('list_bindings').length).toBeGreaterThan(0))

		const sw = container.querySelector('#settings-camera-switch')
		await waitFor(() => expect(sw.getAttribute('aria-checked')).toBe('false'))
		fireEvent.click(sw)

		await waitFor(() => expect(callsFor('set_binding_enabled').length).toBe(1))
		await waitFor(() =>
			expect(callsFor('update_binding_remote_root').length).toBeGreaterThan(0),
		)
		const updates = callsFor('update_binding_remote_root')
		expect(updates[updates.length - 1][1]).toEqual({
			id: '1',
			remoteRoot: 'Camera/Pixel 8',
		})
	})

	it('re-enabling a disabled camera binding uses set_binding_enabled, not add_binding', async () => {
		mockNativeInvoke([
			{
				id: '1',
				mode: 'auto_upload',
				enabled: false,
				local_path: '/p',
				remote_root: 'Camera',
			},
		])
		const { container } = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		await waitFor(() => expect(callsFor('list_bindings').length).toBeGreaterThan(0))

		const sw = container.querySelector('#settings-camera-switch')
		await waitFor(() => expect(sw.getAttribute('aria-checked')).toBe('false'))
		fireEvent.click(sw)

		await waitFor(() => expect(callsFor('set_binding_enabled').length).toBe(1))
		expect(callsFor('set_binding_enabled')[0][1]).toEqual({
			id: '1',
			enabled: true,
		})
		expect(callsFor('add_binding').length).toBe(0)
	})

	it('changing the local folder for an existing binding updates its path', async () => {
		mockNativeInvoke([
			{
				id: '1',
				mode: 'auto_upload',
				enabled: true,
				local_path: '/old',
				remote_root: 'Camera',
			},
		])
		pickLocalFolder.mockResolvedValue('/new')
		const { getByText, queryByText } = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		await waitFor(() =>
			expect(queryByText('Change local folder')).not.toBeNull(),
		)

		fireEvent.click(getByText('Change local folder'))

		await waitFor(() => expect(callsFor('update_binding_local_path').length).toBe(1))
		expect(callsFor('update_binding_local_path')[0][1]).toEqual({
			id: '1',
			localPath: '/new',
		})
		expect(callsFor('add_binding').length).toBe(0)
		expect(callsFor('set_binding_enabled').length).toBe(0)
	})

	it('toggling a folder binding row calls set_binding_enabled', async () => {
		mockNativeInvoke([
			{
				id: 'f1',
				mode: 'folder_upload',
				enabled: true,
				local_path: '/f',
				remote_root: 'R',
			},
		])
		const { container } = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		await waitFor(() =>
			expect(
				container.querySelector('#settings-folder-switch-f1'),
			).not.toBeNull(),
		)

		const sw = container.querySelector('#settings-folder-switch-f1')
		fireEvent.click(sw)

		await waitFor(() => expect(callsFor('set_binding_enabled').length).toBe(1))
		expect(callsFor('set_binding_enabled')[0][1]).toEqual({
			id: 'f1',
			enabled: false,
		})
	})

	it('migrates legacy Camera root to per-device folder on refresh', async () => {
		mockNativeInvoke([
			{
				id: '1',
				mode: 'auto_upload',
				enabled: true,
				local_path: '/p',
				remote_root: 'Camera',
			},
		])
		render(() => <SettingsSyncPanel storageId="sid" storageName="Test" />)
		await waitFor(() =>
			expect(callsFor('update_binding_remote_root').length).toBe(1),
		)
		expect(callsFor('update_binding_remote_root')[0][1]).toEqual({
			id: '1',
			remoteRoot: 'Camera/Pixel 8',
		})
	})

	it('migrates Camera/Unknown device placeholder to resolved device folder', async () => {
		mockNativeInvoke([
			{
				id: '1',
				mode: 'auto_upload',
				enabled: true,
				local_path: '/p',
				remote_root: 'Camera/Unknown device',
			},
		])
		render(() => <SettingsSyncPanel storageId="sid" storageName="Test" />)
		await waitFor(() =>
			expect(callsFor('update_binding_remote_root').length).toBe(1),
		)
		expect(callsFor('update_binding_remote_root')[0][1]).toEqual({
			id: '1',
			remoteRoot: 'Camera/Pixel 8',
		})
	})

	it('does not show Unknown device before labels resolve', async () => {
		let resolveDevice
		const devicePromise = new Promise((r) => {
			resolveDevice = r
		})
		const state = mockNativeInvoke([
			{
				id: '1',
				mode: 'auto_upload',
				enabled: true,
				local_path: '/p',
				remote_root: 'Camera/Pixel 8',
			},
		])
		nativeInvoke.mockImplementation(async (cmd, args = {}) => {
			if (cmd === 'device_label') return devicePromise
			if (cmd === 'platform_label') return devicePromise.then(() => 'Android')
			switch (cmd) {
				case 'list_bindings':
					return state.bindings.map((b) => ({ ...b }))
				case 'get_client_prefs':
					return {
						wifi_only: true,
						background_sync: true,
						app_lock_enabled: false,
						app_lock_pin: null,
					}
				case 'sync_statuses':
					return []
				case 'sync_transfer_queue':
					return { uploading: 0, downloading: 0, items: [] }
				default:
					return null
			}
		})

		const { container } = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		await waitFor(() =>
			expect(
				container
					.querySelector('#settings-camera-switch')
					?.getAttribute('aria-checked'),
			).toBe('true'),
		)
		expect(container.textContent).not.toContain('Unknown device')

		resolveDevice('Pixel 8')
		await waitFor(() => expect(container.textContent).toContain('Camera/Pixel 8'))
	})

	it('add_binding uses a fresh native device_label even if UI signal is empty', async () => {
		mockNativeInvoke([])
		const { container } = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		await waitFor(() => expect(callsFor('list_bindings').length).toBeGreaterThan(0))

		const sw = container.querySelector('#settings-camera-switch')
		fireEvent.click(sw)

		await waitFor(() => expect(callsFor('add_binding').length).toBe(1))
		expect(callsFor('add_binding')[0][1]).toMatchObject({
			mode: 'auto_upload',
			remoteRoot: 'Camera/Pixel 8',
		})
		expect(
			callsFor('device_label').length,
		).toBeGreaterThanOrEqual(2)
	})

	it('shows Downloading/Uploading rows and opens upload list', async () => {
		const state = mockNativeInvoke([])
		nativeInvoke.mockImplementation(async (cmd) => {
			switch (cmd) {
				case 'platform_label':
					return ''
				case 'device_label':
					return 'Pixel 8'
				case 'list_bindings':
					return state.bindings.map((b) => ({ ...b }))
				case 'get_client_prefs':
					return {
						wifi_only: true,
						background_sync: true,
						app_lock_enabled: false,
						app_lock_pin: null,
					}
				case 'sync_statuses':
					return []
				case 'sync_transfer_queue':
					return {
						uploading: 1,
						downloading: 0,
						items: [
							{
								id: 't1',
								binding_id: 'b1',
								direction: 'upload',
								path: 'Camera',
								name: 'a.jpg',
								size: 12,
								status: 'active',
								updated_at_ms: 1,
							},
						],
					}
				case 'default_gallery_path':
					return '/pictures'
				default:
					return null
			}
		})

		const { container, getByText, findByText } = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		await waitFor(() =>
			expect(callsFor('sync_transfer_queue').length).toBeGreaterThan(0),
		)
		expect(getByText('Downloading')).toBeTruthy()
		expect(getByText('Uploading')).toBeTruthy()
		const uploadRow = [
			...container.querySelectorAll('.settings-sync-panel__queue-row'),
		].find((el) => el.textContent.includes('Uploading'))
		expect(uploadRow?.textContent).toContain('1')
		fireEvent.click(uploadRow)
		expect(await findByText('Upload list')).toBeTruthy()
		expect(await findByText('a.jpg')).toBeTruthy()
	})
})
