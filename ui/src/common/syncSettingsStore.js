import { createMemo, createRoot, createSignal } from 'solid-js'

import { nativeInvoke, pickLocalFolder } from './nativeBridge'
import {
	bindingStorageId,
	cameraBinding,
	cameraRemoteRoot,
	displayCameraRemoteRoot,
	needsCameraRootMigration,
	resolveCameraToggle,
} from './autoUploadActions'
import { sortTransferItems } from './syncTransferQueue'
import { syncScanHint } from './syncScanHint'
import { alertStore } from '../components/AlertStack'

const CAMERA_ENABLED_CACHE_KEY = 'sarca.client.cameraAutoUploadEnabled'
const SLOW_POLL_MS = 8000
const FAST_POLL_MS = 2000

/**
 * localStorage first so the toggle keeps its value across app restarts, then
 * sessionStorage for webviews that block persistent storage. Reading both on
 * load also migrates clients that only ever wrote the session copy.
 * @returns {Array<Storage>}
 */
function cacheStores() {
	const stores = []
	try {
		if (typeof localStorage !== 'undefined') stores.push(localStorage)
	} catch {
		// blocked storage
	}
	try {
		if (typeof sessionStorage !== 'undefined') stores.push(sessionStorage)
	} catch {
		// blocked storage
	}
	return stores
}

/**
 * Last known auto-upload state, or `null` when this client has never been told.
 * `null` means OFF for rendering purposes — auto-upload is opt-in everywhere.
 * @returns {boolean | null}
 */
function readCachedCameraEnabled() {
	for (const store of cacheStores()) {
		try {
			const v = store.getItem(CAMERA_ENABLED_CACHE_KEY)
			if (v === '1') return true
			if (v === '0') return false
		} catch {
			// private mode / blocked storage
		}
	}
	return null
}

function writeCachedCameraEnabled(enabled) {
	for (const store of cacheStores()) {
		try {
			store.setItem(CAMERA_ENABLED_CACHE_KEY, enabled ? '1' : '0')
		} catch {
			// ignore
		}
	}
}

/**
 * Sync lives outside the Settings modal on purpose: the panel is opened and
 * closed constantly, and re-running the whole cold-start dance (list_bindings
 * + statuses + transfer queue) on every mount is what made re-entering
 * Settings look frozen. This store owns the state and the polling; a panel
 * that mounts paints the last snapshot immediately and only refreshes in the
 * background.
 */
