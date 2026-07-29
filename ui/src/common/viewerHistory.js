export const VIEWER_HISTORY_KEY = 'sarcaViewer'

/** @param {unknown} state */
export function isViewerHistoryState(state) {
	return Boolean(state && typeof state === 'object' && state[VIEWER_HISTORY_KEY])
}

/**
 * @param {{ pushState: (data: object, unused: string, url?: string) => void }} history
 * @param {string} url
 */
export function pushViewerHistory(history, url) {
	history.pushState({ [VIEWER_HISTORY_KEY]: 1 }, '', url)
}

/**
 * @param {{ viewerOpen: boolean, state?: unknown }} opts
 */
export function shouldCloseViewerOnPopstate({ viewerOpen }) {
	return Boolean(viewerOpen)
}
