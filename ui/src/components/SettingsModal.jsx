import { For, Show, createEffect, createSignal, onCleanup } from 'solid-js'
import { useNavigate } from '@solidjs/router'
import IconButton from '@suid/material/IconButton'
import Button from '@suid/material/Button'
import TextField from '@suid/material/TextField'
import Typography from '@suid/material/Typography'
import API from '../api'
import createLocalStore from '../../libs'
import { clearSession } from '../common/auth'
import { settingsStore } from '../common/settings'
import { filesChromeStore } from '../common/filesChrome'
import { storageSettingsStore } from '../common/storageSettings'
import {
	THEMES,
	setThemeMode,
	themeHints,
	themeLabels,
	useThemeMode,
} from '../common/theme'
import { alertStore } from './AlertStack'
import Access from './Access'
import FluentIcon from './FluentIcon'
import GrantAccess from './GrantAccess'

const SettingsModal = () => {
	const { isOpen, closeSettings, tab, setTab } = settingsStore
	const chrome = filesChromeStore
	const { addAlert } = alertStore
	const [, setStore] = createLocalStore()
	const navigate = useNavigate()
	const mode = useThemeMode()
	const { open: openStorageSettings } = storageSettingsStore

	const isNativeClient = () => {
		try {
			return localStorage.getItem('sarca_native') === '1'
		} catch {
			return false
		}
	}

	const openNativeSyncSettings = (event) => {
		event?.preventDefault?.()
		// Cross-origin webview cannot invoke; Rust intercepts this scheme.
		window.location.assign('sarca-sync://open')
	}

	/** @type {[import("solid-js").Accessor<import("../api").StorageWithInfo[]>, any]} */
	const [storages, setStorages] = createSignal([])
	/** Access tab */
	const [accessStorageId, setAccessStorageId] = createSignal('')
	const [accessUsers, setAccessUsers] = createSignal([])
	const [canManageAccess, setCanManageAccess] = createSignal(false)
	const [isGrantVisible, setIsGrantVisible] = createSignal(false)
	const [trashRetentionDays, setTrashRetentionDays] = createSignal(30)
	const [trashSettingsSaving, setTrashSettingsSaving] = createSignal(false)

	const logout = () => {
		closeSettings()
		navigate(clearSession(setStore))
	}

	const openCurrentStorageSettings = () => {
		const id = chrome.storageId()
		if (!id) return

		closeSettings()
		openStorageSettings({
			id,
			name: chrome.storageName() || 'Storage',
		})
	}

	const refreshStorages = async () => {
		try {
			const storagesSchema = await API.storages.listStorages()
			setStorages(storagesSchema.storages)
			const preferred = chrome.storageId() || storagesSchema.storages[0]?.id || ''
			// Only seed once when nothing is selected — never snap back on user change.
			if (!accessStorageId() && preferred) {
				setAccessStorageId(preferred)
			}
		} catch (err) {
			console.error(err)
		}
	}

	const fetchAccessUsers = async () => {
		const id = accessStorageId()
		if (!id) {
			setAccessUsers([])
			setCanManageAccess(false)
			return
		}
		try {
			const users = await API.access.listUsersWithAccess(id)
			setAccessUsers(users)
			setCanManageAccess(true)
		} catch (err) {
			console.error(err)
			setAccessUsers([])
			setCanManageAccess(false)
		}
	}

	createEffect(() => {
		if (!isOpen()) return

		refreshStorages()
		document.body.style.overflow = 'hidden'

		const onKeyDown = (e) => {
			if (e.key === 'Escape') closeSettings()
		}
		window.addEventListener('keydown', onKeyDown)

		onCleanup(() => {
			document.body.style.overflow = ''
			window.removeEventListener('keydown', onKeyDown)
		})
	})

	createEffect(() => {
		if (!isOpen() || tab() !== 'access') return
		accessStorageId()
		fetchAccessUsers()
	})

	createEffect(() => {
		if (!isOpen() || tab() !== 'trash') return
		API.settings
			.getTrashSettings()
			.then((s) => setTrashRetentionDays(s.retention_days))
			.catch(() => {})
	})

	const saveTrashSettings = async () => {
		const days = Number(trashRetentionDays())
		if (!Number.isFinite(days) || days < 1 || days > 30) {
			addAlert('Retention must be between 1 and 30 days', 'error')
			return
		}
		setTrashSettingsSaving(true)
		try {
			const s = await API.settings.setTrashSettings(days)
			setTrashRetentionDays(s.retention_days)
			addAlert('Trash settings saved', 'success')
		} finally {
			setTrashSettingsSaving(false)
		}
	}

	return (
		<>
			<Show when={isOpen()}>
				<div
					class="settings-overlay"
					onClick={(e) => {
						if (e.target === e.currentTarget) closeSettings()
					}}
					role="presentation"
				>
					<div
						class="settings-modal"
						role="dialog"
						aria-modal="true"
						aria-labelledby="settings-modal-title"
						onClick={(e) => e.stopPropagation()}
					>
						<div class="settings-modal__header">
							<div>
								<h2 id="settings-modal-title">Settings</h2>
								<p class="settings-modal__sub">
									{isNativeClient()
										? 'General, access, trash, storage, and sync'
										: 'General, access, trash, and storage'}
								</p>
							</div>
							<IconButton
								aria-label="Close settings"
								onClick={closeSettings}
								class="sarca-header-icon"
								size="small"
							>
								<FluentIcon name="dismiss" size={20} />
							</IconButton>
						</div>

						<div class="settings-modal__layout">
							<nav class="settings-nav" aria-label="Settings sections">
								<p class="settings-nav__label">Menu</p>
								<button
									type="button"
									class="settings-nav__item"
									classList={{ 'settings-nav__item--active': tab() === 'general' }}
									onClick={() => setTab('general')}
								>
									<span class="settings-nav__icon" aria-hidden="true">
										<FluentIcon
											name={tab() === 'general' ? 'personFilled' : 'person'}
											size={20}
										/>
									</span>
									<span class="settings-nav__text">
										<span class="settings-nav__title">General</span>
										<span class="settings-nav__desc">Theme &amp; session</span>
									</span>
								</button>
								<button
									type="button"
									class="settings-nav__item"
									classList={{ 'settings-nav__item--active': tab() === 'access' }}
									onClick={() => setTab('access')}
								>
									<span class="settings-nav__icon" aria-hidden="true">
										<FluentIcon
											name={
												tab() === 'access' ? 'lockClosedFilled' : 'lockClosed'
											}
											size={20}
										/>
									</span>
									<span class="settings-nav__text">
										<span class="settings-nav__title">Access</span>
										<span class="settings-nav__desc">Who can open</span>
									</span>
								</button>
								<button
									type="button"
									class="settings-nav__item"
									classList={{ 'settings-nav__item--active': tab() === 'trash' }}
									onClick={() => setTab('trash')}
								>
									<span class="settings-nav__icon" aria-hidden="true">
										<FluentIcon
											name={tab() === 'trash' ? 'deleteFilled' : 'delete'}
											size={20}
										/>
									</span>
									<span class="settings-nav__text">
										<span class="settings-nav__title">Trash</span>
										<span class="settings-nav__desc">Auto-delete</span>
									</span>
								</button>
								<button
									type="button"
									class="settings-nav__item"
									classList={{ 'settings-nav__item--active': tab() === 'storage' }}
									onClick={() => setTab('storage')}
								>
									<span class="settings-nav__icon" aria-hidden="true">
										<FluentIcon
											name={tab() === 'storage' ? 'storageFilled' : 'storage'}
											size={20}
										/>
									</span>
									<span class="settings-nav__text">
										<span class="settings-nav__title">Storage</span>
										<span class="settings-nav__desc">Bot &amp; channels</span>
									</span>
								</button>
								<Show when={isNativeClient()}>
									<button
										type="button"
										class="settings-nav__item"
										classList={{ 'settings-nav__item--active': tab() === 'sync' }}
										onClick={() => setTab('sync')}
									>
										<span class="settings-nav__icon" aria-hidden="true">
											<FluentIcon
												name={tab() === 'sync' ? 'cloudFilled' : 'cloud'}
												size={20}
											/>
										</span>
										<span class="settings-nav__text">
											<span class="settings-nav__title">Sync</span>
											<span class="settings-nav__desc">
												Auto-upload &amp; folders
											</span>
										</span>
									</button>
								</Show>
							</nav>

							<div class="settings-modal__body">
								<Show when={tab() === 'access'}>
									<p class="settings-bot-hint">
										Telegram bot and channels are in{' '}
										<strong>Storage settings</strong> — use the gear on a storage
										card, open the <strong>Storage</strong> tab here while browsing
										files, or (on desktop) the tune icon in the header.
									</p>

									<div class="settings-access">
										<div class="settings-access__toolbar">
											<label class="settings-select-field">
												<span class="settings-select-field__label">Storage</span>
												<select
													class="settings-select"
													value={accessStorageId()}
													onChange={(e) =>
														setAccessStorageId(e.currentTarget.value)
													}
												>
													<Show when={!storages().length}>
														<option value="" disabled>
															No storages
														</option>
													</Show>
													<For each={storages()}>
														{(storage) => (
															<option value={storage.id}>{storage.name}</option>
														)}
													</For>
												</select>
											</label>
											<Show when={canManageAccess() && accessStorageId()}>
												<Button
													variant="contained"
													color="secondary"
													startIcon={<FluentIcon name="add" size={18} />}
													onClick={() => setIsGrantVisible(true)}
												>
													Grant access
												</Button>
											</Show>
										</div>

										<Show
											when={accessStorageId()}
											fallback={
												<Typography
													color="text.secondary"
													sx={{ py: 4, textAlign: 'center' }}
												>
													Select a storage to manage access.
												</Typography>
											}
										>
											<Show
												when={canManageAccess()}
												fallback={
													<Typography
														color="text.secondary"
														sx={{ py: 4, textAlign: 'center' }}
													>
														You do not have permissions to manage access for this
														storage.
													</Typography>
												}
											>
												<Access
													storageId={accessStorageId()}
													users={accessUsers()}
													onMount={fetchAccessUsers}
													refetchUsers={fetchAccessUsers}
												/>
											</Show>
										</Show>
									</div>

									<GrantAccess
										isVisible={isGrantVisible()}
										afterGrant={fetchAccessUsers}
										onClose={() => setIsGrantVisible(false)}
										storageId={accessStorageId()}
									/>
								</Show>

								<Show when={tab() === 'trash'}>
									<div class="settings-trash">
										<Typography
											variant="body2"
											color="text.secondary"
											sx={{ mb: 2 }}
										>
											Deleted files stay in the trash for this many days (1–30),
											then are permanently removed from Sarca and Telegram.
										</Typography>
										<TextField
											type="number"
											label="Days in trash"
											fullWidth
											inputProps={{ min: 1, max: 30, step: 1 }}
											value={trashRetentionDays()}
											onChange={(e) =>
												setTrashRetentionDays(Number(e.target.value))
											}
										/>
										<div style={{ 'margin-top': '16px' }}>
											<Button
												variant="contained"
												color="secondary"
												disabled={trashSettingsSaving()}
												onClick={saveTrashSettings}
											>
												Save
											</Button>
										</div>
									</div>
								</Show>

								<Show when={tab() === 'general'}>
									<div class="settings-account">
										<div class="settings-account__row settings-account__row--theme">
											<div>
												<p class="settings-account__label">Theme</p>
												<p class="settings-account__hint">
													{themeHints[mode()] ?? themeHints.light}
												</p>
											</div>
											<div
												class="theme-picker"
												role="radiogroup"
												aria-label="Theme"
											>
												<For each={[...THEMES]}>
													{(t) => (
														<button
															type="button"
															role="radio"
															aria-checked={mode() === t}
															class="theme-picker__option"
															classList={{
																'theme-picker__option--active':
																	mode() === t,
															}}
															onClick={() => setThemeMode(t)}
														>
															{themeLabels[t]}
														</button>
													)}
												</For>
											</div>
										</div>
										<div class="settings-account__row">
											<div>
												<p class="settings-account__label">Session</p>
												<p class="settings-account__hint">
													Sign out of Sarca on this device
												</p>
											</div>
											<Button
												variant="outlined"
												color="error"
												startIcon={<FluentIcon name="signOut" size={18} />}
												onClick={logout}
											>
												Log out
											</Button>
										</div>
									</div>
								</Show>

								<Show when={tab() === 'storage'}>
									<div class="settings-storage-tab">
										<Show
											when={chrome.storageId()}
											fallback={
												<>
													<Typography color="text.secondary" sx={{ mb: 2 }}>
														Open a storage first, then manage its bot and channels
														here.
													</Typography>
													<Button
														onClick={() => {
															closeSettings()
															navigate('/storages')
														}}
													>
														Go to storages
													</Button>
												</>
											}
										>
											<Typography color="text.secondary" sx={{ mb: 2 }}>
												Manage bot and channels for{' '}
												<strong>{chrome.storageName() || 'this storage'}</strong>.
											</Typography>
											<Button
												variant="contained"
												color="secondary"
												startIcon={<FluentIcon name="options" size={18} />}
												onClick={openCurrentStorageSettings}
											>
												Open storage settings
											</Button>
										</Show>
									</div>
								</Show>

								<Show when={tab() === 'sync'}>
									<div class="settings-sync">
										<p class="settings-bot-hint">
											Configure Media auto-upload and folder sync in the Sarca
											app. Bindings run in the background while you are connected.
										</p>
										<ul class="settings-sync__status">
											<li>
												Media auto-upload and folder sync are managed in the app.
											</li>
											<li>
												On desktop, you can also open Sync from the tray menu
												(Sync settings).
											</li>
										</ul>
										<Button
											variant="contained"
											color="secondary"
											onClick={openNativeSyncSettings}
										>
											Open Sync settings
										</Button>
									</div>
								</Show>
							</div>
						</div>
					</div>
				</div>
			</Show>
		</>
	)
}

export default SettingsModal
