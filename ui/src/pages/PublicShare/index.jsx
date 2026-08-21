import { useParams } from '@solidjs/router'
import {
	For,
	Show,
	createEffect,
	createMemo,
	createSignal,
	onCleanup,
} from 'solid-js'
import { Portal } from 'solid-js/web'
import Button from '@suid/material/Button'
import CircularProgress from '@suid/material/CircularProgress'
import CssBaseline from '@suid/material/CssBaseline'
import IconButton from '@suid/material/IconButton'
import Stack from '@suid/material/Stack'
import TextField from '@suid/material/TextField'
import Typography from '@suid/material/Typography'
import Breadcrumbs from '@suid/material/Breadcrumbs'
import Link from '@suid/material/Link'
import DownloadIcon from '@suid/icons-material/Download'
import FolderZipIcon from '@suid/icons-material/FolderZip'
import LockOutlinedIcon from '@suid/icons-material/LockOutlined'
import VisibilityIcon from '@suid/icons-material/Visibility'

import { t } from '../../common/i18n'
import API from '../../api'
import { convertSize } from '../../common/size_converter'
import { loadThumb } from '../../common/previewLoader'
import { clearThumbQueue } from '../../common/thumbQueue'
import FileTypeIcon from '../../components/FileTypeIcon'
import FileViewer from '../../components/FileViewer'
import FluentIcon from '../../components/FluentIcon'
import { alertStore } from '../../components/AlertStack'
import AppIcon from '../../components/AppIcon'
import LoadingDots from '../../components/LoadingDots'

const SHARE_VIEW_MODE_KEY = 'sarca.shareViewMode'

/** @returns {'list' | 'tiles'} */
const readStoredShareViewMode = () => {
	try {
		const v = localStorage.getItem(SHARE_VIEW_MODE_KEY)
		if (v === 'list' || v === 'tiles') return v
	} catch {
		/* ignore */
	}
	return 'list'
}

/**
 * Guest-facing public share page at `/s/:token`.
 * No app chrome / login — password unlock via HttpOnly cookie.
 */
