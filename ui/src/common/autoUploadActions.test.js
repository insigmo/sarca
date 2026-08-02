import { describe, it, expect } from 'vitest'
import {
	cameraBinding,
	cameraRemoteRoot,
	displayCameraRemoteRoot,
	needsCameraRootMigration,
	resolveCameraToggle,
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
	it('rebinds when enabling for a different storage than the binding', () => {
		expect(
			resolveCameraToggle(
				[
					{
						id: '1',
						mode: 'auto_upload',
						enabled: true,
						storage_id: 'old-storage',
					},
				],
				true,
				'new-storage',
			),
		).toEqual({ action: 'rebind', id: '1' })
	})
	it('rebinds disabled binding when storage differs', () => {
		expect(
			resolveCameraToggle(
				[
					{
						id: '1',
						mode: 'auto_upload',
						enabled: false,
						storage_id: 'old-storage',
					},
				],
				true,
				'new-storage',
			),
		).toEqual({ action: 'rebind', id: '1' })
	})
	it('noops when already enabled on the same storage', () => {
		expect(
			resolveCameraToggle(
				[
					{
						id: '1',
						mode: 'auto_upload',
						enabled: true,
						storage_id: 'same',
					},
				],
				true,
				'same',
			),
		).toEqual({ action: 'noop' })
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

describe('cameraRemoteRoot', () => {
	it('builds Camera/<device> path', () => {
		expect(cameraRemoteRoot('Pixel 8')).toBe('Camera/Pixel 8')
	})

	it('sanitizes unsafe path characters and falls back', () => {
		expect(cameraRemoteRoot('../My/Phone\\Name')).toBe('Camera/My Phone Name')
		expect(cameraRemoteRoot('   ')).toBe('Camera/Unknown device')
	})

	it('rejects localhost-style hostnames', () => {
		expect(cameraRemoteRoot('localhost')).toBe('Camera/Unknown device')
		expect(cameraRemoteRoot('127.0.0.1')).toBe('Camera/Unknown device')
	})
})

describe('displayCameraRemoteRoot', () => {
	it('returns null while device and platform labels are still loading', () => {
		expect(displayCameraRemoteRoot('', '')).toBe(null)
		expect(displayCameraRemoteRoot('  ', '')).toBe(null)
	})

	it('prefers device label over platform once either is known', () => {
		expect(displayCameraRemoteRoot('Pixel 8', 'Android')).toBe('Camera/Pixel 8')
		expect(displayCameraRemoteRoot('', 'Android')).toBe('Camera/Android')
	})
})

describe('needsCameraRootMigration', () => {
	it('flags legacy Camera and Unknown-device placeholders', () => {
		expect(needsCameraRootMigration('Camera', 'Camera/Pixel 8')).toBe(true)
		expect(needsCameraRootMigration('Camera/Unknown device', 'Camera/Pixel 8')).toBe(
			true,
		)
		expect(needsCameraRootMigration('Camera/Pixel 8', 'Camera/Pixel 8')).toBe(false)
		expect(needsCameraRootMigration('Camera/Old', 'Camera/Pixel 8')).toBe(false)
	})
})
