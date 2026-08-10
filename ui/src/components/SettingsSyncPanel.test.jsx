import { render, fireEvent, waitFor } from '@solidjs/testing-library'
import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('../common/nativeBridge', () => ({
	nativeInvoke: vi.fn(),
	pickLocalFolder: vi.fn(),
	isMobileNativePlatform: () => false,
	formatBytes: (n) => String(n),
	describeNativeError: (e) => String(e?.message || e || ''),
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
import { syncSettingsStore } from '../common/syncSettingsStore'
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
					app_lock_enabled: false,
					app_lock_pin_set: false,
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
	for (const store of [localStorage, sessionStorage]) {
		try {
			store.removeItem('sarca.client.cameraAutoUploadEnabled')
		} catch {
			// ignore
		}
	}
	// Sync state is a module singleton on purpose (it survives Settings being
	// closed), so each test has to start it from scratch — after the cache
	// keys above are gone, since reset() re-reads them.
	syncSettingsStore.reset()
})

describe('SettingsSyncPanel', () => {
	it('flips the switch before the native work finishes', async () => {
		// The enable path does several IPC round trips (list_bindings,
		// ensure_remote_folder over the network, add_binding). Waiting for them
		// before repainting is what made the toggle feel stuck.
		let release
		const gate = new Promise((resolve) => {
			release = resolve
		})
		const state = mockNativeInvoke([])
		const stateful = nativeInvoke.getMockImplementation()
		nativeInvoke.mockImplementation(async (cmd, args = {}) => {
			if (cmd === 'ensure_remote_folder' || cmd === 'add_binding') await gate
			return stateful(cmd, args)
		})

		const { container } = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		await waitFor(() => expect(callsFor('list_bindings').length).toBeGreaterThan(0))

		const sw = container.querySelector('#settings-camera-switch')
		fireEvent.click(sw)

		// Still blocked on the native side, already ON on screen.
		await waitFor(() => expect(sw.getAttribute('aria-checked')).toBe('true'))
		expect(callsFor('add_binding').length).toBe(0)
		expect(sw.disabled).toBe(false)

		release()
		await waitFor(() => expect(callsFor('add_binding').length).toBe(1))
		expect(state.bindings.length).toBe(1)
		expect(sw.getAttribute('aria-checked')).toBe('true')
	})

	it('reverts the switch when the native call fails', async () => {
		mockNativeInvoke([])
		const stateful = nativeInvoke.getMockImplementation()
		nativeInvoke.mockImplementation(async (cmd, args = {}) => {
			if (cmd === 'add_binding') throw new Error('nope')
			return stateful(cmd, args)
		})

		const { container } = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		await waitFor(() => expect(callsFor('list_bindings').length).toBeGreaterThan(0))

		const sw = container.querySelector('#settings-camera-switch')
		fireEvent.click(sw)
		await waitFor(() => expect(callsFor('add_binding').length).toBe(1))
		await waitFor(() => expect(sw.getAttribute('aria-checked')).toBe('false'))
	})

	it('keeps its state across a Settings close and reopen', async () => {
		mockNativeInvoke([
			{
				id: 'b1',
				mode: 'auto_upload',
				enabled: true,
				local_path: '/p',
				remote_root: 'Camera/Pixel 8',
				storage_id: 'sid',
			},
		])
		const first = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		await waitFor(() =>
			expect(
				first.container
					.querySelector('#settings-camera-switch')
					.getAttribute('aria-checked'),
			).toBe('true'),
		)
		first.unmount()

		// Reopening must paint from the store, not from a cold list_bindings.
		nativeInvoke.mockImplementation(() => new Promise(() => {}))
		const second = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		const sw = second.container.querySelector('#settings-camera-switch')
		expect(sw.getAttribute('aria-checked')).toBe('true')
		expect(sw.disabled).toBe(false)
		expect(second.getByText('/p → Camera/Pixel 8')).toBeTruthy()
	})

	it('enabling with no camera binding adds one', async () => {
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
						app_lock_enabled: false,
						app_lock_pin_set: false,
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

	it('shows the upload list inline, without a download row', async () => {
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
						app_lock_enabled: false,
						app_lock_pin_set: false,
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

		const { container, queryByText, findByText } = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		await waitFor(() =>
			expect(callsFor('sync_transfer_queue').length).toBeGreaterThan(0),
		)
		// The list is right there — no row to tap open, no download section.
		expect(await findByText('a.jpg')).toBeTruthy()
		expect(queryByText('Downloading')).toBeNull()
		expect(
			container.querySelector('.settings-sync-panel__queue-count')?.textContent,
		).toContain('1')
	})

	it('shows unfinished upload count including waiting', async () => {
		mockNativeInvoke([])
		nativeInvoke.mockImplementation(async (cmd) => {
			switch (cmd) {
				case 'platform_label':
					return ''
				case 'device_label':
					return 'Pixel 8'
				case 'list_bindings':
					return []
				case 'get_client_prefs':
					return {
						wifi_only: true,
						app_lock_enabled: false,
						app_lock_pin_set: false,
					}
				case 'sync_statuses':
					return []
				case 'sync_transfer_queue':
					return {
						uploading: 2,
						downloading: 0,
						items: [
							{
								direction: 'upload',
								status: 'active',
								name: 'a.jpg',
								path: '',
								size: 1,
							},
							{
								direction: 'upload',
								status: 'waiting',
								name: 'b.jpg',
								path: '',
								size: 1,
							},
						],
					}
				case 'default_gallery_path':
					return '/pictures'
				default:
					return null
			}
		})

		const { container } = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		await waitFor(() =>
			expect(callsFor('sync_transfer_queue').length).toBeGreaterThan(0),
		)
		await waitFor(() =>
			expect(
				container.querySelector('.settings-sync-panel__queue-count')
					?.textContent,
			).toContain('2'),
		)
	})

	it('shows already-uploaded scan hint for Camera binding', async () => {
		nativeInvoke.mockImplementation(async (cmd) => {
			switch (cmd) {
				case 'platform_label':
					return 'Linux'
				case 'device_label':
					return 'dev'
				case 'list_bindings':
					return [
						{
							id: 'cam',
							mode: 'auto_upload',
							enabled: true,
							local_path: '/pictures',
							remote_root: 'Camera/dev',
							storage_id: 'sid',
						},
					]
				case 'get_client_prefs':
					return {
						wifi_only: true,
						app_lock_enabled: false,
						app_lock_pin_set: false,
					}
				case 'sync_statuses':
					return [
						{
							binding_id: 'cam',
							scanned: 5,
							pending: 0,
							already_synced: 5,
							uploading: 0,
							downloading: 0,
							conflicts: 0,
							cursor: 0,
							last_error: null,
						},
					]
				case 'sync_transfer_queue':
					return { uploading: 0, downloading: 0, items: [] }
				case 'default_gallery_path':
					return '/pictures'
				default:
					return null
			}
		})

		const { findByText } = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		expect(
			await findByText('5 media files found, all already uploaded'),
		).toBeTruthy()
	})

	it('does not rebind the camera folder to a storage merely being viewed while its bound storage still exists', async () => {
		// Regression test: opening Settings → Sync from *any* storage other than
		// the one the Camera binding is actually attached to used to be treated
		// as "the old storage was deleted" and silently rebound (removed +
		// recreated) the binding onto whatever storage happened to be open —
		// even though the originally bound storage was never deleted. A user
		// with more than one storage would have their Camera auto-upload
		// silently moved just by opening Settings while browsing a different
		// storage.
		mockNativeInvoke([
			{
				id: '1',
				mode: 'auto_upload',
				enabled: true,
				local_path: '/p',
				remote_root: 'Camera/Pixel 8',
				storage_id: 'storage-A',
			},
		])
		nativeInvoke.mockImplementation(
			(orig => async (cmd, args = {}) => {
				if (cmd === 'list_storages') {
					return [
						{ id: 'storage-A', name: 'A' },
						{ id: 'storage-B', name: 'B' },
					]
				}
				return orig(cmd, args)
			})(nativeInvoke.getMockImplementation()),
		)

		render(() => <SettingsSyncPanel storageId="storage-B" storageName="B" />)

		await waitFor(() => expect(callsFor('list_bindings').length).toBeGreaterThan(0))
		// Give any (buggy) automatic migration a chance to fire.
		await new Promise((r) => setTimeout(r, 50))

		expect(callsFor('remove_binding').length).toBe(0)
		expect(callsFor('add_binding').length).toBe(0)
	})

	it('shows cached camera ON immediately while list_bindings is slow', async () => {
		localStorage.setItem('sarca.client.cameraAutoUploadEnabled', '1')
		let release
		const gate = new Promise((resolve) => {
			release = resolve
		})
		nativeInvoke.mockReset()
		nativeInvoke.mockImplementation(async (cmd) => {
			switch (cmd) {
				case 'platform_label':
					return ''
				case 'device_label':
					return 'Pixel 8'
				case 'list_bindings':
					await gate
					return [
						{
							id: '1',
							mode: 'auto_upload',
							enabled: true,
							local_path: '/p',
							remote_root: 'Camera/Pixel 8',
						},
					]
				case 'get_client_prefs':
					return {
						wifi_only: true,
						app_lock_enabled: false,
						app_lock_pin_set: false,
					}
				case 'sync_statuses':
					return []
				case 'sync_transfer_queue':
					return { uploading: 0, downloading: 0, items: [] }
				case 'default_gallery_path':
					return '/pictures'
				default:
					return null
			}
		})

		const { container } = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		const sw = await waitFor(() => {
			const el = container.querySelector('#settings-camera-switch')
			expect(el).toBeTruthy()
			return el
		})
		expect(sw.getAttribute('aria-checked')).toBe('true')
		// Interactive from the first paint — a slow list_bindings must not lock
		// the switch, that is exactly what read as "the toggle hangs".
		expect(sw.disabled).toBe(false)

		release()
		await waitFor(() => expect(callsFor('list_bindings').length).toBeGreaterThan(0))
		expect(sw.disabled).toBe(false)
		expect(sw.getAttribute('aria-checked')).toBe('true')
	})

	it('does not flash camera OFF before slow list_bindings when cache says ON', async () => {
		localStorage.setItem('sarca.client.cameraAutoUploadEnabled', '1')
		const seen = []
		let release
		const gate = new Promise((resolve) => {
			release = resolve
		})
		nativeInvoke.mockReset()
		nativeInvoke.mockImplementation(async (cmd) => {
			switch (cmd) {
				case 'platform_label':
					return ''
				case 'device_label':
					return 'Pixel 8'
				case 'list_bindings':
					await gate
					return [
						{
							id: '1',
							mode: 'auto_upload',
							enabled: true,
							local_path: '/p',
							remote_root: 'Camera/Pixel 8',
						},
					]
				case 'get_client_prefs':
					return {
						wifi_only: true,
						app_lock_enabled: false,
						app_lock_pin_set: false,
					}
				case 'sync_statuses':
					await gate
					return []
				case 'sync_transfer_queue':
					return { uploading: 0, downloading: 0, items: [] }
				case 'default_gallery_path':
					return '/pictures'
				default:
					return null
			}
		})

		const { container } = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		for (let i = 0; i < 5; i++) {
			const sw = container.querySelector('#settings-camera-switch')
			seen.push(sw ? sw.getAttribute('aria-checked') : 'missing')
			await new Promise((r) => setTimeout(r, 10))
		}
		expect(seen.every((v) => v === 'true')).toBe(true)
		release()
		await waitFor(() =>
			expect(
				container.querySelector('#settings-camera-switch')?.disabled,
			).toBe(false),
		)
	})

	it('renders an enabled OFF switch instantly on a cold client, never a spinner', async () => {
		// No cache entry at all — a client that has never touched auto-upload.
		// The old panel showed a CircularProgress until list_bindings landed,
		// which is what "the toggle hangs when I open Sync settings" was.
		let release
		const gate = new Promise((resolve) => {
			release = resolve
		})
		nativeInvoke.mockReset()
		nativeInvoke.mockImplementation(async (cmd) => {
			if (cmd === 'list_bindings') {
				await gate
				return []
			}
			switch (cmd) {
				case 'platform_label':
					return ''
				case 'device_label':
					return 'Pixel 8'
				case 'get_client_prefs':
					return { wifi_only: true, app_lock_enabled: false, app_lock_pin_set: false }
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

		const sw = container.querySelector('#settings-camera-switch')
		expect(sw).toBeTruthy()
		expect(sw.getAttribute('aria-checked')).toBe('false')
		expect(sw.disabled).toBe(false)
		expect(
			container.querySelector('[aria-label="Loading auto-upload state"]'),
		).toBeNull()

		release()
		await waitFor(() => expect(callsFor('list_bindings').length).toBeGreaterThan(0))
		expect(sw.getAttribute('aria-checked')).toBe('false')
	})

	it('persists the toggle across a remount, without re-reading bindings first', async () => {
		// Cold-start survival: sessionStorage was cleared on every app restart,
		// so a client with auto-upload ON came back up rendering OFF (or a
		// spinner) until IPC answered. localStorage keeps the last known value.
		mockNativeInvoke([])
		const first = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		await waitFor(() => expect(callsFor('list_bindings').length).toBeGreaterThan(0))
		fireEvent.click(first.container.querySelector('#settings-camera-switch'))
		await waitFor(() => expect(callsFor('add_binding').length).toBe(1))
		await waitFor(() =>
			expect(
				first.container
					.querySelector('#settings-camera-switch')
					.getAttribute('aria-checked'),
			).toBe('true'),
		)
		first.unmount()

		expect(localStorage.getItem('sarca.client.cameraAutoUploadEnabled')).toBe('1')

		// Remount against a native layer that never answers: only the cache can
		// drive the first paint.
		nativeInvoke.mockReset()
		nativeInvoke.mockImplementation(() => new Promise(() => {}))
		const second = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		const sw = second.container.querySelector('#settings-camera-switch')
		expect(sw.getAttribute('aria-checked')).toBe('true')
		expect(sw.disabled).toBe(false)
	})

	it('records OFF in the cache so a disabled client stays disabled on restart', async () => {
		mockNativeInvoke([
			{
				id: '1',
				mode: 'auto_upload',
				enabled: true,
				local_path: '/p',
				remote_root: 'Camera/Pixel 8',
			},
		])
		const { container } = render(() => (
			<SettingsSyncPanel storageId="sid" storageName="Test" />
		))
		const sw = container.querySelector('#settings-camera-switch')
		await waitFor(() => expect(sw.getAttribute('aria-checked')).toBe('true'))

		fireEvent.click(sw)
		await waitFor(() => expect(callsFor('set_binding_enabled').length).toBe(1))
		await waitFor(() =>
			expect(localStorage.getItem('sarca.client.cameraAutoUploadEnabled')).toBe(
				'0',
			),
		)
	})
})
