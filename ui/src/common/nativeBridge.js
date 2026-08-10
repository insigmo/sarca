/** How long a caller waits for the bridge to come up before giving up. */
const BRIDGE_READY_TIMEOUT_MS = 12_000
/** Gap between warm-up attempts while the bridge is still coming up. */
const BRIDGE_RETRY_MS = 150

/**
 * True for the transient errors seen only in the first moments after the
 * client navigates the WebView to a server.
 *
 * The native side grants this origin its capability immediately before the
 * navigation, but the grant is not visible to the page the instant it starts
 * running scripts. Every command fired from `onMount` therefore had a window
 * in which it came back "not allowed by ACL" — which is why opening a server
 * used to flash an ACL error that fixed itself a moment later.
 * @param {unknown} error
 */
const isBridgeWarmingUp = (error) => {
	const message = String(error?.message || error || '').toLowerCase()
	return (
		message.includes('not allowed by acl') ||
		message.includes('denied by acl') ||
		message.includes('unauthorized origin') ||
		message.includes('native bridge unavailable') ||
		message.includes('tauri invoke unavailable')
	)
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

/**
 * Resolves once any command has gone through. Shared by every caller, so the
 * warm-up is paid once per page load instead of once per component - the old
 * arrangement, where each caller ran its own retry loop, meant whichever one
 * had no loop reported the error to the user.
 * @type {Promise<void> | null}
 */
let bridgeReady = null

/** @returns {unknown} */
const rawInvoke = (cmd, args) => {
	if (typeof window.__sarcaInvoke === 'function') {
		return window.__sarcaInvoke(cmd, args)
	}
	const inv = window.__TAURI_INTERNALS__?.invoke
	if (typeof inv === 'function') return inv(cmd, args)
	return Promise.reject(new Error('Native bridge unavailable'))
}

/**
 * Invoke a native client command from the remote-origin web UI.
 * Prefers the injected `window.__sarcaInvoke` bridge (custom protocol / nav IPC).
 *
 * Retries only while the bridge looks like it is still starting up. A real
 * command error (a rejected argument, a failed disconnect) is returned to the
 * caller on the first try, unchanged.
 * @param {string} cmd
 * @param {Record<string, unknown>} [args]
 * @returns {Promise<unknown>}
 */
export async function nativeInvoke(cmd, args = {}) {
	const deadline = Date.now() + BRIDGE_READY_TIMEOUT_MS
	for (;;) {
		try {
			const result = await rawInvoke(cmd, args)
			bridgeReady = Promise.resolve()
			return result
		} catch (e) {
			if (!isBridgeWarmingUp(e) || Date.now() >= deadline) throw e
			await sleep(BRIDGE_RETRY_MS)
		}
	}
}

/**
 * Message to show a user for a failed native command.
 *
 * Raw bridge text ("Command get_client_prefs not allowed by ACL") means
 * nothing to a user and there is no action they can take on it, so it never
 * reaches a toast. Real command errors are passed through untouched.
 * @param {unknown} error
 * @param {string} [fallback]
 * @returns {string}
 */
export function describeNativeError(error, fallback = 'The app is still starting up. Try again.') {
	if (isBridgeWarmingUp(error)) return fallback
	const message = String(error?.message || error || '').trim()
	return message || fallback
}

/**
 * Await the native bridge before issuing commands, for callers that would
 * rather wait than see a transient failure. Resolves immediately once any
 * command has succeeded.
 * @returns {Promise<void>}
 */
export function whenBridgeReady() {
	if (!bridgeReady) {
		bridgeReady = nativeInvoke('platform_label').then(
			() => undefined,
			() => undefined,
		)
	}
	return bridgeReady
}

/**
 * @param {string} [label]
 * @returns {boolean}
 */
export function isMobileNativePlatform(label) {
	const p = String(label || '').toLowerCase()
	return p === 'android' || p === 'ios'
}

/**
 * System folder picker with typed-path fallback only when the OS cannot provide one.
 * Desktop: native OS dialog. Android: SAF document-tree picker.
 * Prompt is last resort (iOS, or Android tree URI that cannot map to a filesystem path).
 * @param {string} [existing]
 * @returns {Promise<string | null>}
 */
export async function pickLocalFolder(existing = '') {
	let platform = ''
	try {
		platform = String((await nativeInvoke('platform_label')) || '')
	} catch {
		// ignore
	}
	// UA fallback when platform_label is blocked (misconfigured ACL) so Android
	// still gets the mobile hint instead of the desktop prompt wording.
	const mobile =
		isMobileNativePlatform(platform) ||
		/Android|iPhone|iPad|iPod/i.test(navigator.userAgent || '')

	try {
		// Hand the dialog a start directory: the Linux portal otherwise opens
		// on "Recent" and stalls for seconds enumerating it.
		const path = await nativeInvoke('pick_local_folder', {
			current: existing || '',
		})
		// null/undefined = user cancelled — do not fall through to prompt
		if (path) return String(path)
		return null
	} catch (e) {
		const msg = String(e?.message || e || '')
		if (/FOLDER_PICKER_USE_PROMPT/i.test(msg)) {
			// intentional typed-path fallback (unresolvable SAF tree URI, iOS)
		} else if (/cancel/i.test(msg)) {
			return null
		} else {
			// ACL / bridge / plugin errors must surface — never hide behind prompt
			console.warn('pick_local_folder:', msg)
			throw e instanceof Error ? e : new Error(msg)
		}
	}

	const hint = mobile
		? 'Folder picker could not resolve a filesystem path. Enter a local folder path, e.g. /storage/emulated/0/DCIM or /storage/emulated/0/Pictures'
		: 'Enter a local folder path'
	const fallback = existing || (mobile ? '/storage/emulated/0/DCIM' : '')
	const typed = window.prompt(hint, fallback)
	return typed && typed.trim() ? typed.trim() : null
}

/**
 * @param {number} bytes
 * @returns {string}
 */
export function formatBytes(bytes) {
	const n = Number(bytes) || 0
	if (n < 1024) return `${n} B`
	if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KB`
	if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`
	return `${(n / 1024 ** 3).toFixed(2)} GB`
}
