import { For, Show, createSignal, onCleanup, onMount } from 'solid-js'
import Button from '@suid/material/Button'
import Typography from '@suid/material/Typography'

import {
	formatBytes,
	isMobileNativePlatform,
	nativeInvoke,
	pickLocalFolder,
} from '../common/nativeBridge'
import {
	cameraBinding,
	cameraRemoteRoot,
	resolveCameraToggle,
	withBackgroundSyncOn,
} from '../common/autoUploadActions'
import { filesChromeStore } from '../common/filesChrome'
import { alertStore } from './AlertStack'
import SettingsSwitch from './SettingsSwitch'

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

	const isMobile = () => isMobileNativePlatform(platform())

	const lockedStorageId = () => props.storageId || chrome.storageId() || ''

	// Camera row can exist while soft-disabled — never removed on toggle-off.
	const autoBinding = () => cameraBinding(bindings())
	const cameraOn = () => autoBinding()?.enabled === true
	const folderBindings = () =>
		bindings().filter(
			(b) => b.mode === 'folder_upload' || b.mode === 'sync',
		)
	const desiredCameraRemoteRoot = () =>
		cameraRemoteRoot(deviceLabel().trim() || platform().trim() || '')

	const refresh = async () => {
		try {
			const [label, device, bindsResult, prefsDto, statusList] = await Promise.all([
				nativeInvoke('platform_label').catch(() => ''),
				nativeInvoke('device_label').catch(() => ''),
				nativeInvoke('list_bindings').then(
					(v) => ({ ok: true, value: v }),
					(e) => ({ ok: false, error: e }),
				),
				nativeInvoke('get_client_prefs').catch(() => null),
				nativeInvoke('sync_statuses').catch(() => []),
			])
			setPlatform(String(label || ''))
			setDeviceLabel(String(device || ''))
			if (bindsResult.ok) {
				setBindings(Array.isArray(bindsResult.value) ? bindsResult.value : [])
			} else {
				setMsg(String(bindsResult.error))
			}
			setStatuses(Array.isArray(statusList) ? statusList : [])
			if (prefsDto && typeof prefsDto === 'object') {
				setPrefs({
					wifi_only: prefsDto.wifi_only !== false,
					background_sync: prefsDto.background_sync !== false,
					app_lock_enabled: Boolean(prefsDto.app_lock_enabled),
					app_lock_pin: prefsDto.app_lock_pin ?? null,
				})
				setPrefsLoaded(true)
			}
			const binds = bindsResult.ok
				? Array.isArray(bindsResult.value)
					? bindsResult.value
					: []
				: bindings()
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
		} catch (e) {
			setMsg(String(e))
		}
	}

	onMount(() => {
		refresh()
		const id = window.setInterval(() => {
			refresh()
		}, 8000)
		onCleanup(() => window.clearInterval(id))
	})

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

			const decision = resolveCameraToggle(live, enable)

			if (decision.action === 'noop') {
				setBindings(live)
				return
			}

			if (decision.action === 'set_enabled') {
				// Soft-disable / re-enable in place — never remove_binding, so the
				// index and remote mapping survive a toggle-off/on cycle.
				await nativeInvoke('set_binding_enabled', {
					id: decision.id,
					enabled: decision.enabled,
				})
				if (decision.enabled) {
					const existing = live.find((b) => b.id === decision.id) || null
					const expectedRoot = desiredCameraRemoteRoot()
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
			const expectedRoot = desiredCameraRemoteRoot()
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
			<p class="settings-bot-hint">
				Photo and video auto-upload goes to remote <code>{desiredCameraRemoteRoot()}/</code>.
			</p>

			<label class="settings-toggle">
				<span>Включить автозагрузку фото и видео</span>
				<SettingsSwitch
					id="settings-camera-switch"
					checked={cameraOn()}
					disabled={busy()}
					onChange={(checked) => setAutoUpload(checked)}
				/>
			</label>

			<Show when={autoBinding()}>
				<p class="settings-sync-panel__meta">
					{autoBinding().local_path} → {autoBinding().remote_root || 'Camera'}
				</p>
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
				<label class="settings-toggle">
					<span>Загружать только через WIFI</span>
					<SettingsSwitch
						id="settings-wifi-switch"
						checked={prefs().wifi_only !== false}
						disabled={busy()}
						onChange={(checked) =>
							savePrefs({ ...prefs(), wifi_only: checked })
						}
					/>
				</label>
			</Show>

			<label class="settings-toggle">
				<span>Background backup / sync</span>
				<SettingsSwitch
					id="settings-background-switch"
					checked={prefs().background_sync !== false}
					disabled={busy()}
					onChange={(checked) =>
						savePrefs({ ...prefs(), background_sync: checked })
					}
				/>
			</label>

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
		</div>
	)
}

export default SettingsSyncPanel
