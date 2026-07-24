import Button from '@suid/material/Button'
import Dialog from '@suid/material/Dialog'
import DialogActions from '@suid/material/DialogActions'
import DialogContent from '@suid/material/DialogContent'
import DialogTitle from '@suid/material/DialogTitle'
import TextField from '@suid/material/TextField'
import Typography from '@suid/material/Typography'
import Stack from '@suid/material/Stack'
import Chip from '@suid/material/Chip'
import IconButton from '@suid/material/IconButton'
import CircularProgress from '@suid/material/CircularProgress'
import Divider from '@suid/material/Divider'
import ContentCopyIcon from '@suid/icons-material/ContentCopy'
import LinkIcon from '@suid/icons-material/Link'
import DeleteOutlineIcon from '@suid/icons-material/DeleteOutline'
import { For, Show, createEffect, createSignal } from 'solid-js'

import API from '../api'
import { alertStore } from './AlertStack'

/** @typedef {'1d' | '7d' | '30d' | 'never' | 'custom'} ExpiryPreset */

/**
 * @typedef {Object} ShareLinkDialogProps
 * @property {boolean} isOpened
 * @property {string} storageId
 * @property {string} path Normalized path (folders end with /)
 * @property {string} [itemName]
 * @property {boolean} [isFile]
 * @property {() => void} onClose
 */

/**
 * @param {ExpiryPreset} preset
 * @param {string} customLocal
 * @returns {string|null|undefined} ISO expires_at, null = never, undefined = invalid custom
 */
const resolveExpiresAt = (preset, customLocal) => {
	if (preset === 'never') return null
	if (preset === 'custom') {
		if (!customLocal) return undefined
		const d = new Date(customLocal)
		if (Number.isNaN(d.getTime())) return undefined
		return d.toISOString()
	}
	const d = new Date()
	if (preset === '1d') d.setDate(d.getDate() + 1)
	else if (preset === '7d') d.setDate(d.getDate() + 7)
	else if (preset === '30d') d.setDate(d.getDate() + 30)
	return d.toISOString()
}

/**
 * @param {string|null|undefined} iso
 */
const formatExpiry = (iso) => {
	if (!iso) return 'Never expires'
	try {
		return `Expires ${new Date(iso).toLocaleString()}`
	} catch {
		return String(iso)
	}
}

/**
 * Create / list / revoke public share links for a path.
 * @param {ShareLinkDialogProps} props
 */
