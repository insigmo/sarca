import Box from '@suid/material/Box'
import Button from '@suid/material/Button'
import Link from '@suid/material/Link'
import Stack from '@suid/material/Stack'
import TextField from '@suid/material/TextField'
import Typography from '@suid/material/Typography'
import CircularProgress from '@suid/material/CircularProgress'
import Chip from '@suid/material/Chip'
import { For, Show, createEffect, createSignal, onCleanup, onMount } from 'solid-js'
import { useNavigate } from '@solidjs/router'

import API from '../../api'
import { alertStore } from '../../components/AlertStack'
import LoadingDots from '../../components/LoadingDots'
import { busyStore } from '../../common/busyStore'
import { storagesStore } from '../../common/storagesStore'
import { t } from '../../common/i18n'

const POLL_MS = 0
const POLL_TIMEOUT_MS = 120_000
const MAX_CHANNELS = 3
const BOT_TOKEN_RE = /^\d+:[A-Za-z0-9_-]{35}$/

/**
 * Format-only sanity check — deliberately does not call Telegram. A real
 * getMe() round trip hangs the wizard indefinitely when the server can't
 * reach api.telegram.org; the actual bot/channel access gets verified for
 * real on the "Check channel" step instead.
 * @param {string} value
 * @returns {boolean}
 */
export const isValidBotToken = (value) => BOT_TOKEN_RE.test(String(value ?? '').trim())

/**
 * Parse a Telegram channel id from raw number or t.me/c/<id> link.
 * @param {string} input
 * @returns {number | null}
 */
export const parseTelegramChatId = (input) => {
	const s = String(input ?? '').trim()
	if (!s) return null
	const link = s.match(/(?:t\.me|telegram\.me)\/c\/(\d+)/i)
	if (link) {
		const id = Number(`-100${link[1]}`)
		return Number.isSafeInteger(id) ? id : null
	}
	if (!/^-?\d+$/.test(s)) return null
	const n = Number(s)
	if (!Number.isSafeInteger(n)) return null
	if (n < 0) return n
	// Bare internal id (123…) or 100-prefixed absolute form.
	if (s.startsWith('100') && s.length > 3) return -n
	const id = Number(`-100${s}`)
	return Number.isSafeInteger(id) ? id : null
}

/**
 * Setup wizard: bot + channel detect → storage.
 */
