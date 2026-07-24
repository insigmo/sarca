import { For, Show, createEffect, createSignal, onCleanup } from 'solid-js'
import IconButton from '@suid/material/IconButton'
import Button from '@suid/material/Button'
import TextField from '@suid/material/TextField'
import Select from '@suid/material/Select'
import InputLabel from '@suid/material/InputLabel'
import FormControl from '@suid/material/FormControl'
import MenuItem from '@suid/material/MenuItem'
import Typography from '@suid/material/Typography'
import CloseIcon from '@suid/icons-material/Close'
import DeleteIcon from '@suid/icons-material/Delete'
import AddIcon from '@suid/icons-material/Add'
import VisibilityIcon from '@suid/icons-material/Visibility'
import VisibilityOffIcon from '@suid/icons-material/VisibilityOff'
import HelpOutlineIcon from '@suid/icons-material/HelpOutline'
import ChevronLeftIcon from '@suid/icons-material/ChevronLeft'
import SmartToyOutlinedIcon from '@suid/icons-material/SmartToyOutlined'
import LockOutlinedIcon from '@suid/icons-material/LockOutlined'

import API from '../api'
import { settingsStore } from '../common/settings'
import { filesChromeStore } from '../common/filesChrome'
import { alertStore } from './AlertStack'
import ActionConfirmDialog from './ActionConfirmDialog'
import Access from './Access'
import GrantAccess from './GrantAccess'
import WaveDivider from './WaveDivider'

const maskToken = (token) => {
	if (!token) return ''
	if (token.length <= 10) return '••••••••'
	return `${token.slice(0, 4)}${'•'.repeat(Math.min(18, token.length - 8))}${token.slice(-4)}`
}

