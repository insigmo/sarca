import { describe, it, expect, vi } from 'vitest'
import {
	VIEWER_HISTORY_KEY,
	isViewerHistoryState,
	pushViewerHistory,
	shouldCloseViewerOnPopstate,
} from './viewerHistory'

describe('isViewerHistoryState', () => {
	it('detects sarcaViewer flag', () => {
		expect(isViewerHistoryState({ [VIEWER_HISTORY_KEY]: 1 })).toBe(true)
		expect(isViewerHistoryState({})).toBe(false)
		expect(isViewerHistoryState(null)).toBe(false)
	})
})

describe('pushViewerHistory', () => {
	it('pushState with viewer flag and current url', () => {
		const history = { pushState: vi.fn() }
		pushViewerHistory(history, 'https://example/files/s1/')
		expect(history.pushState).toHaveBeenCalledWith(
			{ [VIEWER_HISTORY_KEY]: 1 },
			'',
			'https://example/files/s1/',
		)
	})
})

describe('shouldCloseViewerOnPopstate', () => {
	it('closes when viewer open regardless of state payload', () => {
		expect(
			shouldCloseViewerOnPopstate({ viewerOpen: true, state: null }),
		).toBe(true)
		expect(
			shouldCloseViewerOnPopstate({ viewerOpen: false, state: { sarcaViewer: 1 } }),
		).toBe(false)
	})
})