function createSyncSettingsStore() {
	const [platform, setPlatform] = createSignal('')
	const [deviceLabel, setDeviceLabel] = createSignal('')
	const [bindings, setBindings] = createSignal([])
	const [bindingsLoaded, setBindingsLoaded] = createSignal(false)
	const [cachedCameraOn, setCachedCameraOn] = createSignal(
		readCachedCameraEnabled(),
	)
	/**
	 * Toggle state the user just asked for, shown before the native calls
	 * finish. `null` = no pending intent, render the real binding state.
	 * @type {import('solid-js').Accessor<boolean | null>}
	 */
	const [pendingCameraOn, setPendingCameraOn] = createSignal(null)
	const [statuses, setStatuses] = createSignal([])
	// `app_lock_pin_set` is a read-only flag from Rust: the PIN itself never
	// crosses the bridge. Writing a PIN means sending `app_lock_pin` (plus
	// `current_app_lock_pin` when one already exists), which this store never
	// does — only the Security tab in SettingsModal does.
	const [prefs, setPrefs] = createSignal({
		wifi_only: true,
		app_lock_enabled: false,
		app_lock_pin_set: false,
	})
	const [localPath, setLocalPath] = createSignal('')
	const [busy, setBusy] = createSignal(false)
	const [msg, setMsg] = createSignal('')
	const [cameraRootMigrateTried, setCameraRootMigrateTried] = createSignal(false)
	const [staleStorageMigrateTried, setStaleStorageMigrateTried] =
		createSignal(false)
	const [transferSnap, setTransferSnap] = createSignal({
		uploading: 0,
		downloading: 0,
		items: [],
	})

	let storageIdRef = ''
	let mounted = 0
	let slowTimer = null
	let fastTimer = null

	const setStorageId = (id) => {
		storageIdRef = String(id || '')
	}
	const lockedStorageId = () => storageIdRef

	// Camera row can exist while soft-disabled — never removed on toggle-off.
	const autoBinding = () => cameraBinding(bindings())

	// The user's last flip wins over everything: a background refresh that
	// lands mid-flight must never bounce the switch back.
	const cameraOn = () => {
		const pending = pendingCameraOn()
		if (pending !== null) return pending
		if (!bindingsLoaded()) return cachedCameraOn() === true
		return autoBinding()?.enabled === true
	}

	const applyBindings = (raw) => {
		const list = Array.isArray(raw) ? raw : []
		setBindings(list)
		setBindingsLoaded(true)
		const enabled = cameraBinding(list)?.enabled === true
		setCachedCameraOn(enabled)
		writeCachedCameraEnabled(enabled)
		return list
	}

	const desiredCameraRemoteRoot = () =>
		displayCameraRemoteRoot(deviceLabel(), platform())

	const resolveExpectedCameraRoot = async () => {
		// Prefer a fresh native label (cached on disk after startup) so binding
		// creation never races the UI signal and writes Camera/Unknown device.
		try {
			const live = String((await nativeInvoke('device_label')) || '').trim()
			if (live) {
				setDeviceLabel(live)
				return cameraRemoteRoot(live)
			}
		} catch {
			// fall through
		}
		const fromUi = desiredCameraRemoteRoot()
		if (fromUi) return fromUi
		const plat =
			platform().trim() ||
			String((await nativeInvoke('platform_label').catch(() => '')) || '').trim()
		if (plat) {
			setPlatform(plat)
			return cameraRemoteRoot(plat)
		}
		return cameraRemoteRoot('')
	}

	const maybeMigrateLegacyCameraRoot = async (liveBindings) => {
		if (cameraRootMigrateTried()) return
		const sid = lockedStorageId()
		if (!sid) return
		const existing = cameraBinding(liveBindings)
		if (!existing) return
		const expectedRoot = await resolveExpectedCameraRoot()
		if (!needsCameraRootMigration(existing.remote_root, expectedRoot)) return
		setCameraRootMigrateTried(true)
		try {
			await nativeInvoke('ensure_remote_folder', {
				storageId: sid,
				parent: 'Camera',
				name: expectedRoot.replace(/^Camera\//, ''),
			})
			await nativeInvoke('update_binding_remote_root', {
				id: existing.id,
				remoteRoot: expectedRoot,
			})
			await refresh()
		} catch {
			// Keep compatibility with clients that don't expose this command yet.
		}
	}

	const maybeMigrateStaleCameraStorage = async (liveBindings) => {
		if (staleStorageMigrateTried()) return
		const sid = lockedStorageId()
		if (!sid) return
		const existing = cameraBinding(liveBindings)
		if (!existing) return
		const boundSid = bindingStorageId(existing)
		// Only "different storage than bound" is a candidate for migration —
		// this alone does NOT mean the bound storage is gone: Settings can be
		// opened from any storage's own settings, or from Files while browsing
		// a storage other than the one Camera auto-upload targets. Confirm the
		// bound storage was actually deleted before touching the binding.
		if (!boundSid || boundSid === sid) return
		setStaleStorageMigrateTried(true)
		try {
			const storages = await nativeInvoke('list_storages')
			// Can't confirm deletion (bad response) — do nothing rather than
			// risk rebinding a storage that's actually still there.
			if (!Array.isArray(storages)) return
			const stillExists = storages.some((s) => String(s?.id) === boundSid)
			if (stillExists) return
			// Re-run enable path: remove stale binding + add for current storage.
			await applyAutoUpload(true)
		} catch {
			setStaleStorageMigrateTried(false)
		}
	}

	const refreshTransfers = async () => {
		try {
			const snap = await nativeInvoke('sync_transfer_queue')
			if (snap && typeof snap === 'object') {
				setTransferSnap({
					uploading: Number(snap.uploading) || 0,
					downloading: Number(snap.downloading) || 0,
					items: Array.isArray(snap.items) ? snap.items : [],
				})
			}
		} catch {
			// Older clients may not expose this command yet.
		}
	}

	const refreshLabels = async () => {
		const [label, device] = await Promise.all([
			nativeInvoke('platform_label').catch(() => ''),
			nativeInvoke('device_label').catch(() => ''),
		])
		setPlatform(String(label || ''))
		setDeviceLabel(String(device || ''))
	}

	const refresh = async () => {
		try {
			// Kick labels off in parallel, but never let a slow device_label IPC
			// block bindings — that left the auto-upload toggle looking off.
			const labelsP = refreshLabels()
			// Apply bindings as soon as list_bindings resolves — do not wait for
			// sync_statuses / transfer queue (those can stall for seconds while a
			// large upload tick holds the index), or the camera toggle flashes OFF.
			const bindsP = nativeInvoke('list_bindings').then(
				(v) => {
					const list = applyBindings(v)
					return { ok: true, value: list }
				},
				(e) => ({ ok: false, error: e }),
			)
			const prefsP = nativeInvoke('get_client_prefs').catch(() => null)
			const statusP = nativeInvoke('sync_statuses').catch(() => [])
			const [bindsResult, prefsDto, statusList] = await Promise.all([
				bindsP,
				prefsP,
				statusP,
			])
			if (!bindsResult.ok) {
				setMsg(String(bindsResult.error))
			}
			setStatuses(Array.isArray(statusList) ? statusList : [])
			await refreshTransfers()
			if (prefsDto && typeof prefsDto === 'object') {
				setPrefs({
					wifi_only: prefsDto.wifi_only !== false,
					app_lock_enabled: Boolean(prefsDto.app_lock_enabled),
					app_lock_pin_set: Boolean(prefsDto.app_lock_pin_set),
				})
			}
			const binds = bindsResult.ok ? bindsResult.value : bindings()
			const auto = binds.find((b) => b.mode === 'auto_upload')
			if (auto?.local_path) setLocalPath(auto.local_path)
			else if (!localPath()) {
				try {
					const gallery = await nativeInvoke('default_gallery_path')
					if (gallery) setLocalPath(String(gallery))
				} catch {
					// ignore
				}
			}
			await labelsP
			await maybeMigrateLegacyCameraRoot(binds)
			await maybeMigrateStaleCameraStorage(binds)
		} catch (e) {
			setMsg(String(e))
		}
	}

	const kickSyncNow = () => {
		// Do not await: a full upload can take minutes and used to keep the
		// checkbox disabled/unchecked until sync finished (refresh ran only after).
		nativeInvoke('sync_now')
			.then(() => refresh())
			.catch((syncErr) => {
				alertStore.addAlert(String(syncErr), 'error')
				refresh()
			})
	}

	/** Native side of the toggle. Never awaited by the click handler. */
	const applyAutoUpload = async (enable) => {
		const sid = lockedStorageId()
		if (!sid) throw new Error('Open a storage in Files first')

		// Prefer live native list — local state can be empty after a failed refresh
		// while the binding is still in SQLite (looked like "off" in the UI).
		let live = []
		try {
			const listed = await nativeInvoke('list_bindings')
			live = Array.isArray(listed) ? listed : []
		} catch {
			live = bindings()
		}

		const decision = resolveCameraToggle(live, enable, sid)

		if (decision.action === 'noop') {
			setBindings(live)
			return
		}

		if (decision.action === 'rebind') {
			await nativeInvoke('remove_binding', { id: decision.id })
			// fall through to add for current storage
		} else if (decision.action === 'set_enabled') {
			// Soft-disable / re-enable in place — never remove_binding, so the
			// index and remote mapping survive a toggle-off/on cycle.
			await nativeInvoke('set_binding_enabled', {
				id: decision.id,
				enabled: decision.enabled,
			})
			if (decision.enabled) {
				const existing = live.find((b) => b.id === decision.id) || null
				const expectedRoot = await resolveExpectedCameraRoot()
				if (existing && String(existing.remote_root || '') !== expectedRoot) {
					await nativeInvoke('ensure_remote_folder', {
						storageId: sid,
						parent: 'Camera',
						name: expectedRoot.replace(/^Camera\//, ''),
					})
					await nativeInvoke('update_binding_remote_root', {
						id: existing.id,
						remoteRoot: expectedRoot,
					})
				}
			}
			await refresh()
			if (decision.enabled) kickSyncNow()
			return
		}

		// decision.action === 'add'
		let path = localPath().trim()
		if (!path) {
			path = String((await nativeInvoke('default_gallery_path')) || '')
			if (!path) {
				path = String((await pickLocalFolder('')) || '')
			}
			if (path) setLocalPath(path)
		}
		if (!path) throw new Error('Choose a local gallery / Pictures folder')
		await nativeInvoke('ensure_remote_folder', {
			storageId: sid,
			parent: '',
			name: 'Camera',
		})
		const expectedRoot = await resolveExpectedCameraRoot()
		await nativeInvoke('ensure_remote_folder', {
			storageId: sid,
			parent: 'Camera',
			name: expectedRoot.replace(/^Camera\//, ''),
		})
		const binding = await nativeInvoke('add_binding', {
			storageId: sid,
			remoteRoot: expectedRoot,
			localPath: path,
			mode: 'auto_upload',
		})
		// Optimistic UI so the toggle stays on while the engine catches up.
		if (binding && typeof binding === 'object') {
			setBindings((prev) => [
				...(Array.isArray(prev) ? prev.filter((b) => b.mode !== 'auto_upload') : []),
				binding,
			])
		}
		await refresh()
		kickSyncNow()
	}

	/**
	 * Flip the switch now, do the IPC after. The native path can take seconds
	 * (list_bindings behind a busy index, ensure_remote_folder over the
	 * network); waiting for it before repainting is what made the toggle feel
	 * stuck. Failures revert the optimistic state and raise an alert.
	 * @param {boolean} enable
	 * @returns {Promise<void>} resolves when the native work settles
	 */
	const setAutoUpload = (enable) => {
		setPendingCameraOn(enable)
		setCachedCameraOn(enable)
		writeCachedCameraEnabled(enable)
		setMsg('')
		return applyAutoUpload(enable)
			.then(() => {
				// Only drop the optimistic value if it is still ours: a newer flip
				// while this one was in flight owns the switch now.
				if (pendingCameraOn() === enable) setPendingCameraOn(null)
			})
			.catch(async (e) => {
				if (pendingCameraOn() === enable) setPendingCameraOn(null)
				setMsg(String(e))
				alertStore.addAlert(String(e), 'error')
				await refresh()
			})
	}

	/** "Upload now": acknowledge immediately, sync in the background. */
	const runSyncNow = () => {
		setMsg('')
		alertStore.addAlert('Upload started', 'success')
		kickSyncNow()
	}

	const savePrefs = async (next) => {
		setPrefs(next)
		await nativeInvoke('set_client_prefs', { prefs: next })
	}

	const pickFolder = async (current) => {
		setBusy(true)
		setMsg('')
		try {
			const path = await pickLocalFolder(current || '')
			if (path) return String(path)
			setMsg('No folder selected')
			return null
		} catch (e) {
			const text = String(e?.message || e)
			setMsg(text)
			alertStore.addAlert(text || 'Folder picker failed', 'error')
			return null
		} finally {
			setBusy(false)
		}
	}

	/** Repoint the Camera binding at `path`, creating it when missing. */
	const changeLocalFolder = async (path) => {
		setLocalPath(path)
		try {
			const existing = cameraBinding(await nativeInvoke('list_bindings'))
			if (!existing) {
				await setAutoUpload(true)
				return
			}
			await nativeInvoke('update_binding_local_path', {
				id: existing.id,
				localPath: path,
			})
			if (!existing.enabled) {
				await nativeInvoke('set_binding_enabled', {
					id: existing.id,
					enabled: true,
				})
			}
			kickSyncNow()
			await refresh()
		} catch (e) {
			setMsg(String(e))
			alertStore.addAlert(String(e), 'error')
			await refresh()
		}
	}

	// "Nothing is happening" is ambiguous: idle because everything is uploaded,
	// or idle because the scan found nothing? Say which.
	const scanHint = createMemo(() => {
		const binding = autoBinding()
		if (!binding) return null
		const status = statuses().find((s) => s.binding_id === binding.id)
		return syncScanHint(status, { unfinishedUploads: transferSnap().uploading })
	})

	const uploadItems = createMemo(() =>
		sortTransferItems(
			(transferSnap().items || []).filter((i) => i.direction === 'upload'),
		),
	)

	/**
	 * Ref-counted polling: the first mounted panel starts the timers, the last
	 * one to unmount stops them. State survives either way, so reopening
	 * Settings renders from the last snapshot instead of a cold load.
	 */
	const start = () => {
		mounted += 1
		// Re-read the cache on every mount: another window (or a previous app
		// run) may have flipped it since this store was created, and the panel
		// paints from it before list_bindings answers.
		if (!bindingsLoaded()) setCachedCameraOn(readCachedCameraEnabled())
		refresh()
		if (slowTimer !== null) return
		slowTimer = window.setInterval(() => refresh(), SLOW_POLL_MS)
		fastTimer = window.setInterval(() => refreshTransfers(), FAST_POLL_MS)
	}

	const stop = () => {
		mounted = Math.max(0, mounted - 1)
		if (mounted > 0) return
		if (slowTimer !== null) window.clearInterval(slowTimer)
		if (fastTimer !== null) window.clearInterval(fastTimer)
		slowTimer = null
		fastTimer = null
	}

	/** Test-only: drop every cached value and stop polling. */
	const reset = () => {
		mounted = 0
		if (slowTimer !== null) window.clearInterval(slowTimer)
		if (fastTimer !== null) window.clearInterval(fastTimer)
		slowTimer = null
		fastTimer = null
		storageIdRef = ''
		setPlatform('')
		setDeviceLabel('')
		setBindings([])
		setBindingsLoaded(false)
		setCachedCameraOn(readCachedCameraEnabled())
		setPendingCameraOn(null)
		setStatuses([])
		setPrefs({
			wifi_only: true,
			app_lock_enabled: false,
			app_lock_pin_set: false,
		})
		setLocalPath('')
		setBusy(false)
		setMsg('')
		setCameraRootMigrateTried(false)
		setStaleStorageMigrateTried(false)
		setTransferSnap({ uploading: 0, downloading: 0, items: [] })
	}

	return {
		autoBinding,
		busy,
		cameraOn,
		changeLocalFolder,
		deviceLabel,
		kickSyncNow,
		localPath,
		msg,
		pickFolder,
		platform,
		prefs,
		refresh,
		reset,
		runSyncNow,
		savePrefs,
		scanHint,
		setAutoUpload,
		setMsg,
		setStorageId,
		start,
		statuses,
		stop,
		transferSnap,
		uploadItems,
	}
}

export const syncSettingsStore = createRoot(createSyncSettingsStore)
