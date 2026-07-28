import { describe, it, expect } from 'vitest'
import { syncScanHint } from './syncScanHint'

describe('syncScanHint', () => {
	it('returns null when last_error set', () => {
		expect(syncScanHint({ last_error: 'x', scanned: 0 })).toBeNull()
	})
	it('reports empty folder', () => {
		expect(syncScanHint({ scanned: 0, pending: 0 })).toBe(
			'No media files found in the local folder',
		)
	})
	it('reports already uploaded', () => {
		expect(syncScanHint({ scanned: 5, pending: 0, already_synced: 5 })).toBe(
			'5 media files found, all already uploaded',
		)
		expect(syncScanHint({ scanned: 1, pending: 0 })).toBe(
			'1 media file found, all already uploaded',
		)
	})
	it('returns null while uploads unfinished', () => {
		expect(
			syncScanHint({ scanned: 5, pending: 2 }, { unfinishedUploads: 2 }),
		).toBeNull()
	})
	it('returns null when pending > 0 even if unfinished is 0', () => {
		expect(syncScanHint({ scanned: 5, pending: 2 }, { unfinishedUploads: 0 })).toBeNull()
	})
})
