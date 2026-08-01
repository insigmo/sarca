import { For, Show, createMemo, createSignal, onCleanup, onMount } from 'solid-js'
import Button from '@suid/material/Button'
import CircularProgress from '@suid/material/CircularProgress'
import Typography from '@suid/material/Typography'
import AccessTimeIcon from '@suid/icons-material/AccessTime'
import CheckIcon from '@suid/icons-material/Check'
import LoadingDots from './LoadingDots'

import {
	formatBytes,
	isMobileNativePlatform,
	nativeInvoke,
	pickLocalFolder,
} from '../common/nativeBridge'
import {
	cameraBinding,
	cameraRemoteRoot,
	displayCameraRemoteRoot,
	needsCameraRootMigration,
	resolveCameraToggle,
	withBackgroundSyncOn,
} from '../common/autoUploadActions'
import { sortTransferItems } from '../common/syncTransferQueue'
import { syncScanHint } from '../common/syncScanHint'
import { filesChromeStore } from '../common/filesChrome'
import { alertStore } from './AlertStack'
import FluentIcon from './FluentIcon'
import SettingsSwitch from './SettingsSwitch'

const CAMERA_ENABLED_CACHE_KEY = 'sarca.client.cameraAutoUploadEnabled'

function readCachedCameraEnabled() {
	try {
		const v = sessionStorage.getItem(CAMERA_ENABLED_CACHE_KEY)
		if (v === '1') return true
		if (v === '0') return false
	} catch {
		// private mode / blocked storage
	}
	return null
}

function writeCachedCameraEnabled(enabled) {
	try {
		sessionStorage.setItem(CAMERA_ENABLED_CACHE_KEY, enabled ? '1' : '0')
	} catch {
		// ignore
	}
}

/**
 * Sync tab: Camera media auto-upload + manage existing folder bindings.
 * Storage is locked to the currently open Files storage.
 * @param {{ storageId?: string, storageName?: string }} props
 */
