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
import Stack from '@suid/material/Stack'
import TextField from '@suid/material/TextField'
import Typography from '@suid/material/Typography'
import Breadcrumbs from '@suid/material/Breadcrumbs'
import Link from '@suid/material/Link'
import DownloadIcon from '@suid/icons-material/Download'
import FolderZipIcon from '@suid/icons-material/FolderZip'
import LockOutlinedIcon from '@suid/icons-material/LockOutlined'
import VisibilityIcon from '@suid/icons-material/Visibility'

import API from '../../api'
import { fileBaseName, fileExtensionLabel } from '../../common/fileLabel'
import { convertSize } from '../../common/size_converter'
import FileTypeIcon from '../../components/FileTypeIcon'
import FileViewer from '../../components/FileViewer'
import { alertStore } from '../../components/AlertStack'
import AppIcon from '../../components/AppIcon'

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
	const [zipDownloading, setZipDownloading] = createSignal(false)
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
					name: 'Shared item',
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
					? 'This share link is unavailable'
					: err.message || 'Failed to open share',
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
			addAlert(err.message || 'Failed to list folder', 'error')
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

		for (const el of list) {
			if (!el.is_file || !el.has_thumb) continue
			const path = el.path
			API.publicShares
				.thumbPublicShare(t, path)
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
					? 'Incorrect password'
					: err.message || 'Unlock failed',
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
		setBrowsePath(rel)
		loadTree(rel)
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
		try {
			const isFile = el.is_file
			const path = isFile
				? el.path
				: el.path.endsWith('/')
					? el.path
					: `${el.path}/`
			const blob = await API.publicShares.downloadPublicShare(token(), path)
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
			addAlert('Download started', 'success')
		} catch (err) {
			console.error(err)
			addAlert(err.message || 'Download failed', 'error')
		}
	}

	const downloadZip = async () => {
		if (zipDownloading()) return
		setZipDownloading(true)
		try {
			const blob = await API.publicShares.downloadPublicShareZip(token())
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
			addAlert('ZIP ready — download started', 'success')
		} catch (err) {
			console.error(err)
			addAlert(err.message || 'ZIP download failed', 'error')
		} finally {
			setZipDownloading(false)
		}
	}

	const downloadSharedFile = async () => {
		try {
			const blob = await API.publicShares.downloadPublicShare(token(), '')
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
			addAlert('Download started', 'success')
		} catch (err) {
			console.error(err)
			addAlert(err.message || 'Download failed', 'error')
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

	const resolveDownload = (path) =>
		API.publicShares.downloadPublicShare(token(), path || '')

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
								Shared with you
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
								Link unavailable
							</Typography>
							<Typography color="text.secondary">
								This share may have expired, been revoked, or never existed.
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
									<Typography variant="h5">Password required</Typography>
									<Typography variant="body2" color="text.secondary">
										Enter the password to open this share.
									</Typography>
								</Stack>
								<TextField
									type="password"
									label="Password"
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
									{unlocking() ? 'Unlocking…' : 'Unlock'}
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
										Preview
									</Button>
									<Button
										variant="contained"
										color="secondary"
										startIcon={<DownloadIcon />}
										onClick={downloadSharedFile}
									>
										Download
									</Button>
								</Stack>
							</Stack>
							<Typography variant="body2" color="text.secondary">
								Use Preview to open images, video, PDF, and text in the browser.
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
											Share root
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
								<Button
									variant="contained"
									color="secondary"
									startIcon={<FolderZipIcon />}
									disabled={zipDownloading()}
									onClick={downloadZip}
								>
									{zipDownloading() ? 'Preparing ZIP…' : 'Download ZIP'}
								</Button>
							</Stack>

							<Show when={treeLoading()}>
								<div class="public-share__center" style={{ padding: '32px' }}>
									<CircularProgress size={32} color="secondary" />
								</div>
							</Show>

							<Show when={!treeLoading()}>
								<div class="files-canvas" style={{ 'min-height': '200px' }}>
									<Show
										when={children().length}
										fallback={
											<div class="files-canvas__empty">This folder is empty</div>
										}
									>
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
																aria-label="Download"
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
															{fileBaseName(el.name, el.is_file)}
														</div>
														<div class="fs-grid-item__ext">
															{fileExtensionLabel(el.name, el.is_file)}
														</div>
													</div>
												)}
											</For>
										</div>
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
				resolveDownload={resolveDownload}
				onClose={() => setViewerFile(null)}
				onNavigate={(file) => setViewerFile(file)}
			/>

			<Show when={zipDownloading()}>
				<Portal mount={document.body}>
					<div class="download-preparing" role="status" aria-live="polite">
						<CircularProgress color="secondary" size={42} />
						<div class="download-preparing__text">Preparing ZIP archive…</div>
						<div class="download-preparing__hint">
							This may take a while for large folders
						</div>
					</div>
				</Portal>
			</Show>
		</>
	)
}

export default PublicShare
