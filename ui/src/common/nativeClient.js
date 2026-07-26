/**
 * Detect Sarca native shell (Tauri). Set during session inject as localStorage.sarca_native=1.
 * @returns {boolean}
 */
export const isNativeClient = () => {
	try {
		return localStorage.getItem('sarca_native') === '1'
	} catch {
		return false
	}
}

/**
 * Open the local Sync settings page from a remote-origin webview.
 * Uses a same-origin `?__sarca_open_sync=1` navigation that Rust intercepts
 * (reliable on Android WebView). Falls back to `sarca-sync://open`.
 * @param {Event} [event]
 */
export const openNativeSyncSettings = (event) => {
	event?.preventDefault?.()
	try {
		const u = new URL(window.location.href)
		u.searchParams.set('__sarca_open_sync', '1')
		window.location.assign(u.toString())
		return
	} catch {
		// ignore
	}
	window.location.assign('sarca-sync://open')
}
