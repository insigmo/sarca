/**
 * Build a short honesty hint for Sync Settings after a scan tick.
 * Returns null when the error banner or transfer queue should own the UX.
 *
 * @param {{ last_error?: string|null, scanned?: number, pending?: number, already_synced?: number, deferred?: number }|null|undefined} status
 * @param {{ unfinishedUploads?: number }} [opts]
 * @returns {string|null}
 */
export function syncScanHint(status, opts = {}) {
	if (!status || typeof status !== 'object') return null
	// Deferred files come first, ahead of even the error banner: a file that
	// keeps failing is held back by a retry deadline the user cannot see, so
	// without this the panel goes quiet and looks like nothing is wrong while
	// real work is waiting. Say how many, and name the way out.
	const deferred = Number(status.deferred) || 0
	if (deferred > 0) {
		const label = deferred === 1 ? 'file' : 'files'
		return `${deferred} ${label} failed to upload and will be retried automatically — use Upload now to retry immediately`
	}
	if (status.last_error) return null
	const unfinished = Number(opts.unfinishedUploads) || 0
	if (unfinished > 0) return null
	const scanned = Number(status.scanned) || 0
	const pending = Number(status.pending) || 0
	if (pending > 0) return null
	if (scanned === 0) {
		return 'No media files found in the local folder'
	}
	const noun = scanned === 1 ? 'file' : 'files'
	return `${scanned} media ${noun} found, all already uploaded`
}
