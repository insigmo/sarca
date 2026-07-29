import { For, Show, createEffect, createSignal, onCleanup } from 'solid-js'
import { useNavigate } from '@solidjs/router'
import IconButton from '@suid/material/IconButton'
import Button from '@suid/material/Button'
import Box from '@suid/material/Box'
import TextField from '@suid/material/TextField'
import Typography from '@suid/material/Typography'
import API from '../api'
import createLocalStore from '../../libs'
import { clearSession } from '../common/auth'
import { settingsStore } from '../common/settings'
import { filesChromeStore } from '../common/filesChrome'
import { storageSettingsStore } from '../common/storageSettings'
import { formatBytes, nativeInvoke } from '../common/nativeBridge'
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
import SettingsSyncPanel from './SettingsSyncPanel'
import SettingsSwitch from './SettingsSwitch'
import AppLockToggle from './AppLockToggle'
import { nativeClientStore } from '../common/nativeClient'

const SettingsModal = () => {
	const { isOpen, closeSettings, tab, setTab } = settingsStore
	const { isNative, refresh: refreshNative } = nativeClientStore
	const chrome = filesChromeStore
	const { addAlert } = alertStore
	const [store, setStore] = createLocalStore()
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
	const [about, setAbout] = createSignal({ version: '', platform: '' })
	const [cacheBytes, setCacheBytes] = createSignal(0)
	const [cacheLimitBytes, setCacheLimitBytes] = createSignal(1_073_741_824)
	const [sessionInfo, setSessionInfo] = createSignal({ base_url: '', email: '' })
	const [lockEnabled, setLockEnabled] = createSignal(false)
	const [logsEnabled, setLogsEnabled] = createSignal(false)
	const [logsBusy, setLogsBusy] = createSignal(false)
	const [pinDraft, setPinDraft] = createSignal('')
	const [pinConfirm, setPinConfirm] = createSignal('')
	const [securityMsg, setSecurityMsg] = createSignal('')
	/** @type {[import("solid-js").Accessor<boolean>, any]} */
	const [isSuperuser, setIsSuperuser] = createSignal(!!store.user?.is_superuser)
	/** @type {[import("solid-js").Accessor<Array<{id: string, email: string, email_verified: boolean, is_superuser: boolean}>>, any]} */
	const [adminUsers, setAdminUsers] = createSignal([])
	const [newUserEmail, setNewUserEmail] = createSignal('')
	const [newUserPassword, setNewUserPassword] = createSignal('')
	const [usersBusy, setUsersBusy] = createSignal(false)

	const showSyncTab = () => isNative() && Boolean(chrome.storageId())
	const showUsersTab = () => isSuperuser()

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

	const refreshSuperuser = async () => {
		try {
			const me = await API.auth.meSilent()
			const su = !!me?.is_superuser
			setIsSuperuser(su)
			if (me) {
				setStore('user', {
					email: me.email,
					email_verified: me.email_verified,
					is_superuser: su,
				})
			}
			return su
		} catch {
			setIsSuperuser(false)
			return false
		}
	}

	const fetchAdminUsers = async () => {
		try {
			const data = await API.users.listUsers()
			setAdminUsers(data?.users || [])
		} catch (err) {
			console.error(err)
			setAdminUsers([])
		}
	}

	const createAdminUser = async (event) => {
		event.preventDefault()
		const email = newUserEmail().trim()
		const password = newUserPassword()
		if (!email || !password) {
			addAlert('Email and password are required', 'error')
			return
		}
		setUsersBusy(true)
		try {
			await API.users.createUser(email, password)
			setNewUserEmail('')
			setNewUserPassword('')
			addAlert('User created', 'success')
			await fetchAdminUsers()
		} catch (err) {
			console.error(err)
		} finally {
			setUsersBusy(false)
		}
	}

	createEffect(() => {
		if (!isOpen()) return

		// Re-check after late native inject (Android remote WebView).
		refreshNative()
		refreshStorages()
		refreshSuperuser()
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
		if (!isOpen() || tab() !== 'users') return
		refreshSuperuser().then((su) => {
			if (su) fetchAdminUsers()
			else setTab('general')
		})
	})

	createEffect(() => {
		if (!showUsersTab() && tab() === 'users') setTab('general')
	})

	createEffect(() => {
		if (!isOpen() || tab() !== 'trash') return
		API.settings
			.getTrashSettings()
			.then((s) => setTrashRetentionDays(s.retention_days))
			.catch(() => {})
	})

	createEffect(() => {
		if (!isOpen() || tab() !== 'general') return
		const openId = chrome.storageId()
		if (openId) refreshStorages()
		if (isNative()) {
			nativeInvoke('get_about')
				.then((a) => setAbout(a || { version: '', platform: '' }))
				.catch(() => {})
			nativeInvoke('get_cache_size')
				.then((c) => {
					setCacheBytes(Number(c?.bytes) || 0)
					setCacheLimitBytes(Number(c?.limit_bytes) || 1_073_741_824)
				})
				.catch(() => {
					setCacheBytes(0)
					setCacheLimitBytes(1_073_741_824)
				})
			nativeInvoke('get_session')
				.then((s) =>
					setSessionInfo({
						base_url: s?.base_url || '',
						email: s?.email || '',
					}),
				)
				.catch(() => {})
			nativeInvoke('get_client_prefs')
				.then((p) => setLogsEnabled(Boolean(p?.enable_logs)))
				.catch(() => {})
		}
	})

	createEffect(() => {
		if (!isOpen() || tab() !== 'security' || !isNative()) return
		nativeInvoke('get_client_prefs')
			.then((p) => setLockEnabled(Boolean(p?.app_lock_enabled)))
			.catch(() => {})
	})

	createEffect(() => {
		if (!showSyncTab() && tab() === 'sync') setTab('general')
	})

	const occupiedGb = () => {
		const id = chrome.storageId()
		const s = storages().find((x) => x.id === id)
		const bytes = Number(s?.size) || 0
		return (bytes / 1024 ** 3).toFixed(2)
	}

	const clearCache = async () => {
		try {
			await nativeInvoke('clear_local_cache')
			setCacheBytes(0)
			addAlert('Cache cleared', 'success')
		} catch (e) {
			addAlert(String(e), 'error')
		}
	}

	const setEnableLogs = async (enabled) => {
		setLogsBusy(true)
		try {
			const prefs = (await nativeInvoke('get_client_prefs')) || {}
			const next = { ...prefs, enable_logs: enabled }
			await nativeInvoke('set_client_prefs', { prefs: next })
			setLogsEnabled(enabled)
		} catch (e) {
			addAlert(String(e), 'error')
		} finally {
			setLogsBusy(false)
		}
	}

	const exportLogs = async () => {
		setLogsBusy(true)
		try {
			const result = await nativeInvoke('export_logs')
			if (result?.shared) {
				addAlert('Share sheet opened', 'success')
				return
			}
			const content = String(result?.content || '')
			const path = String(result?.path || '')
			if (content && typeof document !== 'undefined') {
				const blob = new Blob([content], { type: 'text/plain;charset=utf-8' })
				const url = URL.createObjectURL(blob)
				const a = document.createElement('a')
				a.href = url
				a.download = 'sarca-client.log'
				a.rel = 'noopener'
				document.body.appendChild(a)
				a.click()
				a.remove()
				URL.revokeObjectURL(url)
			}
			addAlert(path ? `Logs exported (${path})` : 'Logs exported', 'success')
			setLogsEnabled(true)
		} catch (e) {
			addAlert(String(e), 'error')
		} finally {
			setLogsBusy(false)
		}
	}

	const saveAppLock = async (enabled) => {
		setSecurityMsg('')
		try {
			const prefs = (await nativeInvoke('get_client_prefs')) || {}
			if (enabled) {
				const pin = pinDraft().trim()
				const confirm = pinConfirm().trim()
				if (!/^\d{4,8}$/.test(pin)) {
					setSecurityMsg('PIN must be 4–8 digits')
					return
				}
				if (pin !== confirm) {
					setSecurityMsg('PIN confirmation does not match')
					return
				}
				await nativeInvoke('set_client_prefs', {
					prefs: {
						...prefs,
						app_lock_enabled: true,
						app_lock_pin: pin,
					},
				})
				setLockEnabled(true)
				setPinDraft('')
				setPinConfirm('')
				addAlert('App lock enabled', 'success')
			} else {
				const pin = pinDraft().trim()
				if (!pin || pin !== prefs.app_lock_pin) {
					setSecurityMsg('Enter current PIN to disable')
					return
				}
				await nativeInvoke('set_client_prefs', {
					prefs: {
						...prefs,
						app_lock_enabled: false,
						app_lock_pin: null,
					},
				})
				setLockEnabled(false)
				setPinDraft('')
				setPinConfirm('')
				addAlert('App lock disabled', 'success')
			}
		} catch (e) {
			setSecurityMsg(String(e))
		}
	}

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
									{isNative()
										? 'General, access, users, sync, trash, storage, and security'
										: 'General, access, users, trash, storage, and security'}
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
								<Show when={showUsersTab()}>
									<button
										type="button"
										class="settings-nav__item"
										classList={{ 'settings-nav__item--active': tab() === 'users' }}
										onClick={() => setTab('users')}
									>
										<span class="settings-nav__icon" aria-hidden="true">
											<FluentIcon
												name={tab() === 'users' ? 'personFilled' : 'person'}
												size={20}
											/>
										</span>
										<span class="settings-nav__text">
											<span class="settings-nav__title">Users</span>
											<span class="settings-nav__desc">Create accounts</span>
										</span>
									</button>
								</Show>
								<Show when={showSyncTab()}>
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
								<button
									type="button"
									class="settings-nav__item"
									classList={{
										'settings-nav__item--active': tab() === 'security',
									}}
									onClick={() => setTab('security')}
								>
									<span class="settings-nav__icon" aria-hidden="true">
										<FluentIcon
											name={
												tab() === 'security'
													? 'lockClosedFilled'
													: 'lockClosed'
											}
											size={20}
										/>
									</span>
									<span class="settings-nav__text">
										<span class="settings-nav__title">Security</span>
										<span class="settings-nav__desc">App lock</span>
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

								<Show when={tab() === 'users' && showUsersTab()}>
									<div class="settings-users">
										<Typography
											variant="body2"
											color="text.secondary"
											sx={{ mb: 2 }}
										>
											Only the superuser can create accounts. New users can sign in
											with email and password.
										</Typography>
										<Box
											component="form"
											onSubmit={createAdminUser}
											sx={{
												display: 'flex',
												flexDirection: 'column',
												gap: 1.5,
												mb: 3,
											}}
										>
											<TextField
												label="Email"
												type="email"
												required
												value={newUserEmail()}
												onChange={(e) => setNewUserEmail(e.target.value)}
											/>
											<TextField
												label="Password"
												type="password"
												required
												autoComplete="new-password"
												value={newUserPassword()}
												onChange={(e) => setNewUserPassword(e.target.value)}
											/>
											<Button
												type="submit"
												variant="contained"
												color="secondary"
												disabled={usersBusy()}
											>
												Create user
											</Button>
										</Box>
										<div class="settings-users__list">
											<For
												each={adminUsers()}
												fallback={
													<Typography color="text.secondary">
														No users yet.
													</Typography>
												}
											>
												{(u) => (
													<div class="settings-users__row">
														<div>
															<strong>{u.email}</strong>
															<Show when={u.is_superuser}>
																<span class="settings-users__badge">
																	superuser
																</span>
															</Show>
														</div>
														<span class="settings-users__meta">
															{u.email_verified ? 'verified' : 'unverified'}
														</span>
													</div>
												)}
											</For>
										</div>
									</div>
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
										<div class="settings-account__row">
											<div>
												<p class="settings-account__label">Account</p>
												<p class="settings-account__hint">
													{store.user?.email ||
														sessionInfo().email ||
														'Signed in'}
												</p>
											</div>
										</div>
										<Show when={isNative() && sessionInfo().base_url}>
											<div class="settings-account__row">
												<div>
													<p class="settings-account__label">Server</p>
													<p class="settings-account__hint">
														{sessionInfo().base_url}
													</p>
												</div>
											</div>
										</Show>
										<Show when={chrome.storageId()}>
											<div class="settings-account__row">
												<div>
													<p class="settings-account__label">Occupied space</p>
													<p class="settings-account__hint">
														{occupiedGb()} GB used
													</p>
												</div>
											</div>
										</Show>
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
										<Show when={isNative()}>
											<div class="settings-account__row">
												<div>
													<p class="settings-account__label">Cache</p>
													<p class="settings-account__hint">
														{formatBytes(cacheBytes())} /{' '}
														{formatBytes(cacheLimitBytes())}
													</p>
												</div>
												<Button variant="outlined" onClick={clearCache}>
													Clear cache
												</Button>
											</div>
											<div class="settings-account__row">
												<div>
													<p class="settings-account__label">About</p>
													<p class="settings-account__hint">
														Sarca client {about().version || '—'} ·{' '}
														{about().platform || 'native'}
													</p>
												</div>
											</div>
											<div class="settings-toggle">
												<span>Enable logs</span>
												<SettingsSwitch
													id="settings-enable-logs-switch"
													checked={logsEnabled()}
													disabled={logsBusy()}
													onChange={(checked) => setEnableLogs(checked)}
												/>
											</div>
											<div class="settings-account__row">
												<div>
													<p class="settings-account__label">Export logs</p>
													<p class="settings-account__hint">
														Share a log file for debugging auto-upload
													</p>
												</div>
												<Button
													variant="outlined"
													disabled={logsBusy()}
													onClick={exportLogs}
												>
													Export logs
												</Button>
											</div>
										</Show>
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

								<Show when={tab() === 'sync' && showSyncTab()}>
									<SettingsSyncPanel
										storageId={chrome.storageId()}
										storageName={chrome.storageName()}
									/>
								</Show>

								<Show when={tab() === 'security'}>
									<div class="settings-account">
										<p class="settings-bot-hint">
											Require a PIN when opening the native app on this device.
										</p>
										<Show
											when={isNative()}
											fallback={
												<p class="settings-account__hint">
													App lock is available in the Sarca native client.
												</p>
											}
										>
											<AppLockToggle
												checked={lockEnabled()}
												onChange={(on) => {
													if (on) {
														setLockEnabled(true)
														setSecurityMsg('Enter a new PIN below, then save')
													} else {
														saveAppLock(false)
													}
												}}
											/>
											<TextField
												label={
													lockEnabled()
														? 'PIN (new or current)'
														: 'PIN (4–8 digits)'
												}
												type="password"
												size="small"
												fullWidth
												sx={{ mt: 1 }}
												value={pinDraft()}
												onChange={(_, v) => setPinDraft(v)}
												inputProps={{ inputMode: 'numeric', maxLength: 8 }}
											/>
											<TextField
												label="Confirm new PIN"
												type="password"
												size="small"
												fullWidth
												sx={{ mt: 1 }}
												value={pinConfirm()}
												onChange={(_, v) => setPinConfirm(v)}
												inputProps={{ inputMode: 'numeric', maxLength: 8 }}
											/>
											<div class="settings-sync-panel__row" style={{ 'margin-top': '8px' }}>
												<Button
													variant="contained"
													color="secondary"
													onClick={() => saveAppLock(true)}
												>
													{lockEnabled() ? 'Save PIN' : 'Enable lock'}
												</Button>
												<Show when={lockEnabled()}>
													<Button
														variant="outlined"
														color="error"
														onClick={() => saveAppLock(false)}
													>
														Disable
													</Button>
												</Show>
											</div>
											<Show when={securityMsg()}>
												<p class="settings-bot-hint" role="status">
													{securityMsg()}
												</p>
											</Show>
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
