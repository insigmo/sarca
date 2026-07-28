import { describe, it, expect } from 'vitest'
import {
	cameraBinding,
	resolveCameraToggle,
	withBackgroundSyncOn,
} from './autoUploadActions'

describe('cameraBinding', () => {
	it('returns null when no auto_upload row exists', () => {
		expect(cameraBinding([])).toBe(null)
		expect(cameraBinding([{ id: '1', mode: 'folder_upload' }])).toBe(null)
	})
	it('returns the first auto_upload row', () => {
		const rows = [
			{ id: '1', mode: 'folder_upload' },
			{ id: '2', mode: 'auto_upload', enabled: false },
			{ id: '3', mode: 'auto_upload', enabled: true },
		]
		expect(cameraBinding(rows)).toEqual({
			id: '2',
			mode: 'auto_upload',
			enabled: false,
		})
	})
})

describe('resolveCameraToggle', () => {
	it('adds when enabling with no binding', () => {
		expect(resolveCameraToggle([], true)).toEqual({ action: 'add' })
	})
	it('soft-disables existing enabled binding', () => {
		expect(
			resolveCameraToggle([{ id: '1', mode: 'auto_upload', enabled: true }], false),
		).toEqual({ action: 'set_enabled', id: '1', enabled: false })
	})
	it('re-enables disabled binding without add', () => {
		expect(
			resolveCameraToggle([{ id: '1', mode: 'auto_upload', enabled: false }], true),
		).toEqual({ action: 'set_enabled', id: '1', enabled: true })
	})
	it('noops when already enabled', () => {
		expect(
			resolveCameraToggle([{ id: '1', mode: 'auto_upload', enabled: true }], true),
		).toEqual({ action: 'noop' })
	})
	it('noops when disabling with no binding', () => {
		expect(resolveCameraToggle([], false)).toEqual({ action: 'noop' })
	})
})

describe('withBackgroundSyncOn', () => {
	it('forces background_sync true', () => {
		expect(withBackgroundSyncOn({ wifi_only: true, background_sync: false }))
			.toEqual({ wifi_only: true, background_sync: true })
	})
})
