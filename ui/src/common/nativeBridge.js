/**
 * Invoke a native client command from the remote-origin web UI.
 * Prefers the injected `window.__sarcaInvoke` bridge (custom protocol / nav IPC).
 * @param {string} cmd
 * @param {Record<string, unknown>} [args]
 * @returns {Promise<unknown>}
 */
export async function nativeInvoke(cmd, args = {}) {
	if (typeof window.__sarcaInvoke === 'function') {
		return await window.__sarcaInvoke(cmd, args)
	}
	const inv = window.__TAURI_INTERNALS__?.invoke
	if (typeof inv === 'function') return await inv(cmd, args)
	throw new Error('Native bridge unavailable')
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
 * System folder picker with typed-path fallback (required on mobile; useful on Linux).
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
	const mobile = isMobileNativePlatform(platform)

	try {
		const path = await nativeInvoke('pick_local_folder')
		if (path) return String(path)
	} catch (e) {
		const msg = String(e?.message || e || '')
		// Mobile intentionally returns FOLDER_PICKER_USE_PROMPT; desktop may fail too.
		if (
			!mobile &&
			!/FOLDER_PICKER_USE_PROMPT|unavailable|not supported|timed out|cancel/i.test(
				msg,
			)
		) {
			// Unexpected desktop error — still offer typed path, but keep message.
			console.warn('pick_local_folder:', msg)
		}
	}

	const hint = mobile
		? 'Enter a local folder path, e.g. /storage/emulated/0/DCIM or /storage/emulated/0/Pictures'
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
