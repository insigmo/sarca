import { For, Show, createEffect, createSignal, onCleanup } from 'solid-js'
import Chip from '@suid/material/Chip'
import CircularProgress from '@suid/material/CircularProgress'
import IconButton from '@suid/material/IconButton'
import Typography from '@suid/material/Typography'

import API from '../api'
import { t } from '../common/i18n'
import { copyToClipboard } from '../utils/clipboard'
import { alertStore } from './AlertStack'
import FluentIcon from './FluentIcon'

const formatExpiry = (iso) => {
	if (!iso) return t('misc.sharedLinks.neverExpires')
	const date = new Date(iso)
	return Number.isNaN(date.getTime())
		? String(iso)
		: t('misc.sharedLinks.expires', { date: date.toLocaleString() })
}

/** Active shares only (API may still return soft-revoked rows on older servers). */
const activeShares = (list) =>
	(Array.isArray(list) ? list : []).filter((link) => !link.revoked_at)

const SharedLinksPanel = (props) => {
	const { addAlert } = alertStore
	const [links, setLinks] = createSignal([])
	const [loading, setLoading] = createSignal(false)
	const [revokingId, setRevokingId] = createSignal(null)
	let loadGen = 0

	const load = async () => {
		const gen = ++loadGen
		setLoading(true)
		try {
			const data = await API.shares.listShares(props.storageId)
			if (gen !== loadGen) return
			setLinks(activeShares(data))
		} catch {
			if (gen !== loadGen) return
			setLinks([])
		} finally {
			if (gen === loadGen) setLoading(false)
		}
	}

	createEffect(() => {
		if (!props.active) return
		props.storageId
		load()
	})

	onCleanup(() => {
		loadGen += 1
	})

	const copyUrl = async (link) => {
		const url = API.shares.shareAbsoluteUrl(link.token, link.url_path)
		const ok = await copyToClipboard(url)
		if (ok) addAlert(t('misc.sharedLinks.linkCopied'), 'success')
		else addAlert(t('misc.sharedLinks.copyFailed'), 'error')
	}

	const revoke = async (link) => {
		const id = link.id
		if (revokingId()) return
		setRevokingId(id)
		try {
			await API.shares.revokeShare(props.storageId, id)
			addAlert(t('misc.sharedLinks.linkRevoked'), 'success')
			// Drop immediately; bump gen so an in-flight list cannot restore it.
			loadGen += 1
			setLinks((prev) =>
				prev.filter((item) => String(item.id) !== String(id)),
			)
			setLoading(false)
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
					{t('misc.sharedLinks.empty')}
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
									<Chip size="small" label={t('misc.sharedLinks.password')} />
								</Show>
							</div>
							<div class="shared-links-panel__actions">
								<IconButton aria-label={t('misc.sharedLinks.copyLink')} onClick={() => copyUrl(link)}>
									<FluentIcon name="copy" size={18} />
								</IconButton>
								<IconButton
									aria-label={t('misc.sharedLinks.revokeLink')}
									disabled={revokingId() === link.id}
									onClick={() => revoke(link)}
								>
									<FluentIcon name="delete" size={18} />
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