const SetupWizard = () => {
	const navigate = useNavigate()
	const { addAlert } = alertStore
	const { storages: existingStorages } = storagesStore

	const [loading, setLoading] = createSignal(true)

	// Storage setup
	const [step, setStep] = createSignal(0) // 0 name, 1 bot, 2 channel, 3 done prep
	const [storageName, setStorageName] = createSignal('')
	const [token, setToken] = createSignal('')
	const [botUsername, setBotUsername] = createSignal('')
	const [channels, setChannels] = createSignal([])
	const [polling, setPolling] = createSignal(false)
	const [pollError, setPollError] = createSignal('')
	const [finishing, setFinishing] = createSignal(false)
	const [chatIdInput, setChatIdInput] = createSignal('')
	const [probeBusy, setProbeBusy] = createSignal(false)

	let pollTimer = null
	let pollStartedAt = 0
	let pollEpoch = 0
	let pendingProbeIds = []

	onMount(async () => {
		storagesStore.refreshStorages().catch(() => {})
		try {
			await API.setup.getSetupStatus()
		} catch {
			addAlert(t('setup.loadStatusFailed'), 'error')
			navigate('/storages')
		} finally {
			setLoading(false)
		}
	})

	onCleanup(() => {
		pollEpoch += 1
		if (pollTimer) clearTimeout(pollTimer)
	})

	const stopPolling = () => {
		pollEpoch += 1
		if (pollTimer) {
			clearTimeout(pollTimer)
			pollTimer = null
		}
		setPolling(false)
	}

	createEffect(() => {
		// stop polling when leaving channel step
		if (step() !== 2) stopPolling()
	})

	const handleValidateBot = () => {
		const trimmed = token().trim()
		if (!isValidBotToken(trimmed)) {
			addAlert(t('setup.botTokenInvalid'), 'error')
			return
		}
		setBotUsername('')
		setChannels([])
		setPollError('')
		stopPolling()
		setStep(2)
	}

	const startPolling = () => {
		stopPolling()
		setPollError('')
		setPolling(true)
		pollStartedAt = Date.now()
		// Capture epoch after stopPolling bump so in-flight ticks from a previous
		// run (or overlapping setInterval) cannot call getUpdates again → 409.
		const epoch = pollEpoch
		const scheduleNext = () => {
			if (epoch !== pollEpoch || !polling()) return
			pollTimer = setTimeout(tick, POLL_MS)
		}
		const tick = async () => {
			pollTimer = null
			if (epoch !== pollEpoch) return
			if (Date.now() - pollStartedAt > POLL_TIMEOUT_MS) {
				stopPolling()
				setPollError(t('setup.notAddedMsg'))
				return
			}
			try {
				const exclude = channels().map((c) => c.chat_id)
				const probe = pendingProbeIds.splice(0, pendingProbeIds.length)
				const res = await API.setup.pollChannel(token().trim(), exclude, probe)
				if (epoch !== pollEpoch) return
				const hits = Array.isArray(res.channels) ? res.channels : []
				if (hits.length) {
					setChannels((list) => {
						const next = [...list]
						for (const hit of hits) {
							if (next.length >= MAX_CHANNELS) break
							if (next.some((c) => c.chat_id === hit.chat_id)) continue
							next.push({
								chat_id: hit.chat_id,
								title: hit.title || String(hit.chat_id),
							})
						}
						return next
					})
					const labels = hits
						.slice(0, MAX_CHANNELS - exclude.length)
						.map((h) => h.title || h.chat_id)
						.join(', ')
					addAlert(
						hits.length === 1
							? t('setup.foundChannelOne', { labels })
							: t('setup.foundChannelMany', {
									n: Math.min(hits.length, MAX_CHANNELS - exclude.length),
									labels,
								}),
						'success',
					)
					// Keep polling until the storage is full (up to 3).
					if (exclude.length + hits.length >= MAX_CHANNELS) {
						stopPolling()
						return
					}
					scheduleNext()
					return
				}
				if (res.hint) {
					setPollError(res.hint)
				}
				scheduleNext()
			} catch (e) {
				if (epoch !== pollEpoch) return
				stopPolling()
				setPollError(e?.message || t('setup.channelDetectFailed'))
			}
		}
		tick()
	}

	const handleProbeChatId = async () => {
		const chatId = parseTelegramChatId(chatIdInput())
		if (chatId == null) {
			addAlert(t('setup.enterChatIdHint'), 'warning')
			return
		}
		if (channels().some((c) => c.chat_id === chatId)) {
			addAlert(t('setup.channelAlreadyAdded'), 'info')
			return
		}
		setChatIdInput('')
		setPollError('')
		// Avoid concurrent getUpdates (409): feed the active poller when possible.
		if (polling()) {
			pendingProbeIds.push(chatId)
			addAlert(t('setup.verifyingChatId'), 'info')
			return
		}
		setProbeBusy(true)
		try {
			const exclude = channels().map((c) => c.chat_id)
			const res = await API.setup.pollChannel(token().trim(), exclude, [chatId])
			const hits = Array.isArray(res.channels) ? res.channels : []
			if (hits.length) {
				setChannels((list) => {
					const next = [...list]
					for (const hit of hits) {
						if (next.length >= MAX_CHANNELS) break
						if (next.some((c) => c.chat_id === hit.chat_id)) continue
						next.push({
							chat_id: hit.chat_id,
							title: hit.title || String(hit.chat_id),
						})
					}
					return next
				})
				const labels = hits.map((h) => h.title || h.chat_id).join(', ')
				addAlert(
					hits.length === 1
						? t('setup.foundChannelOne', { labels })
						: t('setup.foundChannelMany', { n: hits.length, labels }),
					'success',
				)
				return
			}
			setPollError(res.hint || t('setup.chatIdVerifyFailed'))
		} catch (e) {
			setPollError(e?.message || t('setup.channelVerifyFailed'))
		} finally {
			setProbeBusy(false)
		}
	}

	// First-time setup has nowhere to cancel back to (no storage exists yet);
	// only offer the way out once there is at least one already.
	const canCancel = () => existingStorages().length > 0

	const handleCancel = () => {
		if (finishing() || busyStore.isStorageCreating()) return
		stopPolling()
		navigate('/storages')
	}

	const handleFinish = async () => {
		if (!channels().length) {
			addAlert(t('setup.addAtLeastOneChannel'), 'error')
			return
		}
		// Stop listening so Finish is not blocked and we do not keep getUpdates
		// running while create_storage / set_bot talk to Telegram.
		stopPolling()
		setFinishing(true)
		busyStore.setStorageCreating(true)
		try {
			const created = await API.setup.setupCreateStorage(
				storageName().trim(),
				token().trim(),
				channels().map((c) => c.chat_id),
			)
			// Sync the shared store before leaving: the Storages page redirects
			// back here whenever it reads `loaded() && !storages().length`, and
			// without this the stale pre-creation list (still empty on a first
			// storage) sent the user straight back into "add a storage" the next
			// time they opened Storages, until a hard page reload happened to
			// refetch it.
			await storagesStore.refreshStorages().catch(() => {})
			addAlert(t('setup.storageReady', { name: created.name }), 'success')
			navigate(`/storages/${created.id}/files`)
		} catch {
			/* apiRequest already alerts */
		} finally {
			setFinishing(false)
			busyStore.setStorageCreating(false)
		}
	}

	return (
		<Show
			when={!loading()}
			fallback={
				<Box sx={{ display: 'flex', justifyContent: 'center', py: 8 }}>
					<CircularProgress />
				</Box>
			}
		>
			<Stack class="setup-wizard" spacing={2.5}>
				<div class="page-header">
					<h1>{t('setup.title')}</h1>
					<Show when={canCancel()}>
						<Button
							onClick={handleCancel}
							disabled={finishing() || busyStore.isStorageCreating()}
						>
							{t('setup.cancel')}
						</Button>
					</Show>
				</div>

				<Box class="setup-wizard__card">
					<Typography variant="h5" component="h2" gutterBottom>
						{t('setup.newStorage')}
					</Typography>
					<Typography color="text.secondary" sx={{ mb: 2 }}>
						{t('setup.newStorageDesc')}
					</Typography>

					<Show when={step() === 0}>
						<Box
							component="form"
							onSubmit={(e) => {
								e.preventDefault()
								if (!storageName().trim()) return
								setStep(1)
							}}
						>
							<Stack spacing={2}>
								<TextField
									label={t('setup.storageName')}
									value={storageName()}
									onChange={(e) => setStorageName(e.target.value)}
									autoFocus
								/>
								<Button
									type="submit"
									variant="contained"
									disabled={!storageName().trim()}
								>
									{t('setup.continue')}
								</Button>
							</Stack>
						</Box>
					</Show>

					<Show when={step() === 1}>
						<Box
							component="form"
							onSubmit={(e) => {
								e.preventDefault()
								if (!token().trim()) return
								handleValidateBot()
							}}
						>
							<Stack spacing={2}>
								<Typography>
									{t('setup.botStep1Click')}{' '}
									<Link
										href="https://t.me/BotFather"
										target="_blank"
										rel="noreferrer"
									>
										@BotFather
									</Link>
								</Typography>
								<Typography>
									{t('setup.botStep2')} <code>/newbot</code>
								</Typography>
								<Typography>
									{t('setup.botStep3')}
								</Typography>
								<TextField
									label={t('setup.botToken')}
									value={token()}
									onChange={(e) => setToken(e.target.value)}
									autoComplete="off"
									autoFocus
								/>
								<Stack direction="row" spacing={1}>
									<Button type="button" onClick={() => setStep(0)}>
										{t('setup.back')}
									</Button>
									<Button
										type="submit"
										variant="contained"
										disabled={!token().trim()}
									>
										{t('setup.validateBot')}
									</Button>
								</Stack>
							</Stack>
						</Box>
					</Show>

					<Show when={step() === 2}>
						<Stack spacing={2}>
							<Show when={botUsername()}>
								<Typography>
									{t('setup.botLabel')} <strong>@{botUsername()}</strong>
								</Typography>
							</Show>
							<Typography>
								{t('setup.channelStep1Prefix')}{' '}
								<strong>{t('setup.channelStep1Bold')}</strong>{' '}
								{t('setup.channelStep1Suffix')}
							</Typography>
							<Typography>
								{t('setup.channelStep2Prefix')}{' '}
								<strong>@{botUsername() || t('setup.yourBot')}</strong>{' '}
								{t('setup.channelStep2Admin')}{' '}
								<em>{t('setup.channelStep2WhileChecking')}</em>,{' '}
								<strong>{t('setup.or')}</strong>{' '}
								{t('setup.channelStep2Forward')}{' '}
								<strong>{t('setup.or')}</strong>{' '}
								{t('setup.channelStep2PasteLink')}{' '}
								<code>t.me/c/…</code> {t('setup.channelStep2LinkSuffix')}
							</Typography>

							<Show when={polling()}>
								<Typography variant="body2">
									{t('setup.listeningForChannels')}
									<LoadingDots />
								</Typography>
							</Show>

							<Show when={pollError()}>
								<Typography color="error">{pollError()}</Typography>
							</Show>

							<Show when={channels().length}>
								<Stack direction="row" spacing={1} sx={{ flexWrap: 'wrap', gap: 1 }}>
									<For each={channels()}>
										{(ch) => (
											<Chip
												label={`${ch.title} (${ch.chat_id})`}
												onDelete={
													polling()
														? undefined
														: () =>
																setChannels((list) =>
																	list.filter(
																		(c) =>
																			c.chat_id !==
																			ch.chat_id,
																	),
																)
												}
											/>
										)}
									</For>
								</Stack>
							</Show>

							<Show when={channels().length < MAX_CHANNELS}>
								<Stack
									direction={{ xs: 'column', sm: 'row' }}
									spacing={1}
									alignItems={{ sm: 'flex-start' }}
								>
									<TextField
										label={t('setup.chatIdLabel')}
										value={chatIdInput()}
										onChange={(e) => setChatIdInput(e.target.value)}
										helperText={t('setup.chatIdHelper')}
										fullWidth
									/>
									<Button
										variant="outlined"
										disabled={probeBusy() || !chatIdInput().trim()}
										onClick={handleProbeChatId}
										sx={{ flexShrink: 0, mt: { sm: 0.5 } }}
									>
										{probeBusy() ? t('setup.checking') : t('setup.addById')}
									</Button>
								</Stack>
							</Show>

							<Stack direction={{ xs: 'column', sm: 'row' }} spacing={1}>
								<Button
									onClick={() => {
										stopPolling()
										setPollError('')
										setStep(1)
									}}
								>
									{t('setup.back')}
								</Button>
								<Show when={!polling() && channels().length === 0}>
									<Button variant="contained" onClick={startPolling}>
										{t('setup.checkChannel')}
									</Button>
								</Show>
								<Show when={polling()}>
									<Button
										variant="outlined"
										onClick={() => {
											stopPolling()
											setPollError('')
										}}
									>
										{t('setup.stop')}
									</Button>
								</Show>
								<Show
									when={
										!polling() &&
										channels().length > 0 &&
										channels().length < MAX_CHANNELS
									}
								>
									<Button variant="outlined" onClick={startPolling}>
										{t('setup.detectAnotherChannel')}
									</Button>
								</Show>
								<Button
									variant="contained"
									disabled={!channels().length || finishing()}
									onClick={handleFinish}
								>
									{finishing() ? t('setup.creating') : t('setup.finish')}
								</Button>
							</Stack>
						</Stack>
					</Show>
				</Box>
			</Stack>
		</Show>
	)
}

export default SetupWizard
