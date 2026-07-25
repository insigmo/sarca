import { For, Show, createEffect, createSignal } from 'solid-js'
import Chip from '@suid/material/Chip'
import CircularProgress from '@suid/material/CircularProgress'
import IconButton from '@suid/material/IconButton'
import Typography from '@suid/material/Typography'
import ContentCopyIcon from '@suid/icons-material/ContentCopy'
import DeleteOutlineIcon from '@suid/icons-material/DeleteOutline'

import API from '../api'
import { alertStore } from './AlertStack'

const formatExpiry = (iso) => {
	if (!iso) return 'Never expires'
	const date = new Date(iso)
	return Number.isNaN(date.getTime())
		? String(iso)
		: `Expires ${date.toLocaleString()}`
}

const SharedLinksPanel = (props) => {
	const { addAlert } = alertStore
	const [links, setLinks] = createSignal([])
	const [loading, setLoading] = createSignal(false)
	const [revokingId, setRevokingId] = createSignal(null)

	const load = async () => {
		setLoading(true)
		try {
			setLinks(await API.shares.listShares(props.storageId))
		} catch {
			setLinks([])
		} finally {
			setLoading(false)
		}
	}

	createEffect(() => {
		if (!props.active) return
		props.storageId
		load()
	})

	const copyUrl = async (link) => {
		try {
			const url = API.shares.shareAbsoluteUrl(link.token, link.url_path)
			await navigator.clipboard.writeText(url)
			addAlert('Link copied', 'success')
		} catch {
			addAlert('Failed to copy link', 'error')
		}
	}

	const revoke = async (id) => {
		setRevokingId(id)
		try {
			await API.shares.revokeShare(props.storageId, id)
			addAlert('Share link revoked', 'success')
			await load()
		} catch {
			/* API helper reports the failure. */
		} finally {
			setRevokingId(null)
		}
	}

	return (
		<div class="shared-links-panel">
			<Show when={loading()}>
				<div class="shared-links-panel__loading">
					<CircularProgress size={28} />
				</div>
			</Show>
			<Show when={!loading() && !links().length}>
				<Typography color="text.secondary" sx={{ py: 4, textAlign: 'center' }}>
					No share links yet — open a file or folder menu to create one.
				</Typography>
			</Show>
			<ul class="shared-links-panel__list">
				<For each={links()}>
					{(link) => (
						<li class="shared-links-panel__row">
							<div class="shared-links-panel__meta">
								<p class="shared-links-panel__path">{link.path || '/'}</p>
								<p class="shared-links-panel__expiry">
									{formatExpiry(link.expires_at)}
								</p>
								<Show when={link.has_password}>
									<Chip size="small" label="Password" />
								</Show>
							</div>
							<div class="shared-links-panel__actions">
								<IconButton aria-label="Copy link" onClick={() => copyUrl(link)}>
									<ContentCopyIcon fontSize="small" />
								</IconButton>
								<IconButton
									aria-label="Revoke link"
									disabled={revokingId() === link.id}
									onClick={() => revoke(link.id)}
								>
									<DeleteOutlineIcon fontSize="small" />
								</IconButton>
							</div>
						</li>
					)}
				</For>
			</ul>
		</div>
	)
}

export default SharedLinksPanel
