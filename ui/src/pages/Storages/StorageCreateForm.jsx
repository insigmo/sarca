import Divider from '@suid/material/Divider'
import Box from '@suid/material/Box'
import Button from '@suid/material/Button'
import TextField from '@suid/material/TextField'
import Typography from '@suid/material/Typography'
import { For, createSignal } from 'solid-js'
import { useNavigate } from '@solidjs/router'
import Stack from '@suid/material/Stack'
import IconButton from '@suid/material/IconButton'
import HelpOutlineIcon from '@suid/icons-material/HelpOutline'
import ChevronLeftIcon from '@suid/icons-material/ChevronLeft'
import AddIcon from '@suid/icons-material/Add'
import DeleteIcon from '@suid/icons-material/Delete'

import API from '../../api'
import { alertStore } from '../../components/AlertStack'

const MAX_CHANNELS = 3

let nextKey = 0
const emptyChannel = () => ({ key: nextKey++, chatId: '', name: '', error: null })

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
	// No additional validation - accept any negative number
	// Both regular groups (-XXXXXXXXX) and supergroups (-100XXXXXXXXXX) are valid
	return null
}

const StorageCreateForm = () => {
	const [name, setName] = createSignal('')
	const [nameErr, setNameErr] = createSignal(null)
	const [channels, setChannels] = createSignal([emptyChannel()])
	const { addAlert } = alertStore
	const navigate = useNavigate()

	const updateChannel = (key, patch) => {
		setChannels((list) =>
			list.map((c) => (c.key === key ? { ...c, ...patch } : c)),
		)
	}

	const addChannelRow = () => {
		if (channels().length >= MAX_CHANNELS) return
		setChannels((list) => [...list, emptyChannel()])
	}

	const removeChannelRow = (key) => {
		setChannels((list) => list.filter((c) => c.key !== key))
	}

	/**
	 *
	 * @param {SubmitEvent} event
	 */
	const handleSubmit = async (event) => {
		event.preventDefault()

		const trimmedName = name().trim()
		if (!trimmedName) {
			setNameErr('Name is required')
			return
		}
		setNameErr(null)

		let hasError = false
		setChannels((list) =>
			list.map((c) => {
				const error = validateChatId(c.chatId)
				if (error) hasError = true
				return { ...c, error }
			}),
		)
		if (hasError) return

		const payload = channels().map((c) => {
			const trimmedChannelName = c.name.trim()
			return {
				chat_id: parseInt(c.chatId, 10),
				...(trimmedChannelName ? { name: trimmedChannelName } : {}),
			}
		})

		await API.storages.createStorage(trimmedName, payload)

		addAlert(`Created storage "${trimmedName}"`, 'success')

		navigate('/storages')
	}

	return (
		<Stack sx={{ maxWidth: 540, minWidth: 320, mx: 'auto' }} class="glass-panel" style={{ padding: '24px 28px 32px' }}>
			<Box>
				<Button
					onClick={() => navigate('/storages')}
					variant="outlined"
					startIcon={<ChevronLeftIcon />}
				>
					Back
				</Button>
			</Box>

			<Box
				component="form"
				onSubmit={handleSubmit}
				sx={{
					py: 2,
					mx: 'auto',
					maxWidth: 420,
					display: 'flex',
					flexDirection: 'column',
					alignItems: 'stretch',
					'& > :not(style)': { my: 1.5 },
				}}
			>
				<Typography variant="h5" sx={{ textAlign: 'center' }}>
					Register new storage
					<a
						href="https://github.com/insigmo/sarca#usage"
						target="_blank"
					>
						<IconButton color="warning" sx={{ py: 0 }}>
							<HelpOutlineIcon />
						</IconButton>
					</a>
				</Typography>
				<Divider />
				<TextField
					id="name"
					name="name"
					label="Name"
					variant="standard"
					value={name()}
					onChange={(_, v) => setName(v)}
					error={typeof nameErr() === 'string'}
					helperText={nameErr() || ''}
					fullWidth
					required
				/>

				<Divider textAlign="left">Channels</Divider>

				<For each={channels()}>
					{(channel, index) => (
						<Box
							sx={{
								display: 'flex',
								flexDirection: 'column',
								gap: 1,
								p: 1.5,
								borderRadius: 2,
								border: '1px solid rgba(127,127,127,0.25)',
							}}
						>
							<Box sx={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
								<Typography variant="subtitle2" color="text.secondary">
									Channel {index() + 1}
								</Typography>
								{channels().length > 1 && (
									<IconButton
										size="small"
										aria-label={`Remove channel ${index() + 1}`}
										onClick={() => removeChannelRow(channel.key)}
									>
										<DeleteIcon fontSize="small" />
									</IconButton>
								)}
							</Box>
							<TextField
								label="Chat id"
								type="number"
								variant="standard"
								value={channel.chatId}
								onChange={(_, v) => updateChannel(channel.key, { chatId: v, error: null })}
								helperText={
									channel.error ||
									'Get chat ID via @userinfobot or @getidsbot. Use the ID exactly as provided.'
								}
								error={typeof channel.error === 'string'}
								fullWidth
								required
							/>
							<TextField
								label="Name (optional)"
								variant="standard"
								value={channel.name}
								onChange={(_, v) => updateChannel(channel.key, { name: v })}
								helperText="Auto-detected from Telegram if left blank"
								fullWidth
							/>
						</Box>
					)}
				</For>

				<Button
					onClick={addChannelRow}
					variant="outlined"
					startIcon={<AddIcon />}
					disabled={channels().length >= MAX_CHANNELS}
				>
					Add another channel
				</Button>

				<Button type="submit" variant="contained" color="secondary">
					Register
				</Button>
			</Box>
		</Stack>
	)
}

export default StorageCreateForm
