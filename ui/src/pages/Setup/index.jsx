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

const POLL_MS = 0
const POLL_TIMEOUT_MS = 120_000
const MAX_CHANNELS = 3

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
 * Two-phase setup wizard: Local Bot API (optional/once) → bot + channel detect → storage.
 */
const SetupWizard = () => {
	const navigate = useNavigate()
	const { addAlert } = alertStore

	const [loading, setLoading] = createSignal(true)
	const [phase, setPhase] = createSignal('boot') // boot | local | storage
	const [status, setStatus] = createSignal(null)

	// Phase A
	const [apiId, setApiId] = createSignal('')
	const [apiHash, setApiHash] = createSignal('')
	const [localBusy, setLocalBusy] = createSignal(false)
	const [localHint, setLocalHint] = createSignal('')

	// Phase B
	const [step, setStep] = createSignal(0) // 0 name, 1 bot, 2 channel, 3 done prep
	const [storageName, setStorageName] = createSignal('')
	const [token, setToken] = createSignal('')
	const [botUsername, setBotUsername] = createSignal('')
	const [channels, setChannels] = createSignal([])
	const [polling, setPolling] = createSignal(false)
	const [pollError, setPollError] = createSignal('')
	const [finishing, setFinishing] = createSignal(false)
	const [busy, setBusy] = createSignal(false)
	const [chatIdInput, setChatIdInput] = createSignal('')
	const [probeBusy, setProbeBusy] = createSignal(false)

	let pollTimer = null
	let pollStartedAt = 0
	let pollEpoch = 0
	let pendingProbeIds = []

	onMount(async () => {
		try {
			const s = await API.setup.getSetupStatus()
			setStatus(s)
			setPhase(s.needs_local_api_phase ? 'local' : 'storage')
		} catch (e) {
			addAlert('Failed to load setup status', 'error')
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

	const goStoragePhase = () => setPhase('storage')

	const handleSaveLocal = async () => {
		setLocalBusy(true)
		setLocalHint('')
		try {
			const res = await API.setup.saveLocalApi(apiId().trim(), apiHash().trim())
			setLocalHint(
				[
					res.saved_to_conf
						? 'Saved to sarca.conf and app settings.'
						: 'Saved to app settings (could not write sarca.conf).',
					res.restart_hint,
				]
					.filter(Boolean)
					.join(' '),
			)
			addAlert('Credentials saved', 'success')
		} catch {
			/* apiRequest already alerts */
		} finally {
			setLocalBusy(false)
		}
	}

	const handleVerifyLocal = async () => {
		setLocalBusy(true)
		try {
			const res = await API.setup.verifyLocalApi()
			setLocalHint(res.message)
			if (res.ok && res.uses_local_api) {
				addAlert('Local Bot API reachable', 'success')
				const s = await API.setup.getSetupStatus()
				setStatus(s)
				if (!s.needs_local_api_phase) goStoragePhase()
			} else if (res.ok) {
				addAlert(res.message, 'info')
			} else {
				addAlert(res.message, 'warning')
			}
		} catch {
			/* apiRequest already alerts */
		} finally {
			setLocalBusy(false)
		}
	}

	const handleSkipLocal = async () => {
		setLocalBusy(true)
		try {
			await API.setup.skipLocalApi()
			addAlert('Skipped Local Bot API — uploads limited to ~20 MB', 'warning')
			goStoragePhase()
		} catch {
			/* apiRequest already alerts */
		} finally {
			setLocalBusy(false)
		}
	}

	const handleValidateBot = async () => {
		setBusy(true)
		try {
			const res = await API.setup.validateBot(token().trim())
			setBotUsername(res.username)
			addAlert(`Bot @${res.username} looks good`, 'success')
			setPollError('')
			stopPolling()
			setStep(2)
			// Listen immediately so my_chat_member is caught when the user adds the bot.
			queueMicrotask(() => startPolling())
		} catch {
			/* apiRequest already alerts */
		} finally {
			setBusy(false)
		}
	}

	const NOT_ADDED_MSG =
		'Bot was not added to a channel, or was not given admin rights. Re-add the bot as admin while checking runs, or paste the channel id below.'

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
				setPollError(NOT_ADDED_MSG)
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
							? `Found channel: ${labels}`
							: `Found ${Math.min(hits.length, MAX_CHANNELS - exclude.length)} channels: ${labels}`,
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
				setPollError(e?.message || 'Channel detect failed')
			}
		}
		tick()
	}

	const handleProbeChatId = async () => {
		const chatId = parseTelegramChatId(chatIdInput())
		if (chatId == null) {
			addAlert('Enter a chat id like -100… or a t.me/c/… link', 'warning')
			return
		}
		if (channels().some((c) => c.chat_id === chatId)) {
			addAlert('That channel is already added', 'info')
			return
		}
		setChatIdInput('')
		setPollError('')
		// Avoid concurrent getUpdates (409): feed the active poller when possible.
		if (polling()) {
			pendingProbeIds.push(chatId)
			addAlert('Verifying chat id…', 'info')
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
						? `Found channel: ${labels}`
						: `Found ${hits.length} channels: ${labels}`,
					'success',
				)
				return
			}
			setPollError(
				res.hint ||
					'Could not verify that chat id. Check the bot is an admin there.',
			)
		} catch (e) {
			setPollError(e?.message || 'Channel verify failed')
		} finally {
			setProbeBusy(false)
		}
	}

	const handleFinish = async () => {
		if (!channels().length) {
			addAlert('Add at least one channel', 'error')
			return
		}
		setFinishing(true)
		try {
			const created = await API.setup.setupCreateStorage(
				storageName().trim(),
				token().trim(),
				channels().map((c) => c.chat_id),
			)
			addAlert(`Storage “${created.name}” ready`, 'success')
			navigate(`/storages/${created.id}/files`)
		} catch {
			/* apiRequest already alerts */
		} finally {
			setFinishing(false)
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
					<h1>Setup</h1>
				</div>

				<Show when={phase() === 'local'}>
					<Box class="setup-wizard__card">
						<Typography variant="h5" component="h2" gutterBottom>
							Local Bot API
						</Typography>
						<Typography color="text.secondary" sx={{ mb: 2 }}>
							For files larger than ~20&nbsp;MB, Sarca needs Telegram’s Local Bot API.
							Get <code>api_id</code> and <code>api_hash</code> from{' '}
							<Link href="https://my.telegram.org" target="_blank" rel="noreferrer">
								my.telegram.org
							</Link>{' '}
							→ API development tools.
						</Typography>
						<Show when={status()?.uses_local_api === false}>
							<Typography color="text.secondary" sx={{ mb: 2 }}>
								This server is currently on the official Bot API. After saving
								credentials, set <code>TELEGRAM_LOCAL_API=true</code> in{' '}
								<code>sarca.conf</code>, start Local Bot API, and restart Sarca.
							</Typography>
						</Show>
						<Box
							component="form"
							onSubmit={(e) => {
								e.preventDefault()
								if (localBusy() || !apiId().trim() || !apiHash().trim()) return
								handleSaveLocal()
							}}
						>
							<Stack spacing={2}>
								<TextField
									label="api_id"
									value={apiId()}
									onChange={(e) => setApiId(e.target.value)}
									autoComplete="off"
								/>
								<TextField
									label="api_hash"
									value={apiHash()}
									onChange={(e) => setApiHash(e.target.value)}
									autoComplete="off"
								/>
								<Show when={localHint()}>
									<Typography variant="body2" color="text.secondary">
										{localHint()}
									</Typography>
								</Show>
								<Stack direction={{ xs: 'column', sm: 'row' }} spacing={1}>
									<Button
										type="submit"
										variant="contained"
										disabled={
											localBusy() || !apiId().trim() || !apiHash().trim()
										}
									>
										Save
									</Button>
									<Button
										type="button"
										variant="outlined"
										disabled={localBusy()}
										onClick={handleVerifyLocal}
									>
										Verify
									</Button>
									<Button
										type="button"
										color="inherit"
										disabled={localBusy()}
										onClick={handleSkipLocal}
									>
										Skip for now
									</Button>
								</Stack>
							</Stack>
						</Box>
					</Box>
				</Show>

				<Show when={phase() === 'storage'}>
					<Box class="setup-wizard__card">
						<Typography variant="h5" component="h2" gutterBottom>
							New storage
						</Typography>
						<Typography color="text.secondary" sx={{ mb: 2 }}>
							Create a Telegram bot and a private channel, then check that the bot
							was added as an admin.
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
										label="Storage name"
										value={storageName()}
										onChange={(e) => setStorageName(e.target.value)}
										autoFocus
									/>
									<Button
										type="submit"
										variant="contained"
										disabled={!storageName().trim()}
									>
										Continue
									</Button>
								</Stack>
							</Box>
						</Show>

						<Show when={step() === 1}>
							<Box
								component="form"
								onSubmit={(e) => {
									e.preventDefault()
									if (busy() || !token().trim()) return
									handleValidateBot()
								}}
							>
								<Stack spacing={2}>
									<Typography>
										1. Click{' '}
										<Link
											href="https://t.me/BotFather"
											target="_blank"
											rel="noreferrer"
										>
											@BotFather
										</Link>
									</Typography>
									<Typography>
										2. Send a command <code>/newbot</code>
									</Typography>
									<Typography>
										3. Create new bot and copy the token.
									</Typography>
									<TextField
										label="Bot token"
										value={token()}
										onChange={(e) => setToken(e.target.value)}
										autoComplete="off"
										autoFocus
									/>
									<Stack direction="row" spacing={1}>
										<Button type="button" onClick={() => setStep(0)}>
											Back
										</Button>
										<Button
											type="submit"
											variant="contained"
											disabled={busy() || !token().trim()}
										>
											Validate bot
										</Button>
									</Stack>
								</Stack>
							</Box>
						</Show>

						<Show when={step() === 2}>
							<Stack spacing={2}>
								<Show when={botUsername()}>
									<Typography>
										Bot: <strong>@{botUsername()}</strong>
									</Typography>
								</Show>
								<Typography>
									1. Create a <strong>private channel</strong> in Telegram.
								</Typography>
								<Typography>
									2. Add <strong>@{botUsername() || 'your bot'}</strong> as an
									admin <em>while checking runs</em>,{' '}
									<strong>or</strong> forward any post from the channel to the
									bot in a private chat, <strong>or</strong> paste a{' '}
									<code>t.me/c/…</code> link / chat id below.
								</Typography>

								<Show when={polling()}>
									<Stack direction="row" spacing={1} alignItems="center">
										<CircularProgress size={22} />
										<Typography variant="body2">
											Listening for admin channels…
										</Typography>
									</Stack>
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
											label="Chat id or t.me/c/… link"
											value={chatIdInput()}
											onChange={(e) => setChatIdInput(e.target.value)}
											helperText="Private channel: forward a post to the bot, or paste t.me/c/… / -100…"
											fullWidth
										/>
										<Button
											variant="outlined"
											disabled={probeBusy() || !chatIdInput().trim()}
											onClick={handleProbeChatId}
											sx={{ flexShrink: 0, mt: { sm: 0.5 } }}
										>
											{probeBusy() ? 'Checking…' : 'Add by id'}
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
										Back
									</Button>
									<Show when={!polling() && channels().length === 0}>
										<Button variant="contained" onClick={startPolling}>
											Check channel
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
											Stop
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
											Detect another channel
										</Button>
									</Show>
									<Show when={!polling() && pollError() && !channels().length}>
										<Button variant="outlined" onClick={startPolling}>
											Try again
										</Button>
									</Show>
									<Button
										variant="contained"
										disabled={!channels().length || finishing() || polling()}
										onClick={handleFinish}
									>
										{finishing() ? 'Creating…' : 'Finish'}
									</Button>
								</Stack>
							</Stack>
						</Show>
					</Box>
				</Show>
			</Stack>
		</Show>
	)
}

export default SetupWizard