const PublicShare = () => {
	const params = useParams()
	const { addAlert } = alertStore

	const token = () => decodeURIComponent(params.token || '')

	/** @type {[import('solid-js').Accessor<'loading'|'password'|'ready'|'missing'>, any]} */
	const [phase, setPhase] = createSignal('loading')
	/** @type {[import('solid-js').Accessor<import('../../api').PublicShareMeta | null>, any]} */
	const [meta, setMeta] = createSignal(null)
	const [password, setPassword] = createSignal('')
	const [unlocking, setUnlocking] = createSignal(false)
	const [unlockError, setUnlockError] = createSignal('')
	/** Relative browse path inside a folder share */
	const [browsePath, setBrowsePath] = createSignal('')
	/** @type {[import('solid-js').Accessor<import('../../api').FSElement[]>, any]} */
	const [children, setChildren] = createSignal([])
	const [treeLoading, setTreeLoading] = createSignal(false)
	/** Shared busy state for all three download paths below — only one guest
	 *  download realistically runs at a time, so one overlay covers all of them. */
	const [downloading, setDownloading] = createSignal(false)
	/** Whether the in-flight download is a folder ZIP — picks the right
	 *  indeterminate copy (a ZIP has no `Content-Length` until the server
	 *  finishes building it, so it never gets the determinate bar). */
	const [downloadIsZip, setDownloadIsZip] = createSignal(false)
	/** @type {[import('solid-js').Accessor<import('../../api').DownloadProgress | null>, any]} */
	const [downloadProgress, setDownloadProgress] = createSignal(null)
	/** @type {[import('solid-js').Accessor<'list' | 'tiles'>, any]} */
	const [viewMode, setViewMode] = createSignal(readStoredShareViewMode())
	/** @type {[import('solid-js').Accessor<import('../../api').FSElement | null>, any]} */
	const [viewerFile, setViewerFile] = createSignal(null)
	/** Thumb object URLs by path */
	const [thumbs, setThumbs] = createSignal(/** @type {Record<string, string>} */ ({}))

	const needsPassword = (err) =>
		err?.status === 401 &&
		(err?.body?.need_password === true ||
			String(err?.message || '').includes('need_password'))

	const loadMeta = async () => {
		setPhase('loading')
		setUnlockError('')
		try {
			const m = await API.publicShares.getPublicShare(token())
			setMeta(m)
			setPhase('ready')
			if (!m.is_file) {
				setBrowsePath('')
				await loadTree('')
			} else {
				setViewerFile({
					path: '',
					name: m.name,
					is_file: true,
					size: m.size || 0,
					has_thumb: false,
				})
			}
		} catch (err) {
			if (needsPassword(err)) {
				setMeta({
					path: '',
					name: t('publicShare.sharedItem'),
					is_file: true,
					has_password: true,
				})
				setPhase('password')
				return
			}
			console.error(err)
			setPhase('missing')
			addAlert(
				err.status === 404
					? t('publicShare.shareUnavailable')
					: err.message || t('publicShare.failedToOpenShare'),
				'error',
			)
		}
	}

	const loadTree = async (relPath) => {
		setTreeLoading(true)
		try {
			const layer = await API.publicShares.getPublicShareTree(
				token(),
				relPath,
			)
			setChildren(Array.isArray(layer) ? layer : [])
		} catch (err) {
			if (needsPassword(err)) {
				setPhase('password')
				return
			}
			console.error(err)
			setChildren([])
			addAlert(err.message || t('publicShare.failedToListFolder'), 'error')
		} finally {
			setTreeLoading(false)
		}
	}

	createEffect(() => {
		const t = token()
		if (!t) {
			setPhase('missing')
			return
		}
		loadMeta()
	})

	createEffect(() => {
		const list = children()
		const t = token()
		/** @type {string[]} */
		const created = []
		let cancelled = false
		const ac = new AbortController()

		for (const el of list) {
			if (!el.is_file || !el.has_thumb) continue
			const path = el.path
			loadThumb({
				scope: `share:${t}`,
				path,
				fetchBlob: (signal) => API.publicShares.thumbPublicShare(t, path, signal),
				signal: ac.signal,
			})
				.then((blob) => {
					if (cancelled) return
					const url = URL.createObjectURL(blob)
					created.push(url)
					setThumbs((prev) => ({ ...prev, [path]: url }))
				})
				.catch(() => {})
		}

		onCleanup(() => {
			cancelled = true
			ac.abort()
			for (const url of created) URL.revokeObjectURL(url)
			setThumbs({})
		})
	})

	const onUnlock = async (event) => {
		event?.preventDefault?.()
		if (unlocking() || !password().trim()) return
		setUnlocking(true)
		setUnlockError('')
		try {
			await API.publicShares.unlockPublicShare(token(), password().trim())
			setPassword('')
			await loadMeta()
		} catch (err) {
			console.error(err)
			setUnlockError(
				err.status === 401
					? t('publicShare.incorrectPassword')
					: err.message || t('publicShare.unlockFailed'),
			)
		} finally {
			setUnlocking(false)
		}
	}

	const crumbs = createMemo(() => {
		const p = browsePath()
		if (!p) return []
		const parts = p.split('/').filter(Boolean)
		/** @type {{ label: string, path: string }[]} */
		const out = []
		let acc = ''
		for (const part of parts) {
			acc = acc ? `${acc}/${part}` : part
			out.push({ label: part, path: acc })
		}
		return out
	})

	const goToRel = (rel) => {
		// Same reason the Files page does this on navigation: the thumbs of the
		// folder being left would otherwise sit on the browser's per-origin
		// connections while the new folder's listing waits behind them.
		clearThumbQueue()
		setBrowsePath(rel)
		loadTree(rel)
	}

	const setAndPersistViewMode = (mode) => {
		setViewMode(mode)
		try {
			localStorage.setItem(SHARE_VIEW_MODE_KEY, mode)
		} catch {
			/* ignore */
		}
	}

	const sizeLabel = (el) => {
		if (!el.is_file) return t('viewer.kindFolder')
		const size = Number(el.size)
		if (!Number.isFinite(size) || size < 0) return '—'
		return convertSize(size)
	}

	const mtimeLabel = (el) => {
		const raw = el.mtime
		if (raw == null || raw === '') return ''
		const d = new Date(typeof raw === 'number' && raw < 1e12 ? raw * 1000 : raw)
		if (Number.isNaN(d.getTime())) return ''
		return d.toLocaleString(undefined, {
			year: 'numeric',
			month: 'short',
			day: 'numeric',
			hour: '2-digit',
			minute: '2-digit',
		})
	}

	const openChild = (el) => {
		if (!el.is_file) {
			const next = String(el.path || '').replace(/\/$/, '')
			goToRel(next)
			return
		}
		setViewerFile(el)
	}

	const downloadChild = async (el) => {
		if (downloading()) return
		const isFile = el.is_file
		setDownloading(true)
		setDownloadIsZip(!isFile)
		try {
			const path = isFile
				? el.path
				: el.path.endsWith('/')
					? el.path
					: `${el.path}/`
			const blob = await API.publicShares.downloadPublicShareWithProgress(
				token(),
				path,
				setDownloadProgress,
			)
			const href = URL.createObjectURL(blob)
			const a = Object.assign(document.createElement('a'), {
				href,
				download: isFile ? el.name : `${el.name}.zip`,
				style: 'display: none',
			})
			document.body.appendChild(a)
			a.click()
			URL.revokeObjectURL(href)
			a.remove()
			addAlert(t('publicShare.downloadStarted'), 'success')
		} catch (err) {
			console.error(err)
			addAlert(err.message || t('publicShare.downloadFailed'), 'error')
		} finally {
			setDownloading(false)
			setDownloadProgress(null)
		}
	}

	const downloadZip = async () => {
		if (downloading()) return
		setDownloading(true)
		setDownloadIsZip(true)
		try {
			const blob = await API.publicShares.downloadPublicShareZipWithProgress(
				token(),
				setDownloadProgress,
			)
			const href = URL.createObjectURL(blob)
			const name = meta()?.name || 'shared'
			const a = Object.assign(document.createElement('a'), {
				href,
				download: `${name}.zip`,
				style: 'display: none',
			})
			document.body.appendChild(a)
			a.click()
			URL.revokeObjectURL(href)
			a.remove()
			addAlert(t('publicShare.zipReady'), 'success')
		} catch (err) {
			console.error(err)
			addAlert(err.message || t('publicShare.zipDownloadFailed'), 'error')
		} finally {
			setDownloading(false)
			setDownloadProgress(null)
		}
	}

	const downloadSharedFile = async () => {
		if (downloading()) return
		setDownloading(true)
		setDownloadIsZip(false)
		try {
			const blob = await API.publicShares.downloadPublicShareWithProgress(
				token(),
				'',
				setDownloadProgress,
			)
			const href = URL.createObjectURL(blob)
			const a = Object.assign(document.createElement('a'), {
				href,
				download: meta()?.name || 'download',
				style: 'display: none',
			})
			document.body.appendChild(a)
			a.click()
			URL.revokeObjectURL(href)
			a.remove()
			addAlert(t('publicShare.downloadStarted'), 'success')
		} catch (err) {
			console.error(err)
			addAlert(err.message || t('publicShare.downloadFailed'), 'error')
		} finally {
			setDownloading(false)
			setDownloadProgress(null)
		}
	}

	/** File element for single-file share preview */
	const fileAsElement = createMemo(() => {
		const m = meta()
		if (!m || !m.is_file) return null
		return {
			path: '',
			name: m.name,
			is_file: true,
			size: m.size || 0,
			has_thumb: false,
		}
	})

	const resolveInlineUrl = (path) =>
		API.publicShares.getPublicInlineMediaUrl(token(), path || '')

	const resolvePreviewUrl = (path) =>
		API.publicShares.getPublicPreviewUrl(token(), path || '')

	const resolveDownload = (path, onProgress) =>
		API.publicShares.downloadPublicShareWithProgress(
			token(),
			path || '',
			onProgress,
		)

	return (
		<>
			<CssBaseline />
			<div class="public-share">
				<header class="public-share__header glass-panel">
					<Stack direction="row" alignItems="center" gap={1.5}>
						<AppIcon size={36} />
						<div>
							<Typography
								variant="h6"
								sx={{ fontFamily: 'var(--sarca-display)', lineHeight: 1.2 }}
							>
								Sarca
							</Typography>
							<Typography variant="caption" color="text.secondary">
								{t('publicShare.sharedWithYou')}
							</Typography>
						</div>
					</Stack>
				</header>

				<main class="public-share__main">
					<Show when={phase() === 'loading'}>
						<div class="public-share__center">
							<CircularProgress color="secondary" />
						</div>
					</Show>

					<Show when={phase() === 'missing'}>
						<div class="public-share__center glass-panel public-share__card">
							<Typography variant="h5" gutterBottom>
								{t('publicShare.linkUnavailable')}
							</Typography>
							<Typography color="text.secondary">
								{t('publicShare.linkUnavailableDetail')}
							</Typography>
						</div>
					</Show>

					<Show when={phase() === 'password'}>
						<div class="public-share__center">
							<form
								class="glass-panel public-share__card"
								onSubmit={onUnlock}
							>
								<Stack alignItems="center" gap={1} sx={{ mb: 2 }}>
									<LockOutlinedIcon color="secondary" fontSize="large" />
									<Typography variant="h5">{t('publicShare.passwordRequired')}</Typography>
									<Typography variant="body2" color="text.secondary">
										{t('publicShare.passwordPrompt')}
									</Typography>
								</Stack>
								<TextField
									type="password"
									label={t('publicShare.passwordLabel')}
									value={password()}
									onChange={(e) => setPassword(e.currentTarget.value)}
									autoFocus
									error={Boolean(unlockError())}
									helperText={unlockError()}
									sx={{ mb: 2 }}
								/>
								<Button
									type="submit"
									variant="contained"
									color="secondary"
									fullWidth
									disabled={unlocking() || !password().trim()}
								>
									{unlocking() ? t('publicShare.unlocking') : t('publicShare.unlock')}
								</Button>
							</form>
						</div>
					</Show>

					<Show when={phase() === 'ready' && meta()?.is_file}>
						<div class="glass-panel public-share__card public-share__file">
							<Stack
								direction={{ xs: 'column', sm: 'row' }}
								alignItems={{ sm: 'center' }}
								justifyContent="space-between"
								gap={2}
								sx={{ mb: 2 }}
							>
								<div>
									<Typography variant="h5">{meta()?.name}</Typography>
									<Show when={meta()?.size != null}>
										<Typography variant="body2" color="text.secondary">
											{convertSize(meta()?.size || 0)}
										</Typography>
									</Show>
								</div>
								<Stack direction="row" gap={1} flexWrap="wrap">
									<Button
										variant="outlined"
										color="secondary"
										startIcon={<VisibilityIcon />}
										onClick={() => setViewerFile(fileAsElement())}
									>
										{t('publicShare.preview')}
									</Button>
									<Button
										variant="contained"
										color="secondary"
										startIcon={<DownloadIcon />}
										disabled={downloading()}
										onClick={downloadSharedFile}
									>
										{t('publicShare.download')}
									</Button>
								</Stack>
							</Stack>
							<Typography variant="body2" color="text.secondary">
								{t('publicShare.previewHint')}
							</Typography>
						</div>
					</Show>

					<Show when={phase() === 'ready' && meta() && !meta().is_file}>
						<div class="glass-panel public-share__folder">
							<Stack
								direction={{ xs: 'column', sm: 'row' }}
								alignItems={{ sm: 'center' }}
								justifyContent="space-between"
								gap={2}
								sx={{ mb: 2 }}
							>
								<div>
									<Typography variant="h5">{meta()?.name}</Typography>
									<Breadcrumbs sx={{ mt: 0.5 }}>
										<Link
											component="button"
											underline="hover"
											color="inherit"
											onClick={() => goToRel('')}
										>
											{t('publicShare.shareRoot')}
										</Link>
										<For each={crumbs()}>
											{(c) => (
												<Link
													component="button"
													underline="hover"
													color="inherit"
													onClick={() => goToRel(c.path)}
												>
													{c.label}
												</Link>
											)}
										</For>
									</Breadcrumbs>
								</div>
								<Stack direction="row" alignItems="center" gap={1} flexWrap="wrap">
									<div
										class="files-view-toggle"
										role="group"
										aria-label={t('files.viewModeGroup')}
									>
										<IconButton
											size="small"
											class="files-view-toggle__btn"
											classList={{
												'files-view-toggle__btn--active':
													viewMode() === 'list',
											}}
											aria-label={t('files.listView')}
											aria-pressed={viewMode() === 'list'}
											onClick={() => setAndPersistViewMode('list')}
										>
											<FluentIcon
												name={viewMode() === 'list' ? 'listFilled' : 'list'}
												size={18}
											/>
										</IconButton>
										<IconButton
											size="small"
											class="files-view-toggle__btn"
											classList={{
												'files-view-toggle__btn--active':
													viewMode() === 'tiles',
											}}
											aria-label={t('files.tilesView')}
											aria-pressed={viewMode() === 'tiles'}
											onClick={() => setAndPersistViewMode('tiles')}
										>
											<FluentIcon
												name={viewMode() === 'tiles' ? 'gridFilled' : 'grid'}
												size={18}
											/>
										</IconButton>
									</div>
									<Button
										variant="contained"
										color="secondary"
										startIcon={<FolderZipIcon />}
										disabled={downloading()}
										onClick={downloadZip}
									>
										{downloading() && downloadIsZip()
											? t('publicShare.preparingZip')
											: t('publicShare.downloadZip')}
									</Button>
								</Stack>
							</Stack>

							<Show when={treeLoading()}>
								<div class="public-share__center" style={{ padding: '32px' }}>
									<CircularProgress size={32} color="secondary" />
								</div>
							</Show>

							<Show when={!treeLoading()}>
								<div
									class="files-canvas"
									classList={{ 'files-canvas--list': viewMode() === 'list' }}
									style={{ 'min-height': '200px' }}
								>
									<Show
										when={children().length}
										fallback={
											<div
												class="files-canvas__empty"
												onMouseDown={(e) => e.preventDefault()}
											>
												{t('publicShare.folderEmpty')}
											</div>
										}
									>
										<Show
											when={viewMode() === 'list'}
											fallback={
												<div class="files-grid">
													<For each={children()}>
														{(el) => (
															<div
																class="fs-grid-item"
																role="button"
																tabIndex={0}
																onClick={() => openChild(el)}
																onKeyDown={(e) => {
																	if (e.key === 'Enter' || e.key === ' ') {
																		e.preventDefault()
																		openChild(el)
																	}
																}}
															>
																<div class="fs-grid-item__more">
																	<Button
																		size="small"
																		aria-label={t('publicShare.download')}
																		onClick={(e) => {
																			e.stopPropagation()
																			downloadChild(el)
																		}}
																		sx={{ minWidth: 0, p: 0.5 }}
																	>
																		<DownloadIcon fontSize="small" />
																	</Button>
																</div>
																<FileTypeIcon
																	name={el.name}
																	isFile={el.is_file}
																	thumbUrl={thumbs()[el.path]}
																	size={64}
																/>
																<div
																	class="fs-grid-item__name"
																	title={el.name}
																>
																	{el.name}
																</div>
															</div>
														)}
													</For>
												</div>
											}
										>
											<div class="files-list">
												<For each={children()}>
													{(el) => (
														<div
															class="fs-list-item"
															role="button"
															tabIndex={0}
															onClick={() => openChild(el)}
															onKeyDown={(e) => {
																if (e.key === 'Enter' || e.key === ' ') {
																	e.preventDefault()
																	openChild(el)
																}
															}}
														>
															<FileTypeIcon
																name={el.name}
																isFile={el.is_file}
																thumbUrl={thumbs()[el.path]}
																size={40}
															/>
															<div class="fs-list-item__body">
																<div class="fs-list-item__name" title={el.name}>
																	{el.name}
																</div>
																<div class="fs-list-item__meta">
																	<span>{sizeLabel(el)}</span>
																	<Show when={mtimeLabel(el)}>
																		<span class="fs-list-item__dot">·</span>
																		<span>{mtimeLabel(el)}</span>
																	</Show>
																</div>
															</div>
															<div class="fs-list-item__actions">
																<IconButton
																	size="small"
																	aria-label={t('publicShare.download')}
																	title={t('publicShare.download')}
																	onClick={(e) => {
																		e.stopPropagation()
																		downloadChild(el)
																	}}
																>
																	<FluentIcon name="arrowDownload" size={18} />
																</IconButton>
															</div>
														</div>
													)}
												</For>
											</div>
										</Show>
									</Show>
								</div>
							</Show>
						</div>
					</Show>
				</main>
			</div>

			<FileViewer
				open={Boolean(viewerFile())}
				file={viewerFile()}
				files={
					meta()?.is_file
						? viewerFile()
							? [viewerFile()]
							: []
						: children()
				}
				storageId=""
				resolveInlineUrl={resolveInlineUrl}
				resolvePreviewUrl={resolvePreviewUrl}
				resolveDownload={resolveDownload}
				onClose={() => setViewerFile(null)}
				onNavigate={(file) => setViewerFile(file)}
			/>

			<Show when={downloading()}>
				<Portal mount={document.body}>
					<div class="download-preparing" role="status" aria-live="polite">
						<Show
							when={downloadProgress()?.total}
							fallback={
								<>
									<div class="download-preparing__text">
										{downloadIsZip()
											? t('publicShare.preparingZipArchive')
											: t('publicShare.preparingDownload')}
										<LoadingDots />
									</div>
									<Show when={downloadIsZip()}>
										<div class="download-preparing__hint">
											{t('publicShare.zipHint')}
										</div>
									</Show>
								</>
							}
						>
							<div class="download-preparing__text">
								{t('publicShare.downloadingProgress', {
									percent: Math.round(downloadProgress().percent),
								})}
							</div>
							<div class="download-preparing__bar">
								<div
									class="download-preparing__bar-fill"
									style={{ transform: `scaleX(${downloadProgress().percent / 100})` }}
								/>
							</div>
						</Show>
					</div>
				</Portal>
			</Show>
		</>
	)
}

export default PublicShare
