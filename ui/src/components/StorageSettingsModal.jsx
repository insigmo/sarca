import { For, Show, createEffect, createSignal, onCleanup } from 'solid-js'
import { Portal } from 'solid-js/web'
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
import BadgeOutlinedIcon from '@suid/icons-material/BadgeOutlined'
import HubOutlinedIcon from '@suid/icons-material/HubOutlined'

import API from '../api'
import { alertStore } from './AlertStack'
import ActionConfirmDialog from './ActionConfirmDialog'

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
	const [tab, setTab] = createSignal(/** @type {'general' | 'telegram'} */ ('general'))
	const [name, setName] = createSignal('')
	const [saving, setSaving] = createSignal(false)
	const [confirmDelete, setConfirmDelete] = createSignal(false)

	const [channels, setChannels] = createSignal([])
	const [replication, setReplication] = createSignal(null)
	const [bot, setBot] = createSignal(null)
	const [loadingDetail, setLoadingDetail] = createSignal(false)
	const [detailError, setDetailError] = createSignal(null)

	const [editingId, setEditingId] = createSignal(null)
	const [draftChatId, setDraftChatId] = createSignal('')
	const [draftName, setDraftName] = createSignal('')
	const [draftError, setDraftError] = createSignal(null)
	const [savingChannel, setSavingChannel] = createSignal(false)
	const [pendingRemoveChannel, setPendingRemoveChannel] = createSignal(null)
	const [retrying, setRetrying] = createSignal(false)
	const [refreshingChannels, setRefreshingChannels] = createSignal(false)
	const [editingBot, setEditingBot] = createSignal(false)
	const [botToken, setBotToken] = createSignal('')
	const [savingBot, setSavingBot] = createSignal(false)
	const [botFormError, setBotFormError] = createSignal(null)

	const refreshDetail = async () => {
		const storage = props.storage
		if (!storage) return

		setLoadingDetail(true)
		setDetailError(null)
		try {
			const detail = await API.storages.getStorageDetail(storage.id)
			setChannels(detail.channels || [])
			setReplication(detail.replication || null)
			setBot(detail.bot || null)
		} catch (err) {
			console.error(err)
			setDetailError('Could not load channels. Try reopening settings.')
			setChannels([])
			setReplication(null)
			setBot(null)
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
		setTab('general')
		setConfirmDelete(false)
		setEditingId(null)
		setPendingRemoveChannel(null)
		setEditingBot(false)
		setBotToken('')
		setBotFormError(null)
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
			addAlert('Name unchanged', 'info')
			return
		}

		setSaving(true)
		try {
			const updated = await API.storages.renameStorage(storage.id, next)
			addAlert(`Renamed storage to "${updated.name}"`, 'success')
			props.onRenamed({ ...storage, name: updated.name })
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

	const refreshChannelsFromBot = async () => {
		const storage = props.storage
		if (!storage) return

		setRefreshingChannels(true)
		try {
			const result = await API.storages.refreshChannels(storage.id)
			setChannels(result.channels || [])
			const n = result.added?.length || 0
			if (n > 0) {
				addAlert(n === 1 ? 'Added 1 channel' : `Added ${n} channels`, 'success')
			} else if (result.hint) {
				addAlert(result.hint, 'warning')
			} else if (result.skipped_full) {
				addAlert('Already at 3 channels — remove one to add more', 'info')
			} else if (result.skipped_in_use?.length) {
				addAlert('Found channel(s) already used by another storage', 'warning')
			} else {
				addAlert(
					'No new channels found. Add the bot as admin to a channel, then refresh.',
					'info',
				)
			}
		} catch (err) {
			console.error(err)
			addAlert(
				err?.message ||
					'Could not refresh channels. Is a bot attached to this storage?',
				'error',
			)
		} finally {
			setRefreshingChannels(false)
		}
	}

	const startEditBot = () => {
		setEditingBot(true)
		setBotToken('')
		setBotFormError(null)
	}

	const cancelEditBot = () => {
		setEditingBot(false)
		setBotToken('')
		setBotFormError(null)
	}

	const saveBot = async () => {
		const storage = props.storage
		if (!storage) return

		const token = botToken().trim()
		if (!token || !token.includes(':')) {
			setBotFormError('Paste a valid bot token from @BotFather')
			return
		}

		setSavingBot(true)
		setBotFormError(null)
		try {
			const hadBot = Boolean(bot())
			const next = await API.storages.setStorageBot(storage.id, token)
			setBot(next)
			setEditingBot(false)
			setBotToken('')
			addAlert(
				hadBot ? `Bot updated to "${next.name}"` : `Bot "${next.name}" attached`,
				'success',
			)
			await refreshDetail()
			setBot(next)
		} catch (err) {
			console.error(err)
			setBotFormError(err?.message || 'Could not save bot token')
		} finally {
			setSavingBot(false)
		}
	}

	const channelEditor = () => (
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
					(editingId() === 'new'
						? 'Get chat ID via @userinfobot or @getidsbot.'
						: '')
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
				<Button size="small" disabled={savingChannel()} onClick={cancelEditChannel}>
					Cancel
				</Button>
			</div>
		</div>
	)

	return (
		<>
			<Show when={props.storage}>
				<Portal mount={document.body}>
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
										{props.storage?.name || 'Storage'}
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

							<div class="settings-modal__layout">
								<nav class="settings-nav" aria-label="Storage settings sections">
									<p class="settings-nav__label">Menu</p>
									<button
										type="button"
										class="settings-nav__item"
										classList={{ 'settings-nav__item--active': tab() === 'general' }}
										onClick={() => setTab('general')}
									>
										<span class="settings-nav__icon" aria-hidden="true">
											<BadgeOutlinedIcon fontSize="small" />
										</span>
										<span class="settings-nav__text">
											<span class="settings-nav__title">General</span>
											<span class="settings-nav__desc">Name & delete</span>
										</span>
									</button>
									<button
										type="button"
										class="settings-nav__item"
										classList={{
											'settings-nav__item--active': tab() === 'telegram',
										}}
										onClick={() => setTab('telegram')}
									>
										<span class="settings-nav__icon" aria-hidden="true">
											<HubOutlinedIcon fontSize="small" />
										</span>
										<span class="settings-nav__text">
											<span class="settings-nav__title">Channels</span>
											<span class="settings-nav__desc">
												Bot · {channels().length}/{MAX_CHANNELS}
											</span>
										</span>
									</button>
								</nav>

								<div class="settings-modal__body">
									<Show when={tab() === 'general'}>
										<form class="storage-settings-form" onSubmit={saveName}>
											<p class="settings-panel__lead">
												Rename this storage. The name is only shown in Sarca.
											</p>
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

										<div class="storage-settings-danger">
											<p class="settings-panel__lead">
												Permanently delete this storage and all its files. This
												cannot be undone.
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
									</Show>

									<Show when={tab() === 'telegram'}>
										<div class="bot-section">
											<div class="bot-section__head">
												<p class="settings-panel__lead">
													One bot per storage. Paste a new token to replace
													it.
												</p>
												<Show when={!editingBot()}>
													<Button
														variant="outlined"
														size="small"
														onClick={startEditBot}
													>
														{bot() ? 'Change bot' : 'Add bot'}
													</Button>
												</Show>
											</div>

											<Show when={editingBot()}>
												<div class="bot-section__form">
													<TextField
														label="Bot token"
														value={botToken()}
														onChange={(_, v) => {
															setBotToken(v)
															setBotFormError(null)
														}}
														fullWidth
														required
														autoFocus
														autoComplete="off"
														error={Boolean(botFormError())}
														helperText={
															botFormError() ||
															'From @BotFather → your bot → API Token'
														}
														disabled={savingBot()}
													/>
													<div class="bot-section__form-actions">
														<Button
															variant="contained"
															color="secondary"
															size="small"
															disabled={savingBot() || !botToken().trim()}
															onClick={saveBot}
														>
															{savingBot() ? 'Saving…' : 'Save bot'}
														</Button>
														<Button
															size="small"
															disabled={savingBot()}
															onClick={cancelEditBot}
														>
															Cancel
														</Button>
													</div>
												</div>
											</Show>

											<Show when={!editingBot()}>
												<Show
													when={bot()}
													fallback={
														<div class="bot-section__empty">
															No bot attached yet — click Add bot and paste a
															token from @BotFather.
														</div>
													}
												>
													<div class="bot-section__card">
														<span class="bot-section__label">Name</span>
														<span class="bot-section__name">{bot().name}</span>
														<span class="bot-section__label">Token</span>
														<span class="bot-section__token">
															{bot().token_masked}
														</span>
													</div>
												</Show>
											</Show>
										</div>

										<div class="channels-section">
											<div class="channels-section__head">
												<p class="settings-panel__lead">
													Up to {MAX_CHANNELS} Telegram channels for this
													storage.
												</p>
												<Button
													variant="outlined"
													size="small"
													startIcon={<RefreshIcon />}
													disabled={
														!bot() ||
														refreshingChannels() ||
														loadingDetail() ||
														channels().length >= MAX_CHANNELS
													}
													onClick={refreshChannelsFromBot}
												>
													{refreshingChannels() ? 'Refreshing…' : 'Refresh'}
												</Button>
											</div>

											<Show when={detailError()}>
												<p class="channel-row__dead-message">{detailError()}</p>
											</Show>

											<Show when={loadingDetail() && !channels().length}>
												<Typography
													color="text.secondary"
													sx={{ fontSize: '0.85rem' }}
												>
													Loading channels…
												</Typography>
											</Show>

											<div class="channels-list">
												<For each={channels()}>
													{(channel) => (
														<div
															class="channel-row"
															classList={{
																'channel-row--dead': channel.status === 'dead',
															}}
														>
															<Show
																when={editingId() === channel.id}
																fallback={
																	<>
																		<div class="channel-row__top">
																			<div class="channel-row__info">
																				<span class="channel-row__name">
																					{channel.name ||
																						`Channel ${channel.position}`}
																				</span>
																				<span
																					class="channel-row__chatid"
																					title={String(channel.chat_id)}
																				>
																					{channel.chat_id}
																				</span>
																				<span
																					class={`channel-status channel-status--${channel.status}`}
																				>
																					{channel.status === 'active'
																						? 'Active'
																						: 'Deleted'}
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
																					onClick={() =>
																						requestRemoveChannel(channel)
																					}
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
																					sx={{
																						mr: 0.5,
																						verticalAlign: 'text-bottom',
																					}}
																				/>
																				Channel deleted in Telegram. Set a new
																				chat id or add another channel.
																			</p>
																		</Show>
																	</>
																}
															>
																{channelEditor()}
															</Show>
														</div>
													)}
												</For>

												<Show when={editingId() === 'new'}>
													<div class="channel-row">{channelEditor()}</div>
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
													Add channel
												</Button>
											</Show>

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
														Retry failed
													</Button>
												</div>
											</Show>
										</div>
									</Show>
								</div>
							</div>
						</div>
					</div>
				</Portal>
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