const ShareLinkDialog = (props) => {
	const { addAlert } = alertStore
	/** @type {[import('solid-js').Accessor<import('../api').ShareLink[]>, any]} */
	const [links, setLinks] = createSignal([])
	const [loading, setLoading] = createSignal(false)
	const [creating, setCreating] = createSignal(false)
	/** @type {[import('solid-js').Accessor<ExpiryPreset>, any]} */
	const [preset, setPreset] = createSignal('7d')
	const [customLocal, setCustomLocal] = createSignal('')
	const [password, setPassword] = createSignal('')
	const [createdUrl, setCreatedUrl] = createSignal('')
	const [revokingId, setRevokingId] = createSignal(null)

	const pathForApi = () => {
		const p = String(props.path || '')
		if (props.isFile) return p
		if (!p) return ''
		return p.endsWith('/') ? p : `${p}/`
	}

	const loadLinks = async () => {
		setLoading(true)
		try {
			const all = await API.shares.listShares(props.storageId)
			const target = pathForApi()
			setLinks(
				(all || []).filter((l) => {
					const lp = String(l.path || '')
					const norm = lp.endsWith('/') || !target.endsWith('/') ? lp : `${lp}/`
					return norm === target || lp === target
				}),
			)
		} catch {
			setLinks([])
		} finally {
			setLoading(false)
		}
	}

	createEffect(() => {
		if (!props.isOpened) return
		setPreset('7d')
		setCustomLocal('')
		setPassword('')
		setCreatedUrl('')
		setRevokingId(null)
		loadLinks()
	})

	const onClose = () => {
		if (creating() || revokingId()) return
		props.onClose()
	}

	const copyText = async (text) => {
		try {
			await navigator.clipboard.writeText(text)
			addAlert('Link copied', 'success')
		} catch {
			addAlert('Could not copy — select the URL manually', 'error')
		}
	}

	const onCreate = async (event) => {
		event?.preventDefault?.()
		if (creating()) return
		const expires_at = resolveExpiresAt(preset(), customLocal())
		if (expires_at === undefined) {
			addAlert('Choose a valid expiry date', 'error')
			return
		}
		setCreating(true)
		try {
			const body = { path: pathForApi() }
			if (expires_at !== null) body.expires_at = expires_at
			const pw = password().trim()
			if (pw) body.password = pw
			const created = await API.shares.createShare(props.storageId, body)
			const url = API.shares.shareAbsoluteUrl(created.token, created.url_path)
			setCreatedUrl(url)
			setPassword('')
			addAlert('Share link created', 'success')
			await copyText(url)
			await loadLinks()
		} catch (err) {
			console.error(err)
		} finally {
			setCreating(false)
		}
	}

	const onRevoke = async (id) => {
		if (revokingId()) return
		setRevokingId(id)
		try {
			await API.shares.revokeShare(props.storageId, id)
			addAlert('Share link revoked', 'success')
			if (createdUrl() && links().some((l) => l.id === id)) {
				const gone = links().find((l) => l.id === id)
				if (
					gone &&
					createdUrl().includes(gone.token)
				) {
					setCreatedUrl('')
				}
			}
			await loadLinks()
		} catch (err) {
			console.error(err)
		} finally {
			setRevokingId(null)
		}
	}

	const presets = /** @type {{ id: ExpiryPreset, label: string }[]} */ ([
		{ id: '1d', label: '1 day' },
		{ id: '7d', label: '7 days' },
		{ id: '30d', label: '30 days' },
		{ id: 'never', label: 'Never' },
		{ id: 'custom', label: 'Custom' },
	])

	return (
		<Dialog open={props.isOpened} onClose={onClose} fullWidth maxWidth="sm">
			<form onSubmit={onCreate}>
				<DialogTitle sx={{ display: 'flex', alignItems: 'center', gap: 1 }}>
					<LinkIcon fontSize="small" color="secondary" />
					Share link
				</DialogTitle>
				<DialogContent>
					<Show when={props.itemName}>
						<Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
							{props.isFile ? 'File' : 'Folder'}: {props.itemName}
						</Typography>
					</Show>

					<Typography variant="subtitle2" sx={{ mb: 1 }}>
						Expires
					</Typography>
					<Stack direction="row" flexWrap="wrap" gap={1} sx={{ mb: 2 }}>
						<For each={presets}>
							{(p) => (
								<Chip
									label={p.label}
									color={preset() === p.id ? 'secondary' : 'default'}
									variant={preset() === p.id ? 'filled' : 'outlined'}
									onClick={() => setPreset(p.id)}
									clickable
								/>
							)}
						</For>
					</Stack>

					<Show when={preset() === 'custom'}>
						<TextField
							type="datetime-local"
							label="Expiry"
							margin="dense"
							value={customLocal()}
							onChange={(e) => setCustomLocal(e.currentTarget.value)}
							InputLabelProps={{ shrink: true }}
							sx={{ mb: 2 }}
						/>
					</Show>

					<TextField
						type="password"
						label="Password (optional)"
						margin="dense"
						value={password()}
						onChange={(e) => setPassword(e.currentTarget.value)}
						autoComplete="new-password"
						helperText="Leave blank for an open link"
						sx={{ mb: 2 }}
					/>

					<Show when={createdUrl()}>
						<Stack
							direction="row"
							alignItems="center"
							gap={1}
							sx={{
								mb: 2,
								p: 1.5,
								borderRadius: 2,
								bgcolor: 'action.hover',
							}}
						>
							<Typography
								variant="body2"
								sx={{
									flex: 1,
									fontFamily: 'monospace',
									wordBreak: 'break-all',
								}}
							>
								{createdUrl()}
							</Typography>
							<IconButton
								size="small"
								aria-label="Copy link"
								onClick={() => copyText(createdUrl())}
							>
								<ContentCopyIcon fontSize="small" />
							</IconButton>
						</Stack>
					</Show>

					<Divider sx={{ my: 2 }} />

					<Typography variant="subtitle2" sx={{ mb: 1 }}>
						Existing links for this path
					</Typography>

					<Show when={loading()}>
						<div
							style={{
								display: 'grid',
								'place-items': 'center',
								padding: '16px',
							}}
						>
							<CircularProgress size={28} color="secondary" />
						</div>
					</Show>

					<Show when={!loading() && !links().length}>
						<Typography variant="body2" color="text.secondary">
							No active links yet.
						</Typography>
					</Show>

					<Show when={!loading() && links().length}>
						<Stack gap={1.5}>
							<For each={links()}>
								{(link) => (
									<Stack
										direction="row"
										alignItems="flex-start"
										gap={1}
										sx={{
											p: 1.25,
											borderRadius: 2,
											border: '1px solid',
											borderColor: 'divider',
										}}
									>
										<Stack sx={{ flex: 1, minWidth: 0 }} gap={0.5}>
											<Typography
												variant="body2"
												sx={{
													fontFamily: 'monospace',
													wordBreak: 'break-all',
												}}
											>
												{API.shares.shareAbsoluteUrl(
													link.token,
													link.url_path,
												)}
											</Typography>
											<Typography variant="caption" color="text.secondary">
												{formatExpiry(link.expires_at)}
												{link.has_password ? ' · Password protected' : ''}
											</Typography>
										</Stack>
										<IconButton
											size="small"
											aria-label="Copy link"
											onClick={() =>
												copyText(
													API.shares.shareAbsoluteUrl(
														link.token,
														link.url_path,
													),
												)
											}
										>
											<ContentCopyIcon fontSize="small" />
										</IconButton>
										<IconButton
											size="small"
											aria-label="Revoke link"
											color="error"
											disabled={revokingId() === link.id}
											onClick={() => onRevoke(link.id)}
										>
											<DeleteOutlineIcon fontSize="small" />
										</IconButton>
									</Stack>
								)}
							</For>
						</Stack>
					</Show>
				</DialogContent>
				<DialogActions>
					<Button
						type="submit"
						color="secondary"
						disabled={creating() || loading()}
					>
						{creating() ? 'Creating…' : 'Create'}
					</Button>
					<Button onClick={onClose} color="info" disabled={creating()}>
						Close
					</Button>
				</DialogActions>
			</form>
		</Dialog>
	)
}

export default ShareLinkDialog
