/**
 * Invoke a Tauri command from the remote-origin web UI (native client).
 * @param {string} cmd
 * @param {Record<string, unknown>} [args]
 * @returns {Promise<unknown>}
 */
export async function nativeInvoke(cmd, args = {}) {
	try {
		if (typeof window.__sarcaInvoke === 'function') {
			return await window.__sarcaInvoke(cmd, args)
		}
	} catch {
		// fall through
	}
	try {
		const inv = window.__TAURI_INTERNALS__?.invoke
		if (typeof inv === 'function') return await inv(cmd, args)
	} catch {
		// fall through
	}
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
