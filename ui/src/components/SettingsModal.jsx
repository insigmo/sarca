import { For, Show, createEffect, createSignal, onCleanup } from 'solid-js'
import { useNavigate } from '@solidjs/router'
import IconButton from '@suid/material/IconButton'
import Button from '@suid/material/Button'
import TextField from '@suid/material/TextField'
import Typography from '@suid/material/Typography'
import CloseIcon from '@suid/icons-material/Close'
import AddIcon from '@suid/icons-material/Add'
import LockOutlinedIcon from '@suid/icons-material/LockOutlined'
import DeleteOutlineIcon from '@suid/icons-material/DeleteOutline'
import DarkModeOutlinedIcon from '@suid/icons-material/DarkModeOutlined'
import LightModeOutlinedIcon from '@suid/icons-material/LightModeOutlined'
import LogoutIcon from '@suid/icons-material/Logout'
import PersonOutlineIcon from '@suid/icons-material/PersonOutline'
import StorageOutlinedIcon from '@suid/icons-material/StorageOutlined'
import TuneOutlinedIcon from '@suid/icons-material/TuneOutlined'

import API from '../api'
import createLocalStore from '../../libs'
import { clearSession } from '../common/auth'
import { settingsStore } from '../common/settings'
import { filesChromeStore } from '../common/filesChrome'
import { storageSettingsStore } from '../common/storageSettings'
import { toggleThemeMode, useThemeMode } from '../common/theme'
import { alertStore } from './AlertStack'
import Access from './Access'
import GrantAccess from './GrantAccess'

const SettingsModal = () => {
	const { isOpen, closeSettings, tab, setTab } = settingsStore
	const chrome = filesChromeStore
	const { addAlert } = alertStore
	const [, setStore] = createLocalStore()
	const navigate = useNavigate()
	const mode = useThemeMode()
	const { open: openStorageSettings } = storageSettingsStore

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
									Access, trash, account, and storage
								</p>
							</div>
							<IconButton
								aria-label="Close settings"
								onClick={closeSettings}
								class="sarca-header-icon"
								size="small"
							>
								<CloseIcon />
							</IconButton>
						</div>

						<div class="settings-modal__layout">
							<nav class="settings-nav" aria-label="Settings sections">
								<p class="settings-nav__label">Menu</p>
								<button
									type="button"
									class="settings-nav__item"
									classList={{ 'settings-nav__item--active': tab() === 'access' }}
									onClick={() => setTab('access')}
								>
									<span class="settings-nav__icon" aria-hidden="true">
										<LockOutlinedIcon fontSize="small" />
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
										<DeleteOutlineIcon fontSize="small" />
									</span>
									<span class="settings-nav__text">
										<span class="settings-nav__title">Trash</span>
										<span class="settings-nav__desc">Auto-delete</span>
									</span>
								</button>
								<button
									type="button"
									class="settings-nav__item"
									classList={{ 'settings-nav__item--active': tab() === 'account' }}
									onClick={() => setTab('account')}
								>
									<span class="settings-nav__icon" aria-hidden="true">
										<PersonOutlineIcon fontSize="small" />
									</span>
									<span class="settings-nav__text">
										<span class="settings-nav__title">Account</span>
										<span class="settings-nav__desc">Theme &amp; session</span>
									</span>
								</button>
								<button
									type="button"
									class="settings-nav__item"
									classList={{ 'settings-nav__item--active': tab() === 'storage' }}
									onClick={() => setTab('storage')}
								>
									<span class="settings-nav__icon" aria-hidden="true">
										<StorageOutlinedIcon fontSize="small" />
									</span>
									<span class="settings-nav__text">
										<span class="settings-nav__title">Storage</span>
										<span class="settings-nav__desc">Bot &amp; channels</span>
									</span>
								</button>
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
													startIcon={<AddIcon />}
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

								<Show when={tab() === 'account'}>
									<div class="settings-account">
										<div class="settings-account__row">
											<div>
												<p class="settings-account__label">Theme</p>
												<p class="settings-account__hint">
													{mode() === 'dark' ? 'Dark' : 'Light'} mode
												</p>
											</div>
											<Button
												variant="outlined"
												startIcon={
													mode() === 'dark' ? (
														<LightModeOutlinedIcon />
													) : (
														<DarkModeOutlinedIcon />
													)
												}
												onClick={toggleThemeMode}
											>
												{mode() === 'dark' ? 'Use light' : 'Use dark'}
											</Button>
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
												startIcon={<LogoutIcon />}
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
												startIcon={<TuneOutlinedIcon />}
												onClick={openCurrentStorageSettings}
											>
												Open storage settings
											</Button>
										</Show>
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
