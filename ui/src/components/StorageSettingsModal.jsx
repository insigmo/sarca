import { For, Show, createEffect, createSignal, onCleanup } from 'solid-js'
import IconButton from '@suid/material/IconButton'
import Button from '@suid/material/Button'
import TextField from '@suid/material/TextField'
import Typography from '@suid/material/Typography'
import CloseIcon from '@suid/icons-material/Close'
import DeleteIcon from '@suid/icons-material/Delete'
import EditIcon from '@suid/icons-material/Edit'
import AddIcon from '@suid/icons-material/Add'
import RefreshIcon from '@suid/icons-material/Refresh'
import WarningAmberIcon from '@suid/icons-material/WarningAmber'

import API from '../api'
import { alertStore } from './AlertStack'
import ActionConfirmDialog from './ActionConfirmDialog'
import WaveDivider from './WaveDivider'

const MAX_CHANNELS = 3

/**
 * @typedef {Object} StorageSettingsModalProps
 * @property {import('../api').StorageWithInfo | null} storage
 * @property {() => void} onClose
 * @property {(storage: import('../api').StorageWithInfo) => void} onRenamed
 * @property {(storageId: string) => void} onDeleted
 */

/**
 * @param {string} value
 * @returns {string | null}
 */
const validateChatId = (value) => {
	if (value === '' || value === null || value === undefined) {
		return 'Chat id is required'
	}
	const n = Number(value)
	if (!Number.isFinite(n) || n >= 0) {
		return 'Chat id must be a negative integer'
	}
	return null
}

/**
 * @param {StorageSettingsModalProps} props
 */
