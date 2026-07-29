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
 * Binding storage id (native uses snake_case; tolerate camelCase).
 * @param {{ storage_id?: string, storageId?: string } | null | undefined} binding
 * @returns {string}
 */
export function bindingStorageId(binding) {
	if (!binding || typeof binding !== 'object') return ''
	return String(binding.storage_id || binding.storageId || '').trim()
}

/**
 * Decides what should happen to the Camera auto_upload binding when the
 * user flips the switch, without performing any native calls itself.
 *
 * When enabling for a *different* Files storage than the binding's
 * `storage_id` (e.g. old storage was deleted/recreated), returns `rebind`
 * so callers remove the stale binding and add a fresh one — otherwise
 * uploads keep targeting a missing storage forever while the toggle looks ON.
 *
 * @param {Array<{ id: string, mode: string, enabled?: boolean, storage_id?: string, storageId?: string }>} bindings
 * @param {boolean} enable
 * @param {string} [currentStorageId] Files storage currently open
 * @returns {{ action: 'noop' } | { action: 'add' } | { action: 'rebind', id: string } | { action: 'set_enabled', id: string, enabled: boolean }}
 */
export function resolveCameraToggle(bindings, enable, currentStorageId = '') {
	const existing = cameraBinding(bindings)
	const sid = String(currentStorageId || '').trim()
	if (enable) {
		if (!existing) return { action: 'add' }
		const boundSid = bindingStorageId(existing)
		if (sid && boundSid && sid !== boundSid) {
			return { action: 'rebind', id: existing.id }
		}
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

/**
 * Builds remote camera path per device: `Camera/<device>`.
 * @param {string} deviceLabel
 * @returns {string}
 */
export function cameraRemoteRoot(deviceLabel) {
	const raw = String(deviceLabel || '').trim()
	const lowerRaw = raw.toLowerCase()
	if (
		!raw ||
		lowerRaw === 'localhost' ||
		lowerRaw === 'localhost.localdomain' ||
		lowerRaw === '127.0.0.1'
	) {
		return 'Camera/Unknown device'
	}
	const cleaned = raw
		.replace(/[\\/]+/g, ' ')
		.replace(/\.+/g, ' ')
		.replace(/\s+/g, ' ')
		.trim()
	const suffix = cleaned || 'Unknown device'
	return `Camera/${suffix}`
}

/**
 * Like {@link cameraRemoteRoot}, but returns `null` while labels are still loading
 * so the Sync UI never flashes sticky "Unknown device".
 * @param {string} deviceLabel
 * @param {string} [platformLabel]
 * @returns {string | null}
 */
export function displayCameraRemoteRoot(deviceLabel, platformLabel = '') {
	const label =
		String(deviceLabel || '').trim() || String(platformLabel || '').trim()
	if (!label) return null
	return cameraRemoteRoot(label)
}

/**
 * True when an existing binding remote_root should be rewritten to `expectedRoot`.
 * @param {string} remoteRoot
 * @param {string} expectedRoot
 * @returns {boolean}
 */
export function needsCameraRootMigration(remoteRoot, expectedRoot) {
	const root = String(remoteRoot || '')
	const expected = String(expectedRoot || '')
	if (!expected || expected === 'Camera' || expected === 'Camera/Unknown device') {
		return false
	}
	return root === 'Camera' || root === 'Camera/Unknown device'
}
