import { Show, createEffect, createMemo, createSignal, onCleanup } from 'solid-js'
import { Portal } from 'solid-js/web'
import Button from '@suid/material/Button'
import CircularProgress from '@suid/material/CircularProgress'
import CloseIcon from '@suid/icons-material/Close'
import DownloadIcon from '@suid/icons-material/Download'
import PlayArrowIcon from '@suid/icons-material/PlayArrow'
import PauseIcon from '@suid/icons-material/Pause'
import VolumeUpIcon from '@suid/icons-material/VolumeUp'
import VolumeOffIcon from '@suid/icons-material/VolumeOff'
import VolumeDownIcon from '@suid/icons-material/VolumeDown'
import FullscreenIcon from '@suid/icons-material/Fullscreen'
import FullscreenExitIcon from '@suid/icons-material/FullscreenExit'
import ChevronLeftIcon from '@suid/icons-material/ChevronLeft'
import ChevronRightIcon from '@suid/icons-material/ChevronRight'

import API from '../api'
import { fileKind } from '../common/fileKind'
import { convertSize } from '../common/size_converter'
import FileTypeIcon from './FileTypeIcon'
import { alertStore } from './AlertStack'

const formatTime = (sec) => {
	if (!Number.isFinite(sec) || sec < 0) return '0:00'
	const m = Math.floor(sec / 60)
	const s = Math.floor(sec % 60)
	return `${m}:${String(s).padStart(2, '0')}`
}

/**
 * @typedef {Object} FileViewerProps
 * @property {boolean} open
 * @property {import("../api").FSElement | null} file
 * @property {import("../api").FSElement[]} [files]
 * @property {string} storageId
 * @property {() => void} onClose
 * @property {(file: import("../api").FSElement) => void} [onNavigate]
 */

/**
 * Fullscreen file preview / media player.
 * @param {FileViewerProps} props
 */