const StorageSettingsModal = (props) => {
	const { addAlert } = alertStore
	const [name, setName] = createSignal('')
	const [saving, setSaving] = createSignal(false)
	const [confirmDelete, setConfirmDelete] = createSignal(false)

	// Channels + replication (multi-chat storage)
	const [channels, setChannels] = createSignal([])
	const [replication, setReplication] = createSignal(null)
	const [loadingDetail, setLoadingDetail] = createSignal(false)
	const [detailError, setDetailError] = createSignal(null)

	// Add/edit one channel at a time. `editingId` is either a channel id or
	// the string 'new' for the add-channel form.
	const [editingId, setEditingId] = createSignal(null)
	const [draftChatId, setDraftChatId] = createSignal('')
	const [draftName, setDraftName] = createSignal('')
	const [draftError, setDraftError] = createSignal(null)
	const [savingChannel, setSavingChannel] = createSignal(false)
	const [pendingRemoveChannel, setPendingRemoveChannel] = createSignal(null)
	const [retrying, setRetrying] = createSignal(false)

	const refreshDetail = async () => {
		const storage = props.storage
		if (!storage) return

		setLoadingDetail(true)
		setDetailError(null)
		try {
			const detail = await API.storages.getStorageDetail(storage.id)
			setChannels(detail.channels || [])
			setReplication(detail.replication || null)
		} catch (err) {
			console.error(err)
			setDetailError('Could not load channels. Try reopening settings.')
			setChannels([])
			setReplication(null)
		} finally {
			setLoadingDetail(false)
		}
	}

	createEffect(() => {
		const storage = props.storage
		if (!storage) {
			setConfirmDelete(false)
			return
		}

		setName(storage.name)
		setConfirmDelete(false)
		setEditingId(null)
		setPendingRemoveChannel(null)
		document.body.style.overflow = 'hidden'

		refreshDetail()

		const onKeyDown = (e) => {
			if (e.key === 'Escape') {
				if (confirmDelete()) setConfirmDelete(false)
				else props.onClose()
			}
		}
		window.addEventListener('keydown', onKeyDown)

		onCleanup(() => {
			document.body.style.overflow = ''
			window.removeEventListener('keydown', onKeyDown)
		})
	})

	const saveName = async (e) => {
		e.preventDefault()
		const storage = props.storage
		if (!storage) return

		const next = name().trim()
		if (!next) {
			addAlert('Storage name is required', 'error')
			return
		}
		if (next === storage.name) {
			props.onClose()
			return
		}

		setSaving(true)
		try {
			const updated = await API.storages.renameStorage(storage.id, next)
			addAlert(`Renamed storage to "${updated.name}"`, 'success')
			props.onRenamed({ ...storage, name: updated.name })
			props.onClose()
		} catch (err) {
			console.error(err)
		} finally {
			setSaving(false)
		}
	}

	const deleteStorage = async () => {
		const storage = props.storage
		if (!storage) return

		setSaving(true)
		try {
			await API.storages.deleteStorage(storage.id)
			addAlert(`Deleted storage "${storage.name}" and all its files`, 'success')
			setConfirmDelete(false)
			props.onDeleted(storage.id)
			props.onClose()
		} catch (err) {
			console.error(err)
		} finally {
			setSaving(false)
		}
	}

	const startEditChannel = (channel) => {
		setEditingId(channel.id)
		setDraftChatId(String(channel.chat_id))
		setDraftName(channel.name || '')
		setDraftError(null)
	}

	const startAddChannel = () => {
		if (channels().length >= MAX_CHANNELS) return
		setEditingId('new')
		setDraftChatId('')
		setDraftName('')
		setDraftError(null)
	}

	const cancelEditChannel = () => {
		setEditingId(null)
		setDraftError(null)
	}

	const saveChannel = async () => {
		const storage = props.storage
		if (!storage) return

		const error = validateChatId(draftChatId())
		if (error) {
			setDraftError(error)
			return
		}

		const chatId = parseInt(draftChatId(), 10)
		const trimmedName = draftName().trim()

		setSavingChannel(true)
		try {
			if (editingId() === 'new') {
				await API.storages.addChannel(
					storage.id,
					chatId,
					trimmedName || undefined,
				)
				addAlert('Channel added', 'success')
			} else {
				await API.storages.updateChannel(storage.id, editingId(), {
					chat_id: chatId,
					name: trimmedName || undefined,
				})
				addAlert('Channel updated', 'success')
			}
			setEditingId(null)
			await refreshDetail()
		} catch (err) {
			console.error(err)
		} finally {
			setSavingChannel(false)
		}
	}

	const requestRemoveChannel = (channel) => {
		const activeCount = channels().filter((c) => c.status === 'active').length
		if (channel.status === 'active' && activeCount <= 1) {
			addAlert('Cannot remove the last active channel', 'error')
			return
		}
		setPendingRemoveChannel(channel)
	}

	const confirmRemoveChannel = async () => {
		const storage = props.storage
		const channel = pendingRemoveChannel()
		setPendingRemoveChannel(null)
		if (!storage || !channel) return

		try {
			await API.storages.removeChannel(storage.id, channel.id)
			addAlert('Channel removed', 'success')
			await refreshDetail()
		} catch (err) {
			console.error(err)
		}
	}

	const retryReplication = async () => {
		const storage = props.storage
		if (!storage) return

		setRetrying(true)
		try {
			await API.storages.retryReplication(storage.id)
			addAlert('Retrying failed uploads', 'success')
			await refreshDetail()
		} catch (err) {
			console.error(err)
		} finally {
			setRetrying(false)
		}
	}

	return (
		<>
			<Show when={props.storage}>
				<div
					class="settings-overlay"
					onClick={(e) => {
						if (e.target === e.currentTarget) props.onClose()
					}}
					role="presentation"
				>
					<div
						class="settings-modal settings-modal--storage"
						role="dialog"
						aria-modal="true"
						aria-labelledby="storage-settings-title"
						onClick={(e) => e.stopPropagation()}
					>
						<div class="settings-modal__header">
							<div>
								<h2 id="storage-settings-title">Storage settings</h2>
								<p class="settings-modal__sub">
									Rename, manage channels, or permanently delete this storage
								</p>
							</div>
							<IconButton
								aria-label="Close storage settings"
								onClick={props.onClose}
								class="sarca-header-icon"
								size="small"
							>
								<CloseIcon />
							</IconButton>
						</div>

						<WaveDivider class="settings-modal__wave" />

						<div class="settings-modal__body">
							<form class="storage-settings-form" onSubmit={saveName}>
								<TextField
									label="Name"
									name="name"
									value={name()}
									onChange={(_, v) => setName(v)}
									fullWidth
									required
									autoFocus
									disabled={saving()}
								/>
								<div class="storage-settings-form__actions">
									<Button
										type="submit"
										variant="contained"
										color="secondary"
										disabled={saving() || !name().trim()}
									>
										Save
									</Button>
								</div>
							</form>

							<div class="channels-section">
								<h3>Channels</h3>

								<Show when={detailError()}>
									<p class="channel-row__dead-message">{detailError()}</p>
								</Show>

								<Show when={loadingDetail() && !channels().length}>
									<Typography color="text.secondary" sx={{ fontSize: '0.85rem' }}>
										Loading channels…
									</Typography>
								</Show>

								<div class="channels-list">
									<For each={channels()}>
										{(channel) => (
											<div
												class="channel-row"
												classList={{ 'channel-row--dead': channel.status === 'dead' }}
											>
												<Show
													when={editingId() === channel.id}
													fallback={
														<>
															<div class="channel-row__top">
																<div class="channel-row__info">
																	<span class="channel-row__name">
																		{channel.name || `Channel ${channel.position}`}
																	</span>
																	<span class="channel-row__chatid">
																		id {channel.chat_id}
																	</span>
																	<span
																		class={`channel-status channel-status--${channel.status}`}
																	>
																		{channel.status === 'active' ? 'Active' : 'Deleted'}
																	</span>
																</div>
																<div class="channel-row__actions">
																	<IconButton
																		size="small"
																		aria-label={`Edit channel ${channel.name || channel.position}`}
																		onClick={() => startEditChannel(channel)}
																	>
																		<EditIcon fontSize="small" />
																	</IconButton>
																	<IconButton
																		size="small"
																		aria-label={`Remove channel ${channel.name || channel.position}`}
																		onClick={() => requestRemoveChannel(channel)}
																		sx={{ color: 'error.main' }}
																	>
																		<DeleteIcon fontSize="small" />
																	</IconButton>
																</div>
															</div>
															<Show when={channel.status === 'dead'}>
																<p class="channel-row__dead-message">
																	<WarningAmberIcon
																		fontSize="inherit"
																		sx={{ mr: 0.5, verticalAlign: 'text-bottom' }}
																	/>
																	Channel "{channel.name || `#${channel.position}`}"
																	(id {channel.chat_id}) was deleted. Set a new chat id
																	or add another channel.
																</p>
															</Show>
														</>
													}
												>
													<div class="channel-row__edit-form">
														<TextField
															label="Chat id"
															type="number"
															size="small"
															value={draftChatId()}
															onChange={(_, v) => {
																setDraftChatId(v)
																setDraftError(null)
															}}
															error={typeof draftError() === 'string'}
															helperText={draftError() || ''}
															fullWidth
															required
															autoFocus
														/>
														<TextField
															label="Name (optional)"
															size="small"
															value={draftName()}
															onChange={(_, v) => setDraftName(v)}
															fullWidth
														/>
														<div class="channel-row__edit-actions">
															<Button
																size="small"
																variant="contained"
																color="secondary"
																disabled={savingChannel()}
																onClick={saveChannel}
															>
																Save
															</Button>
															<Button
																size="small"
																disabled={savingChannel()}
																onClick={cancelEditChannel}
															>
																Cancel
															</Button>
														</div>
													</div>
												</Show>
											</div>
										)}
									</For>

									<Show when={editingId() === 'new'}>
										<div class="channel-row">
											<div class="channel-row__edit-form">
												<TextField
													label="Chat id"
													type="number"
													size="small"
													value={draftChatId()}
													onChange={(_, v) => {
														setDraftChatId(v)
														setDraftError(null)
													}}
													error={typeof draftError() === 'string'}
													helperText={
														draftError() ||
														'Get chat ID via @userinfobot or @getidsbot.'
													}
													fullWidth
													required
													autoFocus
												/>
												<TextField
													label="Name (optional)"
													size="small"
													value={draftName()}
													onChange={(_, v) => setDraftName(v)}
													fullWidth
												/>
												<div class="channel-row__edit-actions">
													<Button
														size="small"
														variant="contained"
														color="secondary"
														disabled={savingChannel()}
														onClick={saveChannel}
													>
														Save
													</Button>
													<Button
														size="small"
														disabled={savingChannel()}
														onClick={cancelEditChannel}
													>
														Cancel
													</Button>
												</div>
											</div>
										</div>
									</Show>
								</div>

								<Show when={editingId() === null}>
									<Button
										variant="outlined"
										size="small"
										startIcon={<AddIcon />}
										disabled={channels().length >= MAX_CHANNELS}
										onClick={startAddChannel}
									>
										Add another channel
									</Button>
								</Show>
							</div>

							<Show when={replication()}>
								<div class="replication-summary">
									<h3>Replication</h3>
									<div class="replication-summary__stats">
										<span>Uploaded: {replication().uploaded}</span>
										<span>Pending: {replication().pending}</span>
										<span>Failed: {replication().failed}</span>
									</div>
									<Button
										variant="outlined"
										color="warning"
										size="small"
										startIcon={<RefreshIcon />}
										disabled={retrying() || !replication().failed}
										onClick={retryReplication}
									>
										Retry upload
									</Button>
								</div>
							</Show>

							<div class="storage-settings-danger">
								<h3>Danger zone</h3>
								<p>
									Delete this storage together with all files. This cannot be
									undone.
								</p>
								<Button
									variant="outlined"
									color="error"
									startIcon={<DeleteIcon />}
									disabled={saving()}
									onClick={() => setConfirmDelete(true)}
								>
									Delete storage and files
								</Button>
							</div>
						</div>
					</div>
				</div>
			</Show>

			<ActionConfirmDialog
				isOpened={confirmDelete()}
				entity="storage"
				action="Delete"
				actionDescription={`permanently delete storage "${props.storage?.name || ''}" and all its files`}
				onConfirm={deleteStorage}
				onCancel={() => setConfirmDelete(false)}
			/>

			<ActionConfirmDialog
				isOpened={Boolean(pendingRemoveChannel())}
				entity="channel"
				action="Remove"
				actionDescription={`remove channel "${
					pendingRemoveChannel()?.name || `#${pendingRemoveChannel()?.position}`
				}" (id ${pendingRemoveChannel()?.chat_id}) from this storage`}
				onConfirm={confirmRemoveChannel}
				onCancel={() => setPendingRemoveChannel(null)}
			/>
		</>
	)
}

export default StorageSettingsModal