const SettingsModal = () => {
	const { isOpen, closeSettings, tab, setTab } = settingsStore
	const chrome = filesChromeStore
	const { addAlert } = alertStore

	/** @type {[import("solid-js").Accessor<import("../api").StorageWorker[]>, any]} */
	const [workers, setWorkers] = createSignal([])
	/** @type {[import("solid-js").Accessor<import("../api").StorageWithInfo[]>, any]} */
	const [storages, setStorages] = createSignal([])
	const [storageNameById, setStorageNameById] = createSignal({})
	const [view, setView] = createSignal('list')
	const [visibleTokens, setVisibleTokens] = createSignal({})
	const [pendingDelete, setPendingDelete] = createSignal(null)
	const [loading, setLoading] = createSignal(false)
	const [selectedStorageId, setSelectedStorageId] = createSignal('')
	/** Access tab */
	const [accessStorageId, setAccessStorageId] = createSignal('')
	const [accessUsers, setAccessUsers] = createSignal([])
	const [canManageAccess, setCanManageAccess] = createSignal(false)
	const [isGrantVisible, setIsGrantVisible] = createSignal(false)

	const refreshWorkers = async () => {
		setLoading(true)
		try {
			const [workersList, storagesSchema] = await Promise.all([
				API.storageWorkers.listStorageWorkers(),
				API.storages.listStorages(),
			])
			setWorkers(workersList)
			setStorages(storagesSchema.storages)
			const map = {}
			for (const s of storagesSchema.storages) {
				map[s.id] = s.name
			}
			setStorageNameById(map)

			const preferred = chrome.storageId() || storagesSchema.storages[0]?.id || ''
			if (!accessStorageId() && preferred) {
				setAccessStorageId(preferred)
			}
		} finally {
			setLoading(false)
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

		setView('list')
		refreshWorkers()
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
		const preferred = chrome.storageId()
		if (preferred && preferred !== accessStorageId()) {
			setAccessStorageId(preferred)
		}
	})

	createEffect(() => {
		if (!isOpen() || tab() !== 'access') return
		accessStorageId()
		fetchAccessUsers()
	})

	const toggleToken = (id) => {
		setVisibleTokens((prev) => ({ ...prev, [id]: !prev[id] }))
	}

	const confirmDelete = async () => {
		const sw = pendingDelete()
		setPendingDelete(null)
		if (!sw) return
		await API.storageWorkers.deleteStorageWorker(sw.id)
		addAlert(`Deleted storage worker "${sw.name}"`, 'success')
		await refreshWorkers()
	}

	/**
	 * @param {SubmitEvent} event
	 */
	const handleCreate = async (event) => {
		event.preventDefault()
		const data = new FormData(event.currentTarget)
		const name = data.get('name')
		const token = data.get('token')
		const storageId = selectedStorageId()

		if (!storageId) {
			addAlert('Please select a storage', 'error')
			return
		}

		await API.storageWorkers.createStorageWorker(name, token, storageId)
		addAlert(`Created storage worker "${name}"`, 'success')
		setSelectedStorageId('')
		setView('list')
		await refreshWorkers()
	}

	const storageLabel = (id) => storageNameById()[id] || id

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
									Workers and storage access
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

						<div class="settings-tabs">
							<button
								type="button"
								class="settings-tab"
								classList={{ 'settings-tab--active': tab() === 'workers' }}
								onClick={() => {
									setTab('workers')
									setView('list')
								}}
							>
								<SmartToyOutlinedIcon fontSize="small" />
								Workers
							</button>
							<button
								type="button"
								class="settings-tab"
								classList={{ 'settings-tab--active': tab() === 'access' }}
								onClick={() => setTab('access')}
							>
								<LockOutlinedIcon fontSize="small" />
								Access
							</button>
						</div>

						<WaveDivider class="settings-modal__wave" />

						<div class="settings-modal__body">
							<Show when={tab() === 'workers'}>
								<Show when={view() === 'list'}>
									<div
										style={{
											display: 'flex',
											'justify-content': 'flex-end',
											'margin-bottom': '14px',
										}}
									>
										<Button
											variant="contained"
											color="secondary"
											startIcon={<AddIcon />}
											onClick={() => setView('create')}
										>
											New worker
										</Button>
									</div>

									<div class="worker-row worker-row--head">
										<div class="worker-row__label">Name</div>
										<div class="worker-row__label">Storage</div>
										<div class="worker-row__label">Token</div>
										<div
											class="worker-row__label"
											style={{ 'text-align': 'right' }}
										>
											Actions
										</div>
									</div>

									<Show
										when={!loading() && workers().length}
										fallback={
											<Typography
												color="text.secondary"
												sx={{ py: 4, textAlign: 'center', px: 2 }}
											>
												{loading()
													? 'Loading…'
													: 'No storage workers yet — register a bot token (New worker), or set TELEGRAM_BOT_TOKEN, TELEGRAM_CHANNEL_ID, and STORAGE_NAME in sarca.conf for auto-setup.'}
											</Typography>
										}
									>
										<For each={workers()}>
											{(sw) => (
												<div class="worker-row">
													<div>
														<div class="worker-row__label">Name</div>
														<div class="worker-row__value">{sw.name}</div>
													</div>
													<div>
														<div class="worker-row__label">Storage</div>
														<div class="worker-row__value">
															{storageLabel(sw.storage_id)}
														</div>
													</div>
													<div>
														<div class="worker-row__label">Token</div>
														<div class="worker-row__token worker-row__value">
															<span
																style={{
																	flex: 1,
																	'min-width': 0,
																	overflow: 'hidden',
																	'text-overflow': 'ellipsis',
																}}
															>
																{visibleTokens()[sw.id]
																	? sw.token
																	: maskToken(sw.token)}
															</span>
															<IconButton
																size="small"
																aria-label={
																	visibleTokens()[sw.id]
																		? 'Hide token'
																		: 'Show token'
																}
																onClick={() => toggleToken(sw.id)}
															>
																<Show
																	when={visibleTokens()[sw.id]}
																	fallback={
																		<VisibilityIcon fontSize="small" />
																	}
																>
																	<VisibilityOffIcon fontSize="small" />
																</Show>
															</IconButton>
														</div>
													</div>
													<div style={{ 'text-align': 'right' }}>
														<IconButton
															aria-label="Delete worker"
															onClick={() => setPendingDelete(sw)}
															sx={{ color: 'error.main' }}
														>
															<DeleteIcon />
														</IconButton>
													</div>
												</div>
											)}
										</For>
									</Show>
								</Show>

								<Show when={view() === 'create'}>
									<form class="worker-form" onSubmit={handleCreate}>
										<div
											style={{
												display: 'flex',
												'align-items': 'center',
												gap: '4px',
											}}
										>
											<Button
												type="button"
												variant="outlined"
												size="small"
												startIcon={<ChevronLeftIcon />}
												onClick={() => setView('list')}
											>
												Back
											</Button>
											<a
												href="https://github.com/insigmo/sarca#usage"
												target="_blank"
												rel="noreferrer"
											>
												<IconButton
													color="warning"
													size="small"
													aria-label="Help"
												>
													<HelpOutlineIcon />
												</IconButton>
											</a>
										</div>

										<TextField
											id="worker-name"
											name="name"
											label="Name"
											fullWidth
											required
										/>
										<TextField
											id="worker-token"
											name="token"
											label="Token"
											fullWidth
											required
											autoComplete="off"
										/>
										<FormControl fullWidth required>
											<InputLabel id="worker-storage-label">Storage</InputLabel>
											<Select
												labelId="worker-storage-label"
												label="Storage"
												name="storage_id"
												value={selectedStorageId()}
												onChange={(e) => setSelectedStorageId(e.target.value)}
											>
												<For each={storages()}>
													{(storage) => (
														<MenuItem value={storage.id}>
															{storage.name}
														</MenuItem>
													)}
												</For>
											</Select>
										</FormControl>

										<div class="worker-form-actions">
											<Button type="submit" variant="contained" color="secondary">
												Register
											</Button>
											<Button
												type="button"
												variant="outlined"
												onClick={() => setView('list')}
											>
												Cancel
											</Button>
										</div>
									</form>
								</Show>
							</Show>

							<Show when={tab() === 'access'}>
								<div class="settings-access">
									<div class="settings-access__toolbar">
										<FormControl size="small" sx={{ minWidth: 200, flex: 1 }}>
											<InputLabel id="access-storage-label">Storage</InputLabel>
											<Select
												labelId="access-storage-label"
												label="Storage"
												value={accessStorageId()}
												onChange={(e) => setAccessStorageId(e.target.value)}
											>
												<For each={storages()}>
													{(storage) => (
														<MenuItem value={storage.id}>
															{storage.name}
														</MenuItem>
													)}
												</For>
											</Select>
										</FormControl>
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
						</div>
					</div>
				</div>
			</Show>

			<ActionConfirmDialog
				action="Delete"
				entity="storage worker"
				actionDescription={`delete storage worker ${pendingDelete()?.name || ''}`}
				isOpened={Boolean(pendingDelete())}
				onConfirm={confirmDelete}
				onCancel={() => setPendingDelete(null)}
			/>
		</>
	)
}

export default SettingsModal
