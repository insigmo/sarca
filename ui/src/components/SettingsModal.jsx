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
import { formatBytes, nativeInvoke } from '../common/nativeBridge'
import {
	THEMES,
	setThemeMode,
	themeHints,
	themeLabels,
	useThemeMode,
} from '../common/theme'
import { i18n, LOCALES } from '../common/i18n'
import { alertStore } from './AlertStack'
import Access from './Access'
import ActionConfirmDialog from './ActionConfirmDialog'
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

	/** @type {[import("solid-js").Accessor<import("../api").StorageWithInfo[]>, any]} */
	const [storages, setStorages] = createSignal([])
	/** Access tab */
	const [accessStorageId, setAccessStorageId] = createSignal('')
	const [accessUsers, setAccessUsers] = createSignal([])
	const [canManageAccess, setCanManageAccess] = createSignal(false)
	const [isGrantVisible, setIsGrantVisible] = createSignal(false)
	const [trashRetentionDays, setTrashRetentionDays] = createSignal(30)
	const [trashSettingsSaving, setTrashSettingsSaving] = createSignal(false)
	const [backupPassword, setBackupPassword] = createSignal('')
	const [backupBusy, setBackupBusy] = createSignal(false)
	/** @type {[import("solid-js").Accessor<File | null>, any]} */
	const [restoreFile, setRestoreFile] = createSignal(null)
	const [restorePassword, setRestorePassword] = createSignal('')
	const [restoreBusy, setRestoreBusy] = createSignal(false)
	const [restoreConfirmOpen, setRestoreConfirmOpen] = createSignal(false)
	/** @type {HTMLInputElement | undefined} */
	let restoreInput
	const [about, setAbout] = createSignal({ version: '', platform: '' })
	const [cacheBytes, setCacheBytes] = createSignal(0)
	const [cacheLimitBytes, setCacheLimitBytes] = createSignal(1_073_741_824)
	const [sessionInfo, setSessionInfo] = createSignal({ base_url: '', email: '' })
	const [lockEnabled, setLockEnabled] = createSignal(false)
	// True only while the user is mid-way through turning app lock on (PIN
	// typed but not yet saved). Kept separate from lockEnabled — which must
	// only ever reflect what is actually persisted natively — so a failed
	// or abandoned save can't leave the switch reading "on" for a lock that
	// was never actually enabled.
	const [enablingLock, setEnablingLock] = createSignal(false)
	const [logsEnabled, setLogsEnabled] = createSignal(false)
	const [logsBusy, setLogsBusy] = createSignal(false)
	const [pinDraft, setPinDraft] = createSignal('')
	const [pinConfirm, setPinConfirm] = createSignal('')
	// Whether a PIN is stored natively. The PIN itself is never readable from
	// JS: `get_client_prefs` reports this flag only, changing or clearing the
	// PIN requires `current_app_lock_pin`, and unlocking goes through
	// `verify_app_lock_pin`. Any page reaching the bridge would otherwise have
	// been able to read the PIN straight out of the prefs.
	const [pinSet, setPinSet] = createSignal(false)
	const [pinCurrent, setPinCurrent] = createSignal('')
	const [securityMsg, setSecurityMsg] = createSignal('')
	/** @type {[import("solid-js").Accessor<boolean>, any]} */
	const [isSuperuser, setIsSuperuser] = createSignal(!!store.user?.is_superuser)
	/** @type {[import("solid-js").Accessor<Array<{id: string, email: string, email_verified: boolean, is_superuser: boolean}>>, any]} */
	const [adminUsers, setAdminUsers] = createSignal([])
	const [newUserEmail, setNewUserEmail] = createSignal('')
	const [newUserPassword, setNewUserPassword] = createSignal('')
	const [usersBusy, setUsersBusy] = createSignal(false)
	// Which admin row currently has its "change password" field expanded, or
	// '' when none. Only one at a time — keeps the list from turning into a
	// wall of open forms.
	const [openPasswordRowId, setOpenPasswordRowId] = createSignal('')
	const [rowNewPassword, setRowNewPassword] = createSignal('')
	const [rowPasswordBusy, setRowPasswordBusy] = createSignal(false)
	const [ownCurrentPassword, setOwnCurrentPassword] = createSignal('')
	const [ownNewPassword, setOwnNewPassword] = createSignal('')
	const [ownConfirmPassword, setOwnConfirmPassword] = createSignal('')
	const [ownPasswordBusy, setOwnPasswordBusy] = createSignal(false)

	const showSyncTab = () => isNative() && Boolean(chrome.storageId())

	const logout = () => {
		// Revoke server-side first; the local clear must not wait on the network.
		API.auth.logout()
		closeSettings()
		navigate(clearSession(setStore))
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
			addAlert(i18n.t('settings.emailPasswordRequired'), 'error')
			return
		}
		setUsersBusy(true)
		try {
			await API.users.createUser(email, password)
			setNewUserEmail('')
			setNewUserPassword('')
			addAlert(i18n.t('settings.userCreated'), 'success')
			await fetchAdminUsers()
		} catch (err) {
			console.error(err)
		} finally {
			setUsersBusy(false)
		}
	}

	const submitRowPassword = async (event, userId) => {
		event.preventDefault()
		const next = rowNewPassword()
		if (!next) {
			addAlert(i18n.t('settings.newPasswordRequired'), 'error')
			return
		}
		setRowPasswordBusy(true)
		try {
			await API.users.setUserPassword(userId, next)
			setRowNewPassword('')
			setOpenPasswordRowId('')
			addAlert(i18n.t('settings.passwordChanged'), 'success')
		} catch (err) {
			console.error(err)
		} finally {
			setRowPasswordBusy(false)
		}
	}

	const toggleUserDisabled = async (u) => {
		try {
			await API.users.setUserDisabled(u.id, !u.disabled)
			addAlert(
				u.disabled
					? i18n.t('settings.userEnabled')
					: i18n.t('settings.userDisabled'),
				'success',
			)
			await fetchAdminUsers()
		} catch (err) {
			console.error(err)
		}
	}

	// Server revokes every prior session on a password change (its
	// sessions_valid_after moves forward), including the one making this
	// request — so the response carries a fresh token pair that must be
	// persisted the same way the login flow does, or this request's own
	// tokens go stale and the next call 401s.
	const changeOwnPassword = async (event) => {
		event.preventDefault()
		const current = ownCurrentPassword()
		const next = ownNewPassword()
		const confirm = ownConfirmPassword()
		if (!current || !next) {
			addAlert(i18n.t('settings.currentNewPasswordRequired'), 'error')
			return
		}
		if (next !== confirm) {
			addAlert(i18n.t('settings.newPasswordConfirmMismatch'), 'error')
			return
		}
		setOwnPasswordBusy(true)
		try {
			const tokenData = await API.users.changeMyPassword(current, next)
			setStore('access_token', tokenData.access_token)
			setStore('refresh_token', tokenData.refresh_token)
			setStore('user', {
				...store.user,
				email_verified: tokenData.email_verified,
			})
			setOwnCurrentPassword('')
			setOwnNewPassword('')
			setOwnConfirmPassword('')
			addAlert(i18n.t('settings.passwordChanged'), 'success')
		} catch (err) {
			console.error(err)
		} finally {
			setOwnPasswordBusy(false)
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
		refreshSuperuser().then((su) => {
			if (su) fetchAdminUsers()
		})
	})

	createEffect(() => {
		if (!isOpen() || tab() !== 'general') return
		const openId = chrome.storageId()
		if (openId) refreshStorages()
		// Folded in from the old 'trash' tab effect.
		API.settings
			.getTrashSettings()
			.then((s) => setTrashRetentionDays(s.retention_days))
			.catch(() => {})
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
			// Folded in from the old 'security' tab effect.
			nativeInvoke('get_client_prefs')
				.then((p) => {
					setLockEnabled(Boolean(p?.app_lock_enabled))
					setPinSet(Boolean(p?.app_lock_pin_set))
				})
				.catch(() => {})
		}
	})

	createEffect(() => {
		if (!showSyncTab() && tab() === 'sync') setTab('general')
	})

	// The backup tab is superuser-only. The check is async, so bounce only
	// once the answer is in — not while `isSuperuser()` is still its
	// pessimistic initial value, which would kill a deep link to the tab.
	createEffect(() => {
		if (!isOpen() || tab() !== 'backup') return
		refreshSuperuser().then((su) => {
			if (!su) setTab('general')
		})
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
			addAlert(i18n.t('settings.cacheCleared'), 'success')
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
				addAlert(i18n.t('settings.shareSheetOpened'), 'success')
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
			addAlert(
				path
					? i18n.t('settings.logsExportedWithPath', { path })
					: i18n.t('settings.logsExported'),
				'success',
			)
			setLogsEnabled(true)
		} catch (e) {
			addAlert(String(e), 'error')
		} finally {
			setLogsBusy(false)
		}
	}

	const clearPinFields = () => {
		setPinDraft('')
		setPinConfirm('')
		setPinCurrent('')
	}

	// Rust owns the comparison: a wrong `current_app_lock_pin` comes back as an
	// error from `set_client_prefs`, so there is nothing to check here beyond
	// the shape of the new PIN.
	const saveAppLock = async (enabled) => {
		setSecurityMsg('')
		try {
			const prefs = (await nativeInvoke('get_client_prefs')) || {}
			const current = pinCurrent().trim()
			if (pinSet() && !current) {
				setSecurityMsg(i18n.t('settings.enterCurrentPin'))
				return
			}
			if (enabled) {
				const pin = pinDraft().trim()
				const confirm = pinConfirm().trim()
				if (!/^\d{4,8}$/.test(pin)) {
					setSecurityMsg(i18n.t('settings.pinLengthError'))
					return
				}
				if (pin !== confirm) {
					setSecurityMsg(i18n.t('settings.pinConfirmMismatch'))
					return
				}
				await nativeInvoke('set_client_prefs', {
					prefs: {
						...prefs,
						app_lock_enabled: true,
						app_lock_pin: pin,
						current_app_lock_pin: current || null,
					},
				})
				setLockEnabled(true)
				setPinSet(true)
				setEnablingLock(false)
				clearPinFields()
				addAlert(i18n.t('settings.appLockEnabled'), 'success')
			} else {
				await nativeInvoke('set_client_prefs', {
					prefs: {
						...prefs,
						app_lock_enabled: false,
						app_lock_pin: null,
						current_app_lock_pin: current || null,
					},
				})
				setLockEnabled(false)
				setPinSet(false)
				setEnablingLock(false)
				clearPinFields()
				addAlert(i18n.t('settings.appLockDisabled'), 'success')
			}
		} catch (e) {
			setSecurityMsg(String(e))
		}
	}

	const downloadBackup = async () => {
		setBackupBusy(true)
		try {
			const { blob, filename } = await API.settings.createBackup(backupPassword())
			const url = URL.createObjectURL(blob)
			const a = document.createElement('a')
			a.href = url
			a.download = filename
			a.rel = 'noopener'
			document.body.appendChild(a)
			a.click()
			a.remove()
			URL.revokeObjectURL(url)
			setBackupPassword('')
			addAlert(i18n.t('settings.backupDownloaded'), 'success')
		} catch {
			// apiRequest already surfaced the server's message.
		} finally {
			setBackupBusy(false)
		}
	}

	/**
	 * @param {Event} e
	 */
	const pickRestoreFile = (e) => {
		const input = /** @type {HTMLInputElement} */ (e.currentTarget)
		setRestoreFile(input.files?.[0] || null)
	}

	const runRestore = async () => {
		const file = restoreFile()
		setRestoreConfirmOpen(false)
		if (!file) {
			addAlert(i18n.t('settings.restorePickFile'), 'error')
			return
		}
		setRestoreBusy(true)
		try {
			const result = await API.settings.restoreBackup(file, restorePassword())
			addAlert(
				i18n.t('settings.restoreDone', {
					rows: result.rows,
					tables: result.tables,
				}),
				'success',
			)
			setRestoreFile(null)
			setRestorePassword('')
			if (restoreInput) restoreInput.value = ''
			// The restored database carries its own accounts, so this session's
			// token points at a user that may no longer exist. Sending the user
			// back to login beats letting every later request 401 at them.
			closeSettings()
			navigate(clearSession(setStore))
		} catch (e) {
			addAlert(String(e?.message || e), 'error')
		} finally {
			setRestoreBusy(false)
		}
	}

	const saveTrashSettings = async () => {
		const days = Number(trashRetentionDays())
		if (!Number.isFinite(days) || days < 1 || days > 30) {
			addAlert(i18n.t('settings.retentionRangeError'), 'error')
			return
		}
		setTrashSettingsSaving(true)
		try {
			const s = await API.settings.setTrashSettings(days)
			setTrashRetentionDays(s.retention_days)
			addAlert(i18n.t('settings.trashSettingsSaved'), 'success')
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
								<h2 id="settings-modal-title">{i18n.t('settings.modalTitle')}</h2>
								<p class="settings-modal__sub">
									{isNative()
										? i18n.t('settings.subtitleNative')
										: i18n.t('settings.subtitleWeb')}
								</p>
							</div>
							<IconButton
								aria-label={i18n.t('settings.closeSettings')}
								onClick={closeSettings}
								class="sarca-header-icon"
								size="small"
							>
								<FluentIcon name="dismiss" size={20} />
							</IconButton>
						</div>

						<div class="settings-modal__layout">
							<nav class="settings-nav" aria-label={i18n.t('settings.sectionsAriaLabel')}>
								<p class="settings-nav__label">{i18n.t('settings.menu')}</p>
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
										<span class="settings-nav__title">{i18n.t('settings.generalTab')}</span>
										<span class="settings-nav__desc">{i18n.t('settings.generalTabDesc')}</span>
									</span>
								</button>
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
											<span class="settings-nav__title">{i18n.t('settings.syncTab')}</span>
											<span class="settings-nav__desc">
												{i18n.t('settings.syncTabDesc')}
											</span>
										</span>
									</button>
								</Show>
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
										<span class="settings-nav__title">{i18n.t('settings.accessTab')}</span>
										<span class="settings-nav__desc">{i18n.t('settings.accessTabDesc')}</span>
									</span>
								</button>
								<Show when={isSuperuser()}>
									<button
										type="button"
										class="settings-nav__item"
										classList={{ 'settings-nav__item--active': tab() === 'backup' }}
										onClick={() => setTab('backup')}
									>
										<span class="settings-nav__icon" aria-hidden="true">
											<FluentIcon
												name={tab() === 'backup' ? 'historyFilled' : 'history'}
												size={20}
											/>
										</span>
										<span class="settings-nav__text">
											<span class="settings-nav__title">{i18n.t('settings.backupTab')}</span>
											<span class="settings-nav__desc">{i18n.t('settings.backupTabDesc')}</span>
										</span>
									</button>
								</Show>
							</nav>

							<div class="settings-modal__body">
								<Show when={tab() === 'access'}>
									<p class="settings-bot-hint">
										{i18n.t('settings.botHintPrefix')}{' '}
										<strong>{i18n.t('settings.storageSettingsLabel')}</strong>{' '}
										{i18n.t('settings.botHintSuffix')}
									</p>

									<div class="settings-access">
										<div class="settings-access__toolbar">
											<label class="settings-select-field">
												<span class="settings-select-field__label">{i18n.t('settings.storageLabel')}</span>
												<select
													class="settings-select"
													value={accessStorageId()}
													onChange={(e) =>
														setAccessStorageId(e.currentTarget.value)
													}
												>
													<Show when={!storages().length}>
														<option value="" disabled>
															{i18n.t('settings.noStorages')}
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
													{i18n.t('settings.grantAccess')}
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
													{i18n.t('settings.selectStorageHint')}
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
														{i18n.t('settings.noAccessPermission')}
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
										existingEmails={accessUsers().map((u) => u.email)}
									/>

									<div class="settings-users">
										<Typography variant="h6" sx={{ mt: 4, mb: 1 }}>
											{i18n.t('settings.accounts')}
										</Typography>
										<Show
											when={isSuperuser()}
											fallback={
												<>
													<Typography
														variant="body2"
														color="text.secondary"
														sx={{ mb: 2 }}
													>
														{i18n.t('settings.changeOwnPasswordHint')}
													</Typography>
													<Box
														component="form"
														onSubmit={changeOwnPassword}
														sx={{
															display: 'flex',
															flexDirection: 'column',
															gap: 1.5,
														}}
													>
														<TextField
															label={i18n.t('settings.currentPasswordLabel')}
															type="password"
															required
															autoComplete="current-password"
															value={ownCurrentPassword()}
															onChange={(e) =>
																setOwnCurrentPassword(e.target.value)
															}
														/>
														<TextField
															label={i18n.t('settings.newPasswordLabel')}
															type="password"
															required
															autoComplete="new-password"
															value={ownNewPassword()}
															onChange={(e) => setOwnNewPassword(e.target.value)}
														/>
														<TextField
															label={i18n.t('settings.confirmNewPasswordLabel')}
															type="password"
															required
															autoComplete="new-password"
															value={ownConfirmPassword()}
															onChange={(e) =>
																setOwnConfirmPassword(e.target.value)
															}
														/>
														<Button
															type="submit"
															variant="contained"
															color="secondary"
															disabled={ownPasswordBusy()}
														>
															{i18n.t('settings.changeMyPassword')}
														</Button>
													</Box>
												</>
											}
										>
											<Typography
												variant="body2"
												color="text.secondary"
												sx={{ mb: 2 }}
											>
												{i18n.t('settings.superuserCreateHint')}
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
													label={i18n.t('settings.emailLabel')}
													type="email"
													required
													value={newUserEmail()}
													onChange={(e) => setNewUserEmail(e.target.value)}
												/>
												<TextField
													label={i18n.t('settings.passwordLabel')}
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
													{i18n.t('settings.createUser')}
												</Button>
											</Box>
											<div class="settings-users__list">
												<For
													each={adminUsers()}
													fallback={
														<Typography color="text.secondary">
															{i18n.t('settings.noUsersYet')}
														</Typography>
													}
												>
													{(u) => (
														<div
															class="settings-users__row"
															classList={{
																'settings-users__row--disabled': u.disabled,
															}}
														>
															<div class="settings-users__row-main">
																<div>
																	<strong>{u.email}</strong>
																	<Show when={u.is_superuser}>
																		<span class="settings-users__badge">
																			{i18n.t('settings.superuserBadge')}
																		</span>
																	</Show>
																	<Show when={u.disabled}>
																		<span class="settings-users__badge">
																			{i18n.t('settings.disabledBadge')}
																		</span>
																	</Show>
																</div>
																<div class="settings-users__row-controls">
																	<span class="settings-users__meta">
																		{u.email_verified
																			? i18n.t('settings.verified')
																			: i18n.t('settings.unverified')}
																	</span>
																	<Button
																		size="small"
																		onClick={() =>
																			setOpenPasswordRowId(
																				openPasswordRowId() === u.id ? '' : u.id,
																			)
																		}
																	>
																		{i18n.t('settings.changePassword')}
																	</Button>
																	<SettingsSwitch
																		id={`settings-users-disabled-${u.id}`}
																		ariaLabel={i18n.t('settings.enabledUserAriaLabel', {
																			email: u.email,
																		})}
																		checked={!u.disabled}
																		disabled={store.user?.email === u.email}
																		onChange={() => toggleUserDisabled(u)}
																	/>
																</div>
															</div>
															<Show when={openPasswordRowId() === u.id}>
																<form
																	class="settings-users__password-form"
																	onSubmit={(e) => submitRowPassword(e, u.id)}
																>
																	<TextField
																		size="small"
																		label={i18n.t('settings.newPasswordLabel')}
																		type="password"
																		required
																		autoComplete="new-password"
																		value={rowNewPassword()}
																		onChange={(e) =>
																			setRowNewPassword(e.target.value)
																		}
																	/>
																	<Button
																		type="submit"
																		size="small"
																		variant="contained"
																		color="secondary"
																		disabled={rowPasswordBusy()}
																	>
																		{i18n.t('common.save')}
																	</Button>
																</form>
															</Show>
														</div>
													)}
												</For>
											</div>
										</Show>
									</div>
								</Show>

								<Show when={tab() === 'general'}>
									<div class="settings-account">
										<div class="settings-account__row">
											<div>
												<p class="settings-account__label">{i18n.t('settings.account')}</p>
												<p class="settings-account__hint">
													{store.user?.email ||
														sessionInfo().email ||
														i18n.t('settings.signedIn')}
												</p>
											</div>
										</div>
										<Show when={isNative() && sessionInfo().base_url}>
											<div class="settings-account__row">
												<div>
													<p class="settings-account__label">{i18n.t('settings.server')}</p>
													<p class="settings-account__hint">
														{sessionInfo().base_url}
													</p>
												</div>
											</div>
										</Show>
										<Show when={chrome.storageId()}>
											<div class="settings-account__row">
												<div>
													<p class="settings-account__label">{i18n.t('settings.occupiedSpace')}</p>
													<p class="settings-account__hint">
														{i18n.t('settings.gbUsed', { gb: occupiedGb() })}
													</p>
												</div>
											</div>
										</Show>
										<div class="settings-account__row settings-account__row--theme">
											<div>
												<p class="settings-account__label">{i18n.t('settings.theme')}</p>
												<p class="settings-account__hint">
													{themeHints[mode()] ?? themeHints.light}
												</p>
											</div>
											<div
												class="theme-picker"
												role="radiogroup"
												aria-label={i18n.t('settings.theme')}
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
										{/* `t` is shadowed by the theme loop below, so this
										    section goes through `i18n.t` explicitly. */}
										<div class="settings-account__row settings-account__row--theme">
											<div>
												<p class="settings-account__label">
													{i18n.t('settings.language')}
												</p>
											</div>
											<div
												class="theme-picker"
												role="radiogroup"
												aria-label={i18n.t('settings.language')}
											>
												<For each={LOCALES}>
													{(entry) => (
														<button
															type="button"
															role="radio"
															lang={entry.code}
															aria-checked={i18n.locale() === entry.code}
															class="theme-picker__option"
															classList={{
																'theme-picker__option--active':
																	i18n.locale() === entry.code,
															}}
															onClick={() => i18n.setLocale(entry.code)}
														>
															{entry.label}
														</button>
													)}
												</For>
											</div>
										</div>
										<Show when={isSuperuser()}>
											<div class="settings-trash">
												<p class="settings-account__label">{i18n.t('settings.trashRetention')}</p>
												<Typography
													variant="body2"
													color="text.secondary"
													sx={{ mb: 2 }}
												>
													{i18n.t('settings.trashRetentionHint')}
												</Typography>
												<TextField
													type="number"
													label={i18n.t('settings.daysInTrash')}
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
														{i18n.t('common.save')}
													</Button>
												</div>
											</div>
										</Show>
										<p class="settings-bot-hint">
											{i18n.t('settings.appLockHint')}
										</p>
										<Show
											when={isNative()}
											fallback={
												<p class="settings-account__hint">
													{i18n.t('settings.appLockNativeOnly')}
												</p>
											}
										>
											<AppLockToggle
												checked={lockEnabled() || enablingLock()}
												onChange={(on) => {
													if (on) {
														setEnablingLock(true)
														setSecurityMsg(i18n.t('settings.enterNewPinBelow'))
													} else if (lockEnabled()) {
														saveAppLock(false)
													} else {
														// Was never actually saved — just close the
														// "entering PIN" UI, nothing to disable natively.
														setEnablingLock(false)
														setSecurityMsg('')
														clearPinFields()
													}
												}}
											/>
											<Show when={pinSet()}>
												<TextField
													label={i18n.t('settings.currentPinLabel')}
													type="password"
													size="small"
													fullWidth
													sx={{ mt: 1 }}
													value={pinCurrent()}
													onChange={(_, v) => setPinCurrent(v)}
													inputProps={{ inputMode: 'numeric', maxLength: 8 }}
												/>
											</Show>
											<TextField
												label={
													pinSet()
														? i18n.t('settings.newPinLabel')
														: i18n.t('settings.pinLabel')
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
												label={i18n.t('settings.confirmNewPinLabel')}
												type="password"
												size="small"
												fullWidth
												sx={{ mt: 1 }}
												value={pinConfirm()}
												onChange={(_, v) => setPinConfirm(v)}
												inputProps={{ inputMode: 'numeric', maxLength: 8 }}
											/>
											<div
												class="settings-sync-panel__row"
												style={{ 'margin-top': '8px' }}
											>
												<Button
													variant="contained"
													color="secondary"
													onClick={() => saveAppLock(true)}
												>
													{lockEnabled()
														? i18n.t('settings.savePin')
														: i18n.t('settings.enableLock')}
												</Button>
												<Show when={lockEnabled()}>
													<Button
														variant="outlined"
														color="error"
														onClick={() => saveAppLock(false)}
													>
														{i18n.t('settings.disable')}
													</Button>
												</Show>
											</div>
											<Show when={securityMsg()}>
												<p class="settings-bot-hint" role="status">
													{securityMsg()}
												</p>
											</Show>
										</Show>
										<Show when={isNative()}>
											<div class="settings-account__row">
												<div>
													<p class="settings-account__label">{i18n.t('settings.cache')}</p>
													<p class="settings-account__hint">
														{formatBytes(cacheBytes())} /{' '}
														{formatBytes(cacheLimitBytes())}
													</p>
												</div>
												<Button variant="outlined" onClick={clearCache}>
													{i18n.t('settings.clearCache')}
												</Button>
											</div>
											<div class="settings-account__row">
												<div>
													<p class="settings-account__label">{i18n.t('settings.about')}</p>
													<p class="settings-account__hint">
														{i18n.t('settings.clientVersion', {
															version: about().version || '—',
															platform: about().platform || i18n.t('settings.nativePlatform'),
														})}
													</p>
												</div>
											</div>
											<div class="settings-toggle">
												<span>{i18n.t('settings.enableLogs')}</span>
												<SettingsSwitch
													id="settings-enable-logs-switch"
													checked={logsEnabled()}
													disabled={logsBusy()}
													onChange={(checked) => setEnableLogs(checked)}
												/>
											</div>
											<div class="settings-account__row">
												<div>
													<p class="settings-account__label">{i18n.t('settings.exportLogs')}</p>
													<p class="settings-account__hint">
														{i18n.t('settings.exportLogsHint')}
													</p>
												</div>
												<Button
													variant="outlined"
													disabled={logsBusy()}
													onClick={exportLogs}
												>
													{i18n.t('settings.exportLogs')}
												</Button>
											</div>
										</Show>
										<div class="settings-account__row">
											<div>
												<p class="settings-account__label">{i18n.t('settings.session')}</p>
												<p class="settings-account__hint">
													{i18n.t('settings.signOutHint')}
												</p>
											</div>
											<Button
												variant="outlined"
												color="error"
												startIcon={<FluentIcon name="signOut" size={18} />}
												onClick={logout}
											>
												{i18n.t('sidebar.logOut')}
											</Button>
										</div>
									</div>
								</Show>

								<Show when={tab() === 'backup' && isSuperuser()}>
									<div class="settings-account">
										<div class="settings-backup">
											<p class="settings-account__label">
												{i18n.t('settings.backupTitle')}
											</p>
											<Typography
												variant="body2"
												color="text.secondary"
												sx={{ mb: 2 }}
											>
												{i18n.t('settings.backupHint')}
											</Typography>
											<TextField
												type="password"
												label={i18n.t('settings.backupPasswordLabel')}
												fullWidth
												value={backupPassword()}
												onChange={(e) => setBackupPassword(e.target.value)}
											/>
											<p class="settings-account__hint">
												{i18n.t('settings.backupPasswordHint')}
											</p>
											<div style={{ 'margin-top': '16px' }}>
												<Button
													variant="contained"
													color="secondary"
													disabled={backupBusy()}
													startIcon={
														<FluentIcon name="arrowDownload" size={18} />
													}
													onClick={downloadBackup}
												>
													{backupBusy()
														? i18n.t('settings.backupWorking')
														: i18n.t('settings.downloadBackup')}
												</Button>
											</div>

											<div class="settings-backup__restore">
												<p class="settings-account__label">
													{i18n.t('settings.restoreTitle')}
												</p>
												<Typography
													variant="body2"
													color="text.secondary"
													sx={{ mb: 2 }}
												>
													{i18n.t('settings.restoreHint')}
												</Typography>
												<input
													ref={restoreInput}
													type="file"
													accept=".sarcabak"
													class="settings-backup__file"
													aria-label={i18n.t('settings.restorePickFile')}
													onChange={pickRestoreFile}
												/>
												<Show when={restoreFile()}>
													{(file) => (
														<p class="settings-account__hint">
															{file().name} —{' '}
															{formatBytes(file().size)}
														</p>
													)}
												</Show>
												<div style={{ 'margin-top': '12px' }}>
													<TextField
														type="password"
														label={i18n.t('settings.restorePasswordLabel')}
														fullWidth
														value={restorePassword()}
														onChange={(e) =>
															setRestorePassword(e.target.value)
														}
													/>
												</div>
												<div style={{ 'margin-top': '16px' }}>
													<Button
														variant="outlined"
														color="error"
														disabled={restoreBusy() || !restoreFile()}
														startIcon={
															<FluentIcon name="arrowUndo" size={18} />
														}
														onClick={() => setRestoreConfirmOpen(true)}
													>
														{restoreBusy()
															? i18n.t('settings.restoreWorking')
															: i18n.t('settings.restoreAction')}
													</Button>
												</div>
											</div>
										</div>
									</div>
								</Show>

								<Show when={tab() === 'sync' && showSyncTab()}>
									<SettingsSyncPanel
										storageId={chrome.storageId()}
										storageName={chrome.storageName()}
									/>
								</Show>
							</div>
						</div>
					</div>
				</div>

				<ActionConfirmDialog
					isOpened={restoreConfirmOpen()}
					entity={i18n.t('confirmDialog.restoreEntity')}
					action={i18n.t('confirmDialog.restoreAction')}
					actionDescription={i18n.t('confirmDialog.restoreDescription')}
					onConfirm={runRestore}
					onCancel={() => setRestoreConfirmOpen(false)}
				/>
			</Show>
		</>
	)
}

export default SettingsModal
