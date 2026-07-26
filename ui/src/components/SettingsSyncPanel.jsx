import { For, Show, createSignal, onCleanup, onMount } from 'solid-js'
import Button from '@suid/material/Button'
import TextField from '@suid/material/TextField'
import Typography from '@suid/material/Typography'

import {
	formatBytes,
	isMobileNativePlatform,
	nativeInvoke,
	pickLocalFolder,
} from '../common/nativeBridge'
import { filesChromeStore } from '../common/filesChrome'
import { alertStore } from './AlertStack'

/**
 * Full Sync tab: auto-upload to Camera/, Wi‑Fi-only, folder backup, background, sync now.
 * Storage is locked to the currently open Files storage.
 * @param {{ storageId?: string, storageName?: string }} props
 */
const SettingsSyncPanel = (props) => {
	const { addAlert } = alertStore
	const chrome = filesChromeStore
	const [platform, setPlatform] = createSignal('')
	const [bindings, setBindings] = createSignal([])
	const [statuses, setStatuses] = createSignal([])
	const [prefs, setPrefs] = createSignal({
		wifi_only: true,
		background_sync: true,
		app_lock_enabled: false,
		app_lock_pin: null,
	})
	const [localPath, setLocalPath] = createSignal('')
	const [remoteRoot, setRemoteRoot] = createSignal('')
	const [newFolderName, setNewFolderName] = createSignal('')
	const [busy, setBusy] = createSignal(false)
	const [msg, setMsg] = createSignal('')

	const isMobile = () => isMobileNativePlatform(platform())

	const lockedStorageId = () => props.storageId || chrome.storageId() || ''
	const lockedStorageName = () =>
		props.storageName ||
		chrome.storageName() ||
		(lockedStorageId() ? 'Current storage' : 'No storage open')

	const autoBinding = () => bindings().find((b) => b.mode === 'auto_upload')
	const syncBindings = () => bindings().filter((b) => b.mode !== 'auto_upload')

	const refresh = async () => {
		try {
			const [label, binds, prefsDto, statusList] = await Promise.all([
				nativeInvoke('platform_label').catch(() => ''),
				nativeInvoke('list_bindings').catch(() => []),
				nativeInvoke('get_client_prefs').catch(() => null),
				nativeInvoke('sync_statuses').catch(() => []),
			])
			setPlatform(String(label || ''))
			setBindings(Array.isArray(binds) ? binds : [])
			setStatuses(Array.isArray(statusList) ? statusList : [])
			if (prefsDto && typeof prefsDto === 'object') {
				setPrefs({
					wifi_only: prefsDto.wifi_only !== false,
					background_sync: prefsDto.background_sync !== false,
					app_lock_enabled: Boolean(prefsDto.app_lock_enabled),
					app_lock_pin: prefsDto.app_lock_pin ?? null,
				})
			}
			const auto = (Array.isArray(binds) ? binds : []).find(
				(b) => b.mode === 'auto_upload',
			)
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

	const pickFolder = async () => {
		setBusy(true)
		setMsg('')
		try {
			const path = await pickLocalFolder(localPath())
			if (path) setLocalPath(String(path))
			else setMsg('No folder selected')
		} catch (e) {
			const text = String(e?.message || e)
			setMsg(text)
			addAlert(text || 'Folder picker failed', 'error')
		} finally {
			setBusy(false)
		}
	}

	const setAutoUpload = async (enabled) => {
		setBusy(true)
		setMsg('')
		try {
			const sid = lockedStorageId()
			if (!sid) throw new Error('Open a storage in Files first')
			const existing = bindings().filter((b) => b.mode === 'auto_upload')
			for (const b of existing) {
				await nativeInvoke('remove_binding', { id: b.id })
			}
			if (enabled) {
				let path = localPath().trim()
				if (!path) {
					path = String((await nativeInvoke('default_gallery_path')) || '')
					if (!path) {
						path = String((await pickLocalFolder('')) || '')
					}
					if (path) setLocalPath(path)
				}
				if (!path) throw new Error('Choose a local gallery / Pictures folder')
				const remote = await nativeInvoke('ensure_remote_folder', {
					storageId: sid,
					parent: '',
					name: 'Camera',
				})
				await nativeInvoke('add_binding', {
					storageId: sid,
					remoteRoot: String(remote).replace(/\/$/, '') || 'Camera',
					localPath: path,
					mode: 'auto_upload',
				})
			}
			await refresh()
		} catch (e) {
			setMsg(String(e))
			addAlert(String(e), 'error')
		} finally {
			setBusy(false)
		}
	}

	const addFolderSync = async () => {
		setBusy(true)
		setMsg('')
		try {
			const sid = lockedStorageId()
			let path = localPath().trim()
			if (!path) {
				path = String((await pickLocalFolder('')) || '')
				if (path) setLocalPath(path)
			}
			let remote = remoteRoot().trim().replace(/\/$/, '')
			const name = newFolderName().trim()
			if (name) {
				remote = String(
					await nativeInvoke('ensure_remote_folder', {
						storageId: sid,
						parent: remote,
						name,
					}),
				).replace(/\/$/, '')
				setRemoteRoot(remote)
				setNewFolderName('')
			}
			if (!sid) throw new Error('Open a storage in Files first')
			if (!path) throw new Error('Set a local folder')
			if (!remote) throw new Error('Set a remote folder path or create one')
			await nativeInvoke('add_binding', {
				storageId: sid,
				remoteRoot: remote,
				localPath: path,
				mode: 'sync',
			})
			await refresh()
		} catch (e) {
			setMsg(String(e))
			addAlert(String(e), 'error')
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

	const runSyncNow = async () => {
		setBusy(true)
		setMsg('')
		try {
			await nativeInvoke('sync_now')
			await refresh()
			addAlert('Sync started', 'success')
		} catch (e) {
			setMsg(String(e))
			addAlert(String(e), 'error')
		} finally {
			setBusy(false)
		}
	}

	return (
		<div class="settings-sync-panel">
			<p class="settings-bot-hint">
				Photo and video auto-upload goes to remote <code>Camera/</code>. Folder
				backup uses two-way sync bindings.
			</p>

			<label class="settings-toggle">
				<span>Включить автозагрузку фото и видео</span>
				<input
					type="checkbox"
					checked={Boolean(autoBinding())}
					disabled={busy()}
					onChange={(e) => setAutoUpload(e.currentTarget.checked)}
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
							await pickFolder()
							if (localPath().trim()) await setAutoUpload(true)
						}}
					>
						Change local folder
					</Button>
				</div>
			</Show>

			<Show when={autoBinding() && isMobile()}>
				<label class="settings-toggle">
					<span>Загружать только через WIFI</span>
					<input
						type="checkbox"
						checked={prefs().wifi_only !== false}
						disabled={busy()}
						onChange={(e) =>
							savePrefs({ ...prefs(), wifi_only: e.currentTarget.checked })
						}
					/>
				</label>
			</Show>

			<label class="settings-toggle">
				<span>Background backup / sync</span>
				<input
					type="checkbox"
					checked={prefs().background_sync !== false}
					disabled={busy()}
					onChange={(e) =>
						savePrefs({ ...prefs(), background_sync: e.currentTarget.checked })
					}
				/>
			</label>

			<div class="settings-sync-panel__section">
				<Typography variant="subtitle2" sx={{ mb: 1 }}>
					Folder backup
				</Typography>
				<div class="settings-select-field">
					<span class="settings-select-field__label">Storage</span>
					<p
						class="settings-sync-panel__storage-locked"
						title={lockedStorageId()}
					>
						{lockedStorageName()}
					</p>
				</div>
				<div class="settings-sync-panel__row">
					<TextField
						label="Local folder"
						size="small"
						fullWidth
						value={localPath()}
						onChange={(_, v) => setLocalPath(v)}
						disabled={busy()}
					/>
					<Button
						variant="outlined"
						size="small"
						disabled={busy()}
						onClick={pickFolder}
					>
						Browse…
					</Button>
				</div>
				<TextField
					label="Remote folder path"
					size="small"
					fullWidth
					sx={{ mt: 1 }}
					value={remoteRoot()}
					onChange={(_, v) => setRemoteRoot(v)}
					disabled={busy()}
					placeholder="e.g. Documents/Notes"
				/>
				<div class="settings-sync-panel__row" style={{ 'margin-top': '8px' }}>
					<TextField
						label="Or create remote folder"
						size="small"
						fullWidth
						value={newFolderName()}
						onChange={(_, v) => setNewFolderName(v)}
						disabled={busy()}
					/>
				</div>
				<Button
					variant="contained"
					color="secondary"
					sx={{ mt: 1 }}
					disabled={busy()}
					onClick={addFolderSync}
				>
					Add folder sync
				</Button>
			</div>

			<div class="settings-sync-panel__section">
				<Typography variant="subtitle2" sx={{ mb: 1 }}>
					Bindings
				</Typography>
				<Show
					when={syncBindings().length}
					fallback={
						<p class="settings-account__hint">No folder sync bindings yet.</p>
					}
				>
					<ul class="settings-sync-panel__list">
						<For each={syncBindings()}>
							{(b) => (
								<li>
									<div>
										<strong>{b.mode}</strong>
										<div class="settings-account__hint">{b.local_path}</div>
										<div class="settings-account__hint">
											{b.remote_root || '(root)'}
										</div>
									</div>
									<Button
										size="small"
										color="error"
										variant="outlined"
										disabled={busy()}
										onClick={() => removeBinding(b.id)}
									>
										Remove
									</Button>
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
					Sync now
				</Button>
			</div>

			<Show when={statuses().length}>
				<pre class="settings-sync-panel__status">
					{JSON.stringify(statuses(), null, 2)}
				</pre>
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
