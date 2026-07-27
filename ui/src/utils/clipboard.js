/**
 * Copy text to the system clipboard without hanging the UI.
 * WebKitGTK / Tauri often leave `navigator.clipboard.writeText` pending forever
 * when permission or focus is awkward — never await it unbounded.
 *
 * @param {string} text
 * @param {number} [timeoutMs=1500]
 * @returns {Promise<boolean>} true if write completed within timeout
 */
export async function copyToClipboard(text, timeoutMs = 1500) {
	if (typeof navigator === 'undefined' || !navigator.clipboard?.writeText) {
		return false
	}
	try {
		await Promise.race([
			navigator.clipboard.writeText(text),
			new Promise((_, reject) =>
				setTimeout(() => reject(new Error('clipboard timeout')), timeoutMs),
			),
		])
		return true
	} catch {
		return false
	}
}
