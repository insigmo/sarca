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
	it('names deferred files ahead of the error banner', () => {
		// A held-back file is invisible otherwise: it is not pending, not
		// transferring, and its deadline is not shown anywhere.
		expect(syncScanHint({ scanned: 5, pending: 0, deferred: 3 })).toBe(
			'3 files failed to upload and will be retried automatically — use Upload now to retry immediately',
		)
		expect(
			syncScanHint({ last_error: 'connection refused', scanned: 5, deferred: 1 }),
		).toBe(
			'1 file failed to upload and will be retried automatically — use Upload now to retry immediately',
		)
	})
	it('ignores deferred when zero or absent', () => {
		expect(syncScanHint({ scanned: 5, pending: 0, deferred: 0 })).toBe(
			'5 media files found, all already uploaded',
		)
	})
})
