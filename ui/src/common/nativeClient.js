import { createRoot, createSignal } from 'solid-js'

import { nativeInvoke } from './nativeBridge'
import { settingsStore } from './settings'

/**
 * Detect Sarca native shell (Tauri webview loading the server UI).
 * Prefer localStorage.sarca_native (set by client init/inject); also accept
 * query/hash flags and Tauri globals when present on the page.
 * @returns {boolean}
 */
export const detectNativeClient = () => {
	try {
		if (typeof window === 'undefined') return false
		if (window.__SARCA_NATIVE__ === 1 || window.__SARCA_NATIVE__ === true) {
			return true
		}
		if (localStorage.getItem('sarca_native') === '1') return true
		const u = new URL(window.location.href)
		if (u.searchParams.get('__sarca_native') === '1') {
			try {
				localStorage.setItem('sarca_native', '1')
				window.__SARCA_NATIVE__ = 1
				u.searchParams.delete('__sarca_native')
				const next = `${u.pathname}${u.search}${u.hash}`
				window.history.replaceState(null, '', next || '/')
			} catch {
				// ignore
			}
			return true
		}
		if ((u.hash || '').includes('__sarca_native=1')) return true
		if (window.__TAURI_INTERNALS__ || window.__TAURI__) return true
	} catch {
		// ignore
	}
	return false
}

/**
 * @returns {boolean}
 */
export const isNativeClient = () => detectNativeClient()

/**
 * Reactive native detection so late Android/desktop inject still reveals Sync.
 */
export const nativeClientStore = createRoot(() => {
	const [isNative, setIsNative] = createSignal(detectNativeClient())

	const refresh = () => {
		const next = detectNativeClient()
		if (next !== isNative()) setIsNative(next)
		return next
	}

	if (typeof window !== 'undefined') {
		const onReady = () => {
			refresh()
		}
		window.addEventListener('sarca-native', onReady)
		window.addEventListener('storage', onReady)

		// Late inject (common on Android remote WebView): poll briefly.
		let ticks = 0
		const id = window.setInterval(() => {
			ticks += 1
			if (refresh() || ticks >= 40) window.clearInterval(id)
		}, 250)
	}

	return { isNative, refresh }
})

/**
 * Report visibility to the native shell so its background sync loop can poll
 * fast while someone is watching and back off once they're not — Tauri v2 has
 * no `RunEvent::Paused` to pair with `Resumed`, so the webview is the only
 * thing that actually knows this (see `AppSyncState::is_foreground` on the
 * Rust side). Android's WebView fires `visibilitychange` when the activity is
 * paused, which is the signal we want there too.
 *
 * Every call is gated on `isNativeClient()`, so in the browser build this
 * just adds a few inert listeners and a no-op interval.
 */
const bindForegroundSignal = () => {
	if (typeof window === 'undefined' || typeof document === 'undefined') return

	const report = (active) => {
		if (!isNativeClient()) return
		nativeInvoke('set_app_foreground', { active }).catch(() => {
			// Bridge not wired up yet (early boot) or already tearing down —
			// the next visibilitychange/heartbeat will retry.
		})
	}

	document.addEventListener('visibilitychange', () => {
		report(document.visibilityState === 'visible')
	})
	// visibilitychange does not fire on every platform when the app/tab is
	// actually killed rather than merely hidden; pagehide is the best-effort
	// "going away" signal so a closed webview does not sit reported as
	// foreground until the 3-minute staleness fallback on the Rust side.
	window.addEventListener('pagehide', () => {
		report(false)
	})

	// Heartbeat: re-confirms foreground periodically so a webview that never
	// backgrounds (no visibilitychange fired) does not go stale on the Rust
	// side, whose ping timeout is 3 minutes.
	window.setInterval(() => {
		if (document.visibilityState === 'visible') report(true)
	}, 60_000)
}

bindForegroundSignal()

/**
 * Open in-app Settings → Sync (no separate sync.html as primary UI).
 * @param {Event} [event]
 */
export const openNativeSyncSettings = (event) => {
	event?.preventDefault?.()
	try {
		if (detectNativeClient()) {
			settingsStore.openSettings('sync')
			return
		}
	} catch {
		// ignore
	}
	try {
		if (typeof window.__sarcaOpenSyncSettings === 'function') {
			window.__sarcaOpenSyncSettings()
			return
		}
	} catch {
		// ignore
	}
	try {
		const u = new URL(window.location.href)
		u.searchParams.set('__sarca_open_settings', 'sync')
		window.history.replaceState(null, '', u.pathname + u.search + u.hash)
		window.dispatchEvent(
			new CustomEvent('sarca-open-settings', { detail: { tab: 'sync' } }),
		)
		settingsStore.openSettings('sync')
	} catch {
		// ignore
	}
}

/**
 * Consume `?__sarca_open_settings=` / custom event and open Settings.
 * @returns {() => void}
 */
export const bindOpenSettingsDeepLink = () => {
	if (typeof window === 'undefined') return () => {}

	const openFromUrl = () => {
		try {
			const u = new URL(window.location.href)
			const tab = u.searchParams.get('__sarca_open_settings')
			if (!tab) return
			u.searchParams.delete('__sarca_open_settings')
			window.history.replaceState(null, '', u.pathname + u.search + u.hash)
			settingsStore.openSettings(/** @type {any} */ (tab))
		} catch {
			// ignore
		}
	}

	const onEvent = (event) => {
		const tab = event?.detail?.tab || 'sync'
		settingsStore.openSettings(tab)
	}

	openFromUrl()
	window.addEventListener('sarca-open-settings', onEvent)
	window.addEventListener('popstate', openFromUrl)
	return () => {
		window.removeEventListener('sarca-open-settings', onEvent)
		window.removeEventListener('popstate', openFromUrl)
	}
}
