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
import SettingsSyncPanel from './SettingsSyncPanel'
import { nativeClientStore } from '../common/nativeClient'
import FluentIcon from './FluentIcon'
import { t } from '../common/i18n'

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
export const validateChatId = (value) => {
	if (value === '' || value === null || value === undefined) {
		return t('storageDialogs.chatIdRequired')
	}
	const n = Number(value)
	// Number.isInteger (not just isFinite) — a fractional value like "-100.5"
	// used to pass validation here and then get silently truncated by
	// parseInt(draftChatId(), 10) in saveChannel, saving the wrong chat id
	// with no indication to the user that their input was altered.
	if (!Number.isInteger(n) || n >= 0) {
		return t('storageDialogs.chatIdNegativeInteger')
	}
	return null
}

/**
 * @param {StorageSettingsModalProps} props
 */
const StorageSettingsModal = (props) => {
	const { addAlert } = alertStore
	const { isNative } = nativeClientStore
	const [tab, setTab] = createSignal(
		/** @type {'general' | 'sync' | 'telegram'} */ ('general'),
	)
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
	/** Bot token awaiting the "channels will be removed" confirmation. */
	const [pendingBotReplace, setPendingBotReplace] = createSignal(null)

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
			setDetailError(t('storageDialogs.couldNotLoadChannels'))
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
			addAlert(t('storageDialogs.storageNameRequired'), 'error')
			return
		}
		if (next === storage.name) {
			addAlert(t('storageDialogs.nameUnchanged'), 'info')
			return
		}

		setSaving(true)
		try {
			const updated = await API.storages.renameStorage(storage.id, next)
			addAlert(t('storageDialogs.renamedStorage', { name: updated.name }), 'success')
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
			addAlert(t('storageDialogs.deletedStorage', { name: storage.name }), 'success')
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
				addAlert(t('storageDialogs.channelAdded'), 'success')
			} else {
				await API.storages.updateChannel(storage.id, editingId(), {
					chat_id: chatId,
					name: trimmedName || undefined,
				})
				addAlert(t('storageDialogs.channelUpdated'), 'success')
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
			addAlert(t('storageDialogs.cannotRemoveLastActiveChannel'), 'error')
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
			addAlert(t('storageDialogs.channelRemoved'), 'success')
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
			addAlert(t('storageDialogs.retryingFailedUploads'), 'success')
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
				addAlert(
					n === 1
						? t('storageDialogs.addedOneChannel')
						: t('storageDialogs.addedNChannels', { count: n }),
					'success',
				)
			} else if (result.hint) {
				addAlert(result.hint, 'warning')
			} else if (result.skipped_full) {
				addAlert(t('storageDialogs.alreadyAtMaxChannels', { max: MAX_CHANNELS }), 'info')
			} else if (result.skipped_in_use?.length) {
				addAlert(t('storageDialogs.foundChannelsInUse'), 'warning')
			} else {
				addAlert(t('storageDialogs.noNewChannelsFound'), 'info')
			}
		} catch (err) {
			console.error(err)
			addAlert(err?.message || t('storageDialogs.couldNotRefreshChannels'), 'error')
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

	const saveBot = async (removeChannels = false, tokenOverride) => {
		const storage = props.storage
		if (!storage) return

		const token = (tokenOverride ?? botToken()).trim()
		if (!token || !token.includes(':')) {
			setBotFormError(t('storageDialogs.pasteValidBotToken'))
			return
		}

		setSavingBot(true)
		setBotFormError(null)
		try {
			const hadBot = Boolean(bot())
			const next = await API.storages.setStorageBot(storage.id, token, removeChannels)
			setBot(next)
			setEditingBot(false)
			setBotToken('')
			addAlert(
				hadBot
					? t('storageDialogs.botUpdated', { name: next.name })
					: t('storageDialogs.botAttached', { name: next.name }),
				'success',
			)
			await refreshDetail()
			setBot(next)
		} catch (err) {
			console.error(err)
			if (err?.status === 409) {
				// The server refuses a silent bot replacement: its channels would go
				// with it. Ask the user before retrying with the confirmation flag.
				setPendingBotReplace({ token })
				setEditingBot(false)
			} else {
				setBotFormError(err?.message || t('storageDialogs.couldNotSaveBotToken'))
			}
		} finally {
			setSavingBot(false)
		}
	}

	const channelEditor = () => (
		<div class="channel-row__edit-form">
			<TextField
				label={t('storageDialogs.chatIdLabel')}
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
					(editingId() === 'new' ? t('storageDialogs.chatIdHelper') : '')
				}
				fullWidth
				required
				autoFocus
			/>
			<TextField
				label={t('storageDialogs.nameOptionalLabel')}
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
					{t('common.save')}
				</Button>
				<Button size="small" disabled={savingChannel()} onClick={cancelEditChannel}>
					{t('common.cancel')}
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
									<h2 id="storage-settings-title">{t('storageDialogs.storageSettingsTitle')}</h2>
									<p class="settings-modal__sub">
										{props.storage?.name || t('storages.title')}
									</p>
								</div>
								<IconButton
									aria-label={t('storageDialogs.closeStorageSettingsAria')}
									onClick={props.onClose}
									class="sarca-header-icon"
									size="small"
								>
									<CloseIcon />
								</IconButton>
							</div>

							<div class="settings-modal__layout">
								<nav
									class="settings-nav"
									aria-label={t('storageDialogs.storageSettingsSectionsAria')}
								>
									<p class="settings-nav__label">{t('storageDialogs.menuLabel')}</p>
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
											<span class="settings-nav__title">
												{t('storageDialogs.navGeneral')}
											</span>
											<span class="settings-nav__desc">
												{t('storageDialogs.navGeneralDesc')}
											</span>
										</span>
									</button>
									<Show when={isNative()}>
										<button
											type="button"
											class="settings-nav__item"
											classList={{
												'settings-nav__item--active': tab() === 'sync',
											}}
											onClick={() => setTab('sync')}
										>
											<span class="settings-nav__icon" aria-hidden="true">
												<FluentIcon
													name={tab() === 'sync' ? 'cloudFilled' : 'cloud'}
													size={20}
												/>
											</span>
											<span class="settings-nav__text">
												<span class="settings-nav__title">
													{t('storageDialogs.navSync')}
												</span>
												<span class="settings-nav__desc">
													{t('storageDialogs.navSyncDesc')}
												</span>
											</span>
										</button>
									</Show>
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
											<span class="settings-nav__title">
												{t('storageDialogs.navChannels')}
											</span>
											<span class="settings-nav__desc">
												{t('storageDialogs.navChannelsDesc', {
													count: channels().length,
													max: MAX_CHANNELS,
												})}
											</span>
										</span>
									</button>
								</nav>

								<div class="settings-modal__body">
									<Show when={tab() === 'general'}>
										<form class="storage-settings-form" onSubmit={saveName}>
											<p class="settings-panel__lead">
												{t('storageDialogs.renameLead')}
											</p>
											<TextField
												label={t('storageDialogs.nameLabel')}
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
													{t('common.save')}
												</Button>
											</div>
										</form>

										<div class="storage-settings-danger">
											<p class="settings-panel__lead">
												{t('storageDialogs.deleteStorageLead')}
											</p>
											<Button
												variant="outlined"
												color="error"
												startIcon={<DeleteIcon />}
												disabled={saving()}
												onClick={() => setConfirmDelete(true)}
											>
												{t('storageDialogs.deleteStorageAndFiles')}
											</Button>
										</div>
									</Show>

									<Show when={tab() === 'sync' && isNative()}>
										<SettingsSyncPanel
											storageId={props.storage?.id}
											storageName={props.storage?.name}
										/>
									</Show>

									<Show when={tab() === 'telegram'}>
										<div class="bot-section">
											<div class="bot-section__head">
												<p class="settings-panel__lead">
													{t('storageDialogs.botLead')}
												</p>
												<Show when={!editingBot()}>
													<Button
														variant="outlined"
														size="small"
														onClick={startEditBot}
													>
														{bot()
															? t('storageDialogs.changeBot')
															: t('storageDialogs.addBot')}
													</Button>
												</Show>
											</div>

											<Show when={editingBot()}>
												<div class="bot-section__form">
													<TextField
														label={t('storageDialogs.botTokenLabel')}
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
															t('storageDialogs.botTokenHelper')
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
															{savingBot()
																? t('storageDialogs.savingBot')
																: t('storageDialogs.saveBotButton')}
														</Button>
														<Button
															size="small"
															disabled={savingBot()}
															onClick={cancelEditBot}
														>
															{t('common.cancel')}
														</Button>
													</div>
												</div>
											</Show>

											<Show when={!editingBot()}>
												<Show
													when={bot()}
													fallback={
														<div class="bot-section__empty">
															{t('storageDialogs.noBotAttached')}
														</div>
													}
												>
													<div class="bot-section__card">
														<span class="bot-section__label">
															{t('storageDialogs.botNameLabel')}
														</span>
														<span class="bot-section__name">{bot().name}</span>
														<span class="bot-section__label">
															{t('storageDialogs.botTokenFieldLabel')}
														</span>
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
													{t('storageDialogs.channelsLead', { max: MAX_CHANNELS })}
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
													{refreshingChannels()
														? t('storageDialogs.refreshing')
														: t('storageDialogs.refresh')}
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
													{t('storageDialogs.loadingChannels')}
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
																								t('storageDialogs.channelPosition', { position: channel.position })}
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
																								? t('storageDialogs.channelActive')
																								: t('storageDialogs.channelDeleted')}
																				</span>
																			</div>
																			<div class="channel-row__actions">
																				<IconButton
																					size="small"
																							aria-label={t('storageDialogs.editChannelAria', { name: channel.name || channel.position })}
																					onClick={() => startEditChannel(channel)}
																				>
																					<EditIcon fontSize="small" />
																				</IconButton>
																				<IconButton
																					size="small"
																							aria-label={t('storageDialogs.removeChannelAria', { name: channel.name || channel.position })}
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
																				{t('storageDialogs.channelDeadMessage')}
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
													{t('storageDialogs.addChannel')}
												</Button>
											</Show>

											<Show when={replication()}>
												<div class="replication-summary">
													<h3>{t('storageDialogs.replicationTitle')}</h3>
													<div class="replication-summary__stats">
														<span>
															{t('storageDialogs.uploadedCount', {
																count: replication().uploaded,
															})}
														</span>
														<span>
															{t('storageDialogs.pendingCount', {
																count: replication().pending,
															})}
														</span>
														<span>
															{t('storageDialogs.failedCount', {
																count: replication().failed,
															})}
														</span>
													</div>
													<Button
														variant="outlined"
														color="warning"
														size="small"
														startIcon={<RefreshIcon />}
														disabled={retrying() || !replication().failed}
														onClick={retryReplication}
													>
														{t('storageDialogs.retryFailed')}
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
				entity={t('storageDialogs.storageEntity')}
				action={t('common.delete')}
				actionDescription={t('storageDialogs.deleteStorageDescription', {
					name: props.storage?.name || '',
				})}
				onConfirm={deleteStorage}
				onCancel={() => setConfirmDelete(false)}
			/>

			<ActionConfirmDialog
				isOpened={Boolean(pendingBotReplace())}
				entity={t('storageDialogs.botEntity')}
				action={t('storageDialogs.changeBot')}
				actionDescription={t('storageDialogs.replaceBotDescription', {
					name: bot()?.name || '',
				})}
				onConfirm={() => {
					const pending = pendingBotReplace()
					setPendingBotReplace(null)
					if (pending) void saveBot(true, pending.token)
				}}
				onCancel={() => setPendingBotReplace(null)}
			/>

			<ActionConfirmDialog
				isOpened={Boolean(pendingRemoveChannel())}
				entity={t('storageDialogs.channelEntity')}
				action={t('storageDialogs.removeAction')}
				actionDescription={t('storageDialogs.removeChannelDescription', {
					name:
						pendingRemoveChannel()?.name ||
						`#${pendingRemoveChannel()?.position}`,
					chatId: pendingRemoveChannel()?.chat_id,
				})}
				onConfirm={confirmRemoveChannel}
				onCancel={() => setPendingRemoveChannel(null)}
			/>
		</>
	)
}

export default StorageSettingsModal
