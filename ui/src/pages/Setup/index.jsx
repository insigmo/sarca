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

const POLL_MS = 2000
const POLL_TIMEOUT_MS = 120_000
const MAX_CHANNELS = 3

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

	let pollTimer = null
	let pollStartedAt = 0

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
		if (pollTimer) clearInterval(pollTimer)
	})

	const stopPolling = () => {
		if (pollTimer) {
			clearInterval(pollTimer)
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
		} catch {
			/* apiRequest already alerts */
		} finally {
			setBusy(false)
		}
	}

	const NOT_ADDED_MSG =
		'Bot was not added to a channel, or was not given admin rights.'

	const startPolling = () => {
		stopPolling()
		setPollError('')
		setPolling(true)
		pollStartedAt = Date.now()
		const tick = async () => {
			if (Date.now() - pollStartedAt > POLL_TIMEOUT_MS) {
				stopPolling()
				setPollError(NOT_ADDED_MSG)
				return
			}
			try {
				const exclude = channels().map((c) => c.chat_id)
				const res = await API.setup.pollChannel(token().trim(), exclude)
				if (res.found && res.chat_id != null) {
					stopPolling()
					setChannels((list) => [
						...list,
						{ chat_id: res.chat_id, title: res.title || String(res.chat_id) },
					])
					addAlert(`Found channel: ${res.title || res.chat_id}`, 'success')
					return
				}
				if (res.hint) {
					stopPolling()
					setPollError(res.hint)
				}
			} catch (e) {
				stopPolling()
				setPollError(e?.message || 'Channel detect failed')
			}
		}
		tick()
		pollTimer = setInterval(tick, POLL_MS)
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
									admin with permission.
								</Typography>

								<Show when={polling()}>
									<Stack direction="row" spacing={1} alignItems="center">
										<CircularProgress size={22} />
										<Typography variant="body2">
											Checking whether the bot was added as an admin…
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