const FileViewer = (props) => {
	const { addAlert } = alertStore
	const [loading, setLoading] = createSignal(false)
	const [error, setError] = createSignal(null)
	const [textContent, setTextContent] = createSignal('')
	const [docxHtml, setDocxHtml] = createSignal('')
	const [markdownHtml, setMarkdownHtml] = createSignal('')
	const [htmlDoc, setHtmlDoc] = createSignal('')
	const [mediaUrl, setMediaUrl] = createSignal('')
	const [officeMode, setOfficeMode] = createSignal(false)

	const [playing, setPlaying] = createSignal(false)
	const [muted, setMuted] = createSignal(false)
	const [volume, setVolume] = createSignal(1)
	const [currentTime, setCurrentTime] = createSignal(0)
	const [duration, setDuration] = createSignal(0)
	const [progress, setProgress] = createSignal(0)
	const [chromeVisible, setChromeVisible] = createSignal(true)
	const [isFullscreen, setIsFullscreen] = createSignal(false)

	/** @type {HTMLVideoElement | HTMLAudioElement | undefined} */
	let mediaEl
	/** @type {HTMLElement | undefined} */
	let viewerEl
	/** @type {ReturnType<typeof setTimeout> | null} */
	let hideChromeTimer = null
	let chromePinned = false

	const kind = () =>
		props.file ? fileKind(props.file.name, props.file.is_file) : 'generic'

	/** Document-like previews where side nav would cover the text. */
	const isDocNavKind = () => kind() === 'markdown' || kind() === 'html'

	const streamKinds = () =>
		['image', 'video', 'audio', 'pdf'].includes(kind())

	const viewableFiles = createMemo(() =>
		(props.files || []).filter((f) => f.is_file && f.name !== '..'),
	)

	const currentIndex = createMemo(() => {
		const file = props.file
		if (!file) return -1
		return viewableFiles().findIndex((f) => f.path === file.path)
	})

	const hasPrev = () => currentIndex() > 0
	const hasNext = () => {
		const i = currentIndex()
		return i >= 0 && i < viewableFiles().length - 1
	}

	const goPrev = () => {
		if (!hasPrev() || !props.onNavigate) return
		props.onNavigate(viewableFiles()[currentIndex() - 1])
	}

	const goNext = () => {
		if (!hasNext() || !props.onNavigate) return
		props.onNavigate(viewableFiles()[currentIndex() + 1])
	}

	const clearHideChromeTimer = () => {
		if (hideChromeTimer != null) {
			clearTimeout(hideChromeTimer)
			hideChromeTimer = null
		}
	}

	const scheduleHideChrome = () => {
		clearHideChromeTimer()
		if (kind() !== 'video' || chromePinned) return
		hideChromeTimer = setTimeout(() => {
			setChromeVisible(false)
			hideChromeTimer = null
		}, 2000)
	}

	const revealChrome = () => {
		if (kind() !== 'video') {
			setChromeVisible(true)
			return
		}
		setChromeVisible(true)
		scheduleHideChrome()
	}

	const pinChrome = (pinned) => {
		chromePinned = pinned
		if (kind() !== 'video') return
		if (pinned) {
			clearHideChromeTimer()
			setChromeVisible(true)
		} else {
			scheduleHideChrome()
		}
	}

	const resetMediaState = () => {
		setPlaying(false)
		setCurrentTime(0)
		setDuration(0)
		setProgress(0)
		setChromeVisible(true)
		chromePinned = false
		clearHideChromeTimer()
	}

	const applyVolumeToMedia = () => {
		if (!mediaEl) return
		mediaEl.volume = volume()
		mediaEl.muted = muted()
	}

	createEffect(() => {
		if (!props.open || !props.file) {
			setMediaUrl('')
			setTextContent('')
			setDocxHtml('')
			setMarkdownHtml('')
			setHtmlDoc('')
			setError(null)
			setLoading(false)
			setOfficeMode(false)
			resetMediaState()
			return
		}

		const file = props.file
		const k = fileKind(file.name, file.is_file)
		let cancelled = false
		/** @type {string | null} */
		let objectUrl = null

		setError(null)
		setTextContent('')
		setDocxHtml('')
		setMarkdownHtml('')
		setHtmlDoc('')
		setOfficeMode(false)
		resetMediaState()

		const onKey = (e) => {
			if (e.key === 'Escape') {
				if (document.fullscreenElement) {
					document.exitFullscreen().catch(() => {})
					return
				}
				props.onClose()
				return
			}
			if (e.key === 'ArrowLeft') {
				e.preventDefault()
				goPrev()
			}
			if (e.key === 'ArrowRight') {
				e.preventDefault()
				goNext()
			}
		}

		const onFsChange = () => {
			setIsFullscreen(Boolean(document.fullscreenElement))
		}

		window.addEventListener('keydown', onKey)
		document.addEventListener('fullscreenchange', onFsChange)
		document.body.style.overflow = 'hidden'
		setIsFullscreen(Boolean(document.fullscreenElement))

		onCleanup(() => {
			cancelled = true
			window.removeEventListener('keydown', onKey)
			document.removeEventListener('fullscreenchange', onFsChange)
			document.body.style.overflow = ''
			if (objectUrl) URL.revokeObjectURL(objectUrl)
			clearHideChromeTimer()
			if (document.fullscreenElement && viewerEl?.contains(document.fullscreenElement)) {
				document.exitFullscreen().catch(() => {})
			}
		})

		if (['image', 'video', 'audio', 'pdf'].includes(k)) {
			setLoading(false)
			setMediaUrl(API.files.getInlineMediaUrl(props.storageId, file.path))
			if (k === 'video') scheduleHideChrome()
			return
		}

		const needsFetch =
			k === 'text' ||
			k === 'markdown' ||
			k === 'html' ||
			(k === 'document' && /\.docx$/i.test(file.name))

		if (!needsFetch) {
			setLoading(false)
			setOfficeMode(true)
			return
		}

		setLoading(true)
		setMediaUrl('')

		;(async () => {
			try {
				const blob = await API.files.download(props.storageId, file.path)
				if (cancelled) return

				if (k === 'markdown') {
					const { marked } = await import('marked')
					const raw = await blob.text()
					const html = await marked.parse(raw, {
						gfm: true,
						breaks: true,
					})
					if (cancelled) return
					setMarkdownHtml(html || '<p><em>Empty document</em></p>')
				} else if (k === 'html') {
					const raw = await blob.text()
					if (cancelled) return
					setHtmlDoc(raw || '<!doctype html><p><em>Empty document</em></p>')
				} else if (k === 'text') {
					setTextContent(await blob.text())
				} else {
					const mammoth = (await import('mammoth')).default
					const arrayBuffer = await blob.arrayBuffer()
					const result = await mammoth.convertToHtml({ arrayBuffer })
					if (cancelled) return
					setDocxHtml(result.value || '<p><em>Empty document</em></p>')
				}
			} catch (err) {
				console.error(err)
				if (!cancelled) {
					setError('Could not open this file')
					addAlert('Could not open this file', 'error')
				}
			} finally {
				if (!cancelled) setLoading(false)
			}
		})()
	})

	const downloadFile = async () => {
		if (!props.file) return
		try {
			setLoading(true)
			const blob = await API.files.download(props.storageId, props.file.path)
			const href = URL.createObjectURL(blob)
			const a = Object.assign(document.createElement('a'), {
				href,
				download: props.file.name,
				style: 'display: none',
			})
			document.body.appendChild(a)
			a.click()
			URL.revokeObjectURL(href)
			a.remove()
		} catch (err) {
			console.error(err)
			addAlert('Download failed', 'error')
		} finally {
			setLoading(false)
		}
	}

	const togglePlay = () => {
		if (!mediaEl) return
		if (mediaEl.paused) {
			mediaEl.play()
			setPlaying(true)
		} else {
			mediaEl.pause()
			setPlaying(false)
		}
	}

	const toggleMute = () => {
		if (!mediaEl) return
		const next = !muted()
		setMuted(next)
		mediaEl.muted = next
		if (!next && volume() === 0) {
			setVolume(0.5)
			mediaEl.volume = 0.5
		}
	}

	/**
	 * @param {number} next
	 */
	const changeVolume = (next) => {
		const v = Math.min(1, Math.max(0, next))
		setVolume(v)
		setMuted(v === 0)
		if (mediaEl) {
			mediaEl.volume = v
			mediaEl.muted = v === 0
		}
	}

	const seekVolume = (e) => {
		const rect = e.currentTarget.getBoundingClientRect()
		changeVolume((e.clientX - rect.left) / rect.width)
	}

	const seek = (e) => {
		if (!mediaEl || !duration()) return
		const rect = e.currentTarget.getBoundingClientRect()
		const ratio = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width))
		mediaEl.currentTime = ratio * duration()
	}

	const toggleFullscreen = async () => {
		try {
			if (document.fullscreenElement) {
				await document.exitFullscreen()
				return
			}
			const el = viewerEl
			if (!el) return
			if (el.requestFullscreen) {
				await el.requestFullscreen()
			}
		} catch (err) {
			console.error(err)
		}
	}

	const onMediaTimeUpdate = () => {
		if (!mediaEl) return
		setCurrentTime(mediaEl.currentTime)
		setProgress(duration() ? (mediaEl.currentTime / duration()) * 100 : 0)
	}

	const onMediaMeta = () => {
		if (!mediaEl) return
		setDuration(mediaEl.duration || 0)
		applyVolumeToMedia()
	}

	const VolumeGlyph = () => (
		<Show
			when={!muted() && volume() > 0}
			fallback={<VolumeOffIcon fontSize="small" />}
		>
			<Show when={volume() < 0.5} fallback={<VolumeUpIcon fontSize="small" />}>
				<VolumeDownIcon fontSize="small" />
			</Show>
		</Show>
	)

	const volumeControls = () => (
		<div class="file-viewer__volume">
			<button
				type="button"
				class="file-viewer__ctrl-btn"
				onClick={toggleMute}
				aria-label={muted() || volume() === 0 ? 'Unmute' : 'Mute'}
			>
				<VolumeGlyph />
			</button>
			<div
				class="file-viewer__volume-slider"
				onClick={seekVolume}
				role="slider"
				aria-label="Volume"
				aria-valuemin={0}
				aria-valuemax={100}
				aria-valuenow={Math.round((muted() ? 0 : volume()) * 100)}
				tabIndex={0}
				onKeyDown={(e) => {
					if (e.key === 'ArrowRight' || e.key === 'ArrowUp') {
						e.preventDefault()
						e.stopPropagation()
						changeVolume(volume() + 0.05)
					}
					if (e.key === 'ArrowLeft' || e.key === 'ArrowDown') {
						e.preventDefault()
						e.stopPropagation()
						changeVolume(volume() - 0.05)
					}
				}}
			>
				<div
					class="file-viewer__volume-fill"
					style={{ width: `${(muted() ? 0 : volume()) * 100}%` }}
				/>
			</div>
		</div>
	)

	return (
		<Show when={props.open && props.file}>
			<Portal mount={document.body}>
				<div
					ref={(el) => {
						viewerEl = el
					}}
					class="file-viewer"
					classList={{
						'file-viewer--chrome-hidden': kind() === 'video' && !chromeVisible(),
						'file-viewer--doc-nav': isDocNavKind(),
					}}
					role="dialog"
					aria-modal="true"
					aria-label={props.file?.name}
					onMouseMove={() => {
						if (kind() === 'video') revealChrome()
					}}
				>
					<button
						type="button"
						class="file-viewer__close"
						aria-label="Close"
						title="Close"
						onClick={props.onClose}
						onMouseEnter={() => pinChrome(true)}
						onMouseLeave={() => pinChrome(false)}
					>
						<CloseIcon fontSize="inherit" />
					</button>

					<button
						type="button"
						class="file-viewer__download"
						aria-label="Download"
						title="Download"
						onClick={downloadFile}
						onMouseEnter={() => pinChrome(true)}
						onMouseLeave={() => pinChrome(false)}
					>
						<DownloadIcon fontSize="inherit" />
					</button>

					<div class="file-viewer__caption">
						<span class="file-viewer__title">{props.file.name}</span>
						<span class="file-viewer__meta">
							{convertSize(props.file.size || 0)}
							{streamKinds() ? ' · streaming' : ''}
							{viewableFiles().length > 1
								? ` · ${currentIndex() + 1}/${viewableFiles().length}`
								: ''}
						</span>
					</div>

					<Show when={hasPrev()}>
						<div class="file-viewer__nav-zone file-viewer__nav-zone--prev">
							<button
								type="button"
								class="file-viewer__nav file-viewer__nav--prev"
								aria-label="Previous file"
								title="Previous file"
								onClick={goPrev}
								onMouseEnter={() => pinChrome(true)}
								onMouseLeave={() => pinChrome(false)}
							>
								<ChevronLeftIcon fontSize="inherit" />
							</button>
						</div>
					</Show>

					<Show when={hasNext()}>
						<div class="file-viewer__nav-zone file-viewer__nav-zone--next">
							<button
								type="button"
								class="file-viewer__nav file-viewer__nav--next"
								aria-label="Next file"
								title="Next file"
								onClick={goNext}
								onMouseEnter={() => pinChrome(true)}
								onMouseLeave={() => pinChrome(false)}
							>
								<ChevronRightIcon fontSize="inherit" />
							</button>
						</div>
					</Show>

					<div class="file-viewer__stage">
						<Show when={loading()}>
							<div class="file-viewer__loading">
								<CircularProgress color="secondary" />
								<span>Loading…</span>
							</div>
						</Show>

						<Show when={error()}>
							<div class="file-viewer__empty">{error()}</div>
						</Show>

						<Show when={!loading() && !error()}>
							<Show when={kind() === 'image' && mediaUrl()}>
								<img
									class="file-viewer__image"
									src={mediaUrl()}
									alt={props.file.name}
								/>
							</Show>

							<Show when={kind() === 'video' && mediaUrl()}>
								<div class="file-viewer__player">
									<video
										ref={(el) => {
											mediaEl = el
											applyVolumeToMedia()
										}}
										src={mediaUrl()}
										playsinline
										preload="metadata"
										onTimeUpdate={onMediaTimeUpdate}
										onLoadedMetadata={onMediaMeta}
										onPlay={() => setPlaying(true)}
										onPause={() => setPlaying(false)}
										onClick={togglePlay}
										class="file-viewer__video"
									/>
									<div
										class="file-viewer__controls"
										onMouseEnter={() => pinChrome(true)}
										onMouseLeave={() => pinChrome(false)}
									>
										<button
											type="button"
											class="file-viewer__ctrl-btn"
											onClick={togglePlay}
											aria-label={playing() ? 'Pause' : 'Play'}
										>
											<Show when={playing()} fallback={<PlayArrowIcon />}>
												<PauseIcon />
											</Show>
										</button>
										<span class="file-viewer__time">
											{formatTime(currentTime())} / {formatTime(duration())}
										</span>
										<div
											class="file-viewer__seek"
											onClick={seek}
											role="slider"
											aria-valuenow={progress()}
											aria-valuemin={0}
											aria-valuemax={100}
											tabIndex={0}
										>
											<div
												class="file-viewer__seek-fill"
												style={{ width: `${progress()}%` }}
											/>
										</div>
										{volumeControls()}
										<button
											type="button"
											class="file-viewer__ctrl-btn"
											onClick={toggleFullscreen}
											aria-label={
												isFullscreen() ? 'Exit fullscreen' : 'Fullscreen'
											}
										>
											<Show
												when={isFullscreen()}
												fallback={<FullscreenIcon />}
											>
												<FullscreenExitIcon />
											</Show>
										</button>
									</div>
								</div>
							</Show>

							<Show when={kind() === 'audio' && mediaUrl()}>
								<div class="file-viewer__audio-card">
									<div class="file-viewer__audio-orb" aria-hidden="true" />
									<audio
										ref={(el) => {
											mediaEl = el
											applyVolumeToMedia()
										}}
										src={mediaUrl()}
										preload="metadata"
										onTimeUpdate={onMediaTimeUpdate}
										onLoadedMetadata={onMediaMeta}
										onPlay={() => setPlaying(true)}
										onPause={() => setPlaying(false)}
									/>
									<div class="file-viewer__controls file-viewer__controls--audio">
										<button
											type="button"
											class="file-viewer__ctrl-btn file-viewer__ctrl-btn--lg"
											onClick={togglePlay}
											aria-label={playing() ? 'Pause' : 'Play'}
										>
											<Show when={playing()} fallback={<PlayArrowIcon />}>
												<PauseIcon />
											</Show>
										</button>
										<div style={{ flex: 1, 'min-width': 0 }}>
											<div class="file-viewer__audio-name">{props.file.name}</div>
											<div
												class="file-viewer__seek"
												onClick={seek}
												role="slider"
												tabIndex={0}
											>
												<div
													class="file-viewer__seek-fill"
													style={{ width: `${progress()}%` }}
												/>
											</div>
											<span class="file-viewer__time">
												{formatTime(currentTime())} / {formatTime(duration())}
											</span>
										</div>
										{volumeControls()}
									</div>
								</div>
							</Show>

							<Show when={kind() === 'pdf' && mediaUrl()}>
								<iframe
									class="file-viewer__iframe"
									src={mediaUrl()}
									title={props.file.name}
								/>
							</Show>

							<Show when={kind() === 'text' && textContent()}>
								<pre class="file-viewer__text">{textContent()}</pre>
							</Show>

							<Show when={kind() === 'markdown' && markdownHtml()}>
								<div
									class="file-viewer__markdown"
									innerHTML={markdownHtml()}
								/>
							</Show>

							<Show when={kind() === 'html' && htmlDoc()}>
								<iframe
									class="file-viewer__html"
									title={props.file.name}
									sandbox=""
									srcdoc={htmlDoc()}
								/>
							</Show>

							<Show when={docxHtml()}>
								<div class="file-viewer__docx" innerHTML={docxHtml()} />
							</Show>

							<Show when={officeMode()}>
								<div class="file-viewer__office">
									<FileTypeIcon
										name={props.file.name}
										isFile={true}
										size={88}
									/>
									<p>
										{kind() === 'presentation'
											? 'Presentations open best in PowerPoint or LibreOffice.'
											: kind() === 'spreadsheet'
												? 'Spreadsheets open best in Excel or LibreOffice.'
												: kind() === 'document'
													? 'This document format needs an external app to view fully.'
													: 'Preview is not available for this file type.'}
									</p>
									<Button
										variant="contained"
										color="secondary"
										startIcon={<DownloadIcon />}
										onClick={downloadFile}
									>
										Download & open
									</Button>
								</div>
							</Show>
						</Show>
					</div>
				</div>
			</Portal>
		</Show>
	)
}

export default FileViewer
