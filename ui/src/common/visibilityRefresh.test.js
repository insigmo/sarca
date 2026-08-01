import { describe, it, expect } from 'vitest'

import { shouldRefreshOnVisibilityEvent } from './visibilityRefresh'

describe('shouldRefreshOnVisibilityEvent', () => {
	// Regression: native/background Camera auto-upload runs entirely in the
	// Tauri/Rust layer and never touches uploadQueueStore, so the only
	// existing refresh trigger (onItemDone/onIdle from the browser's own
	// upload UI) never fires for it — users had to pull-to-refresh manually
	// to see auto-uploaded files. Returning to the tab/app is now treated as
	// a signal to refresh the listing.
	it('refreshes when the page becomes visible', () => {
		expect(shouldRefreshOnVisibilityEvent('visible')).toBe(true)
	})

	it('does not refresh while the page is hidden', () => {
		expect(shouldRefreshOnVisibilityEvent('hidden')).toBe(false)
	})
})
