/**
 * Pure helpers for the Camera auto-upload toggle + client prefs coupling.
 * No I/O here — callers own the native invocations and side effects.
 */

/**
 * @param {Array<{ mode: string }>} bindings
 * @returns {{ id: string, mode: string, enabled?: boolean } | null}
 */
export function cameraBinding(bindings) {
	const list = Array.isArray(bindings) ? bindings : []
	return list.find((b) => b?.mode === 'auto_upload') || null
}

/**
 * Decides what should happen to the Camera auto_upload binding when the
 * user flips the switch, without performing any native calls itself.
 * @param {Array<{ id: string, mode: string, enabled?: boolean }>} bindings
 * @param {boolean} enable
 * @returns {{ action: 'noop' } | { action: 'add' } | { action: 'set_enabled', id: string, enabled: boolean }}
 */
export function resolveCameraToggle(bindings, enable) {
	const existing = cameraBinding(bindings)
	if (enable) {
		if (!existing) return { action: 'add' }
		if (existing.enabled === true) return { action: 'noop' }
		return { action: 'set_enabled', id: existing.id, enabled: true }
	}
	if (!existing) return { action: 'noop' }
	return { action: 'set_enabled', id: existing.id, enabled: false }
}

/**
 * @param {Record<string, unknown>} prefs
 * @returns {Record<string, unknown>}
 */
export function withBackgroundSyncOn(prefs) {
	return { ...prefs, background_sync: true }
}