const SettingsSyncPanel = (props) => {
	const { addAlert } = alertStore
	const chrome = filesChromeStore
	const [platform, setPlatform] = createSignal('')
	const [deviceLabel, setDeviceLabel] = createSignal('')
	const [bindings, setBindings] = createSignal([])
	const [bindingsLoaded, setBindingsLoaded] = createSignal(false)
	const [cachedCameraOn, setCachedCameraOn] = createSignal(readCachedCameraEnabled())
	const [statuses, setStatuses] = createSignal([])
	const [prefs, setPrefs] = createSignal({
		wifi_only: true,
		background_sync: true,
		app_lock_enabled: false,
		app_lock_pin: null,
	})
	const [prefsLoaded, setPrefsLoaded] = createSignal(false)
	const [localPath, setLocalPath] = createSignal('')
	const [busy, setBusy] = createSignal(false)
	const [msg, setMsg] = createSignal('')
	const [cameraRootMigrateTried, setCameraRootMigrateTried] = createSignal(false)
	const [staleStorageMigrateTried, setStaleStorageMigrateTried] = createSignal(false)
	/** @type {import('solid-js').Accessor<'upload' | 'download' | null>} */
	const [queueView, setQueueView] = createSignal(null)
	const [transferSnap, setTransferSnap] = createSignal({
		uploading: 0,
		downloading: 0,
		items: [],
	})

	const isMobile = () => isMobileNativePlatform(platform())

	const lockedStorageId = () => props.storageId || chrome.storageId() || ''

	// Camera row can exist while soft-disabled — never removed on toggle-off.
	const autoBinding = () => cameraBinding(bindings())
	// Until the first successful list_bindings, prefer session cache so remounting
	// Sync does not flash the empty-bindings default (OFF) while IPC is in flight.
	const cameraOn = () => {
		if (!bindingsLoaded()) {
			return cachedCameraOn() === true
		}
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
	const cameraScanHint = createMemo(() => {
		const auto = autoBinding()
		if (!auto) return null
		const st = statuses().find((s) => s?.binding_id === auto.id) || null
		return syncScanHint(st, {
			unfinishedUploads: Number(transferSnap().uploading) || 0,
		})
	})
	const folderBindings = () =>
		bindings().filter(
			(b) => b.mode === 'folder_upload' || b.mode === 'sync',
		)
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
		const plat = platform().trim() || String((await nativeInvoke('platform_label').catch(() => '')) || '').trim()
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
					background_sync: prefsDto.background_sync !== false,
					app_lock_enabled: Boolean(prefsDto.app_lock_enabled),
					app_lock_pin: prefsDto.app_lock_pin ?? null,
				})
				setPrefsLoaded(true)
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

	const maybeMigrateStaleCameraStorage = async (liveBindings) => {
		if (staleStorageMigrateTried()) return
		const sid = lockedStorageId()
		if (!sid) return
		const decision = resolveCameraToggle(liveBindings, true, sid)
		if (decision.action !== 'rebind') return
		setStaleStorageMigrateTried(true)
		try {
			// Re-run enable path: remove stale binding + add for current storage.
			await setAutoUpload(true)
		} catch {
			setStaleStorageMigrateTried(false)
		}
	}

	onMount(() => {
		refresh()
		const id = window.setInterval(() => {
			refresh()
		}, 8000)
		const fast = window.setInterval(() => {
			refreshTransfers()
		}, 2000)
		onCleanup(() => {
			window.clearInterval(id)
			window.clearInterval(fast)
		})
	})

	const queueItems = createMemo(() => {
		const dir = queueView()
		if (!dir) return []
		return sortTransferItems(
			(transferSnap().items || []).filter((i) => i.direction === dir),
		)
	})

	const statusIcon = (status) => {
		if (status === 'active') {
			// A transfer list can hold many "active" rows at once; a spinner per
			// row means that many continuously-repainting animations at all
			// times, which is costly under WebKitGTK. Animated dot text is not.
			return (
				<span style={{ 'font-size': '16px', 'line-height': 1, color: 'var(--sarca-teal)' }}>
					<LoadingDots />
				</span>
			)
		}
		if (status === 'waiting') {
			return <AccessTimeIcon sx={{ fontSize: 16, color: 'text.secondary' }} />
		}
		return <CheckIcon sx={{ fontSize: 16, color: 'success.main' }} />
	}

	const savePrefs = async (next) => {
		setPrefs(next)
		await nativeInvoke('set_client_prefs', { prefs: next })
	}

	// Turning a binding on always wants background_sync enabled, but naively
	// saving `withBackgroundSyncOn(prefs())` before the first successful
	// `refresh()` would persist the signal's hardcoded defaults (in
	// particular app_lock_enabled/app_lock_pin) over whatever the user
	// actually has saved, silently disabling their app lock. Fetch a fresh
	// base straight from native prefs whenever we haven't loaded real prefs
	// yet, and skip the save entirely if that also fails.
	const enableBackgroundSyncSafely = async () => {
		if (prefsLoaded()) {
			await savePrefs(withBackgroundSyncOn(prefs()))
			return
		}
		try {
			const fresh = await nativeInvoke('get_client_prefs')
			if (fresh && typeof fresh === 'object') {
				setPrefsLoaded(true)
				await savePrefs(withBackgroundSyncOn(fresh))
			}
		} catch {
			// No reliable prefs source — skip rather than risk overwriting
			// real prefs (e.g. app lock) with defaults.
		}
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
			addAlert(text || 'Folder picker failed', 'error')
			return null
		} finally {
			setBusy(false)
		}
	}

	const kickSyncNow = () => {
		// Do not await: a full upload can take minutes and used to keep the
		// checkbox disabled/unchecked until sync finished (refresh ran only after).
		nativeInvoke('sync_now')
			.then(() => refresh())
			.catch((syncErr) => {
				addAlert(String(syncErr), 'error')
				refresh()
			})
	}

	const setAutoUpload = async (enable) => {
		setBusy(true)
		setMsg('')
		try {
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
					await enableBackgroundSyncSafely()
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
					...(Array.isArray(prev)
						? prev.filter((b) => b.mode !== 'auto_upload')
						: []),
					binding,
				])
			}
			await enableBackgroundSyncSafely()
			await refresh()
			kickSyncNow()
		} catch (e) {
			setMsg(String(e))
			addAlert(String(e), 'error')
			await refresh()
		} finally {
			setBusy(false)
		}
	}

	const removeBinding = async (id) => {
		setBusy(true)
		try {
			await nativeInvoke('remove_binding', { id })
			await refresh()
		} catch (e) {
			addAlert(String(e), 'error')
		} finally {
			setBusy(false)
		}
	}

	const toggleFolderBinding = async (id, enabled) => {
		setBusy(true)
		try {
			await nativeInvoke('set_binding_enabled', { id, enabled })
			await refresh()
		} catch (e) {
			addAlert(String(e), 'error')
		} finally {
			setBusy(false)
		}
	}

	const runSyncNow = () => {
		setMsg('')
		addAlert('Upload started', 'success')
		kickSyncNow()
	}

	const modeLabel = (mode) => {
		if (mode === 'auto_upload') return 'Camera auto-upload'
		if (mode === 'folder_upload') return 'Folder auto-upload'
		if (mode === 'sync') return 'Legacy two-way sync'
		return mode
	}

	return (
		<div class="settings-sync-panel">
			<Show
				when={queueView()}
				fallback={
					<>
			<p class="settings-bot-hint">
				Photo and video auto-upload goes to remote{' '}
				<code>
					{desiredCameraRemoteRoot()
						? `${desiredCameraRemoteRoot()}/`
						: 'Camera/…/'}
				</code>
				.
			</p>

			<div class="settings-toggle">
				<span>Enable photo and video auto-upload</span>
				<Show
					when={bindingsLoaded() || cachedCameraOn() !== null}
					fallback={
						<CircularProgress
							size={22}
							color="secondary"
							aria-label="Loading auto-upload state"
						/>
					}
				>
					<SettingsSwitch
						id="settings-camera-switch"
						checked={cameraOn()}
						disabled={busy() || !bindingsLoaded()}
						onChange={(checked) => setAutoUpload(checked)}
					/>
				</Show>
			</div>

			<Show when={autoBinding()}>
				<p class="settings-sync-panel__meta">
					{autoBinding().local_path} → {autoBinding().remote_root || 'Camera'}
				</p>
				<Show when={cameraScanHint()}>
					<p class="settings-bot-hint">{cameraScanHint()}</p>
				</Show>
				<div class="settings-sync-panel__row">
					<Button
						variant="outlined"
						size="small"
						disabled={busy()}
						onClick={async () => {
							const path = await pickFolder(localPath())
							if (!path) return
							setLocalPath(path)
							try {
								const existing = cameraBinding(
									await nativeInvoke('list_bindings'),
								)
								if (existing) {
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
									await enableBackgroundSyncSafely()
									kickSyncNow()
									await refresh()
								} else {
									await setAutoUpload(true)
								}
							} catch (e) {
								setMsg(String(e))
								addAlert(String(e), 'error')
								await refresh()
							}
						}}
					>
						Change local folder
					</Button>
				</div>
			</Show>

			<Show when={autoBinding() && isMobile()}>
				<div class="settings-toggle">
					<span>Upload on Wi‑Fi only</span>
					<SettingsSwitch
						id="settings-wifi-switch"
						checked={prefs().wifi_only !== false}
						disabled={busy()}
						onChange={(checked) =>
							savePrefs({ ...prefs(), wifi_only: checked })
						}
					/>
				</div>
			</Show>

			<div class="settings-toggle">
				<span>Background backup / sync</span>
				<SettingsSwitch
					id="settings-background-switch"
					checked={prefs().background_sync !== false}
					disabled={busy()}
					onChange={(checked) =>
						savePrefs({ ...prefs(), background_sync: checked })
					}
				/>
			</div>

			<div class="settings-sync-panel__section">
				<Typography variant="subtitle2" sx={{ mb: 1 }}>
					Upload &amp; download
				</Typography>
				<div class="settings-sync-panel__queue">
					<button
						type="button"
						class="settings-sync-panel__queue-row"
						onClick={() => setQueueView('download')}
					>
						<span>Downloading</span>
						<span class="settings-sync-panel__queue-count">
							{transferSnap().downloading}
							<FluentIcon name="chevronRight" size={16} aria-hidden="true" />
						</span>
					</button>
					<button
						type="button"
						class="settings-sync-panel__queue-row"
						onClick={() => setQueueView('upload')}
					>
						<span>Uploading</span>
						<span class="settings-sync-panel__queue-count">
							{transferSnap().uploading}
							<FluentIcon name="chevronRight" size={16} aria-hidden="true" />
						</span>
					</button>
				</div>
			</div>

			<div class="settings-sync-panel__section">
				<Typography variant="subtitle2" sx={{ mb: 1 }}>
					Folder bindings
				</Typography>
				<Show
					when={folderBindings().length}
					fallback={
						<p class="settings-account__hint">No folder bindings.</p>
					}
				>
					<ul class="settings-sync-panel__list">
						<For each={folderBindings()}>
							{(b) => (
								<li>
									<div>
										<strong>{modeLabel(b.mode)}</strong>
										<div class="settings-account__hint">{b.local_path}</div>
										<div class="settings-account__hint">
											{b.remote_root || '(root)'}
										</div>
									</div>
									<div class="settings-sync-panel__row">
										<SettingsSwitch
											id={`settings-folder-switch-${b.id}`}
											ariaLabel={`${modeLabel(b.mode)}: ${b.local_path}`}
											checked={b.enabled === true}
											disabled={busy()}
											onChange={(checked) =>
												toggleFolderBinding(b.id, checked)
											}
										/>
										<Button
											size="small"
											color="error"
											variant="outlined"
											disabled={busy()}
											onClick={() => removeBinding(b.id)}
										>
											Remove
										</Button>
									</div>
								</li>
							)}
						</For>
					</ul>
				</Show>
			</div>

			<div class="settings-sync-panel__row">
				<Button
					variant="contained"
					color="secondary"
					disabled={busy()}
					onClick={runSyncNow}
				>
					Upload now
				</Button>
			</div>

			<Show when={statuses().length}>
				<pre class="settings-sync-panel__status">
					{JSON.stringify(statuses(), null, 2)}
				</pre>
			</Show>
			<Show when={statuses().some((s) => s.last_error)}>
				<p class="settings-bot-hint" role="alert">
					{statuses()
						.filter((s) => s.last_error)
						.map((s) => s.last_error)
						.join(' · ')}
				</p>
			</Show>
			<Show when={msg()}>
				<p class="settings-bot-hint" role="status">
					{msg()}
				</p>
			</Show>
			<p class="settings-account__hint" style={{ display: 'none' }}>
				{formatBytes(0)}
			</p>
					</>
				}
			>
				<div class="settings-sync-panel__transfer">
					<button
						type="button"
						class="settings-sync-panel__transfer-back"
						onClick={() => setQueueView(null)}
					>
						<FluentIcon name="chevronLeft" size={18} aria-hidden="true" />
						<span>
							{queueView() === 'upload' ? 'Upload list' : 'Download list'}
						</span>
					</button>
					<Show
						when={queueItems().length}
						fallback={
							<p class="settings-account__hint">No transfers yet.</p>
						}
					>
						<ul class="settings-sync-panel__transfer-list">
							<For each={queueItems()}>
								{(item) => (
									<li class="settings-sync-panel__transfer-item">
										<div class="settings-sync-panel__transfer-meta">
											<Show when={item.path}>
												<div class="settings-account__hint">
													{item.path}/
												</div>
											</Show>
											<div class="settings-sync-panel__transfer-name">
												{item.name}
											</div>
											<Show when={item.size != null}>
												<div class="settings-account__hint">
													{formatBytes(Number(item.size) || 0)}
												</div>
											</Show>
										</div>
										<div
											class="settings-sync-panel__transfer-status"
											aria-label={item.status}
										>
											{statusIcon(item.status)}
										</div>
									</li>
								)}
							</For>
						</ul>
					</Show>
				</div>
			</Show>
		</div>
	)
}

export default SettingsSyncPanel
