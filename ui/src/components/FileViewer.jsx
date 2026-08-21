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
import ZoomInIcon from '@suid/icons-material/ZoomIn'
import ZoomOutIcon from '@suid/icons-material/ZoomOut'

import API from '../api'
import { fileKind } from '../common/fileKind'
import {
	ZOOM_MAX,
	ZOOM_MIN,
	createImageZoom,
} from '../common/imageZoom'
import { convertSize } from '../common/size_converter'
import { nativeClientStore } from '../common/nativeClient'
import { loadPreview } from '../common/previewLoader'
import { pauseThumbQueue, resumeThumbQueue } from '../common/thumbQueue'
import { acquireObjectUrl, releaseObjectUrl } from '../common/objectUrlPool'
import { sanitizeHtml } from '../common/sanitizeHtml'
import { t } from '../common/i18n'
import FileTypeIcon from './FileTypeIcon'
import LoadingDots from './LoadingDots'
import { alertStore } from './AlertStack'

/** How far one arrow-key press slides a zoomed photo. */
const PAN_STEP_PX = 80

const formatTime = (sec) => {
	if (!Number.isFinite(sec) || sec < 0) return '0:00'
	const m = Math.floor(sec / 60)
	const s = Math.floor(sec % 60)
	return `${m}:${String(s).padStart(2, '0')}`
}

/**
 * True when a click lands in the letterbox (empty) area of an
 * object-fit:contain media element, not on the painted content.
 * @param {HTMLImageElement | HTMLVideoElement} el
 * @param {number} clientX
 * @param {number} clientY
 */
const isLetterboxClick = (el, clientX, clientY) => {
	const nw = el instanceof HTMLVideoElement ? el.videoWidth : el.naturalWidth
	const nh = el instanceof HTMLVideoElement ? el.videoHeight : el.naturalHeight
	if (!nw || !nh) return false
	const rect = el.getBoundingClientRect()
	const scale = Math.min(rect.width / nw, rect.height / nh)
	const w = nw * scale
	const h = nh * scale
	const left = rect.left + (rect.width - w) / 2
	const top = rect.top + (rect.height - h) / 2
	return clientX < left || clientX > left + w || clientY < top || clientY > top + h
}

/**
 * @typedef {Object} FileViewerProps
 * @property {boolean} open
 * @property {import("../api").FSElement | null} file
 * @property {import("../api").FSElement[]} [files]
 * @property {string} storageId
 * @property {() => void} onClose
 * @property {(file: import("../api").FSElement) => void} [onNavigate]
 * @property {(path: string) => string} [resolveInlineUrl] Override authenticated inline URL (public shares)
 * @property {(path: string) => string} [resolvePreviewUrl] Override authenticated preview URL (public shares)
 * @property {(path: string, onProgress?: (progress: import("../api").DownloadProgress) => void) => Promise<Blob>} [resolveDownload] Override authenticated download (public shares)
 */

/**
 * Fullscreen file preview / media player.
 * @param {FileViewerProps} props
 */
const FileViewer = (props) => {
	const { addAlert } = alertStore
	const { isNative } = nativeClientStore
	const [loading, setLoading] = createSignal(false)
	const [firstChunkLoading, setFirstChunkLoading] = createSignal(false)
	const [isDownloading, setIsDownloading] = createSignal(false)
	/** @type {[import('solid-js').Accessor<import('../api').DownloadProgress | null>, any]} */
	const [downloadProgress, setDownloadProgress] = createSignal(null)
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
	const [navPeekLeft, setNavPeekLeft] = createSignal(false)
	const [navPeekRight, setNavPeekRight] = createSignal(false)
	const [zoom, setZoom] = createSignal(ZOOM_MIN)
	const [zoomOffset, setZoomOffset] = createSignal({ x: 0, y: 0 })

	/** @type {HTMLVideoElement | HTMLAudioElement | undefined} */
	let mediaEl
	/** @type {HTMLElement | undefined} */
	let viewerEl
	/** @type {HTMLImageElement | undefined} */
	let imageEl
	/** @type {ReturnType<typeof setTimeout> | null} */
	let hideChromeTimer = null
	let chromePinned = false
	/** True while a silent play→pause is used only to start buffering. */
	let silentBufferKick = false
	/** Paths already prefetched (or attempted) this session — avoid refetching. */
	const prefetchedPreviews = new Set()

	const kind = () =>
		props.file ? fileKind(props.file.name, props.file.is_file) : 'generic'

	/** Document-like previews where side nav would cover the text. */
	const isDocNavKind = () => kind() === 'markdown' || kind() === 'html'

	const streamKinds = () =>
		['image', 'video', 'audio', 'pdf'].includes(kind())

	const updateDocNavPeek = (clientX, clientY, target) => {
		if (!isDocNavKind() || !target) {
			setNavPeekLeft(false)
			setNavPeekRight(false)
			return
		}
		const rect = target.getBoundingClientRect()
		const x = clientX - rect.left
		const y = clientY - rect.top
		// Keep top chrome (close / download) free of edge peek.
		if (y < 96) {
			setNavPeekLeft(false)
			setNavPeekRight(false)
			return
		}
		const zone = 72
		setNavPeekLeft(x <= zone)
		setNavPeekRight(x >= rect.width - zone)
	}

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

	/** @type {HTMLElement | undefined} */
	let zoomSurfaceEl
	/** @type {(() => void) | null} */
	let detachZoom = null

	const isZoomable = () => kind() === 'image'

	/**
	 * The photo's painted box at 1x. `object-fit: contain` letterboxes it, and
	 * panning has to stop at the picture's edge, not the viewport's.
	 */
	const paintedImageSize = () => {
		if (!imageEl || !zoomSurfaceEl) return null
		const nw = imageEl.naturalWidth
		const nh = imageEl.naturalHeight
		const rect = zoomSurfaceEl.getBoundingClientRect()
		if (!nw || !nh || !rect.width || !rect.height) return null
		const fit = Math.min(rect.width / nw, rect.height / nh)
		return { width: nw * fit, height: nh * fit }
	}

	const zoomer = createImageZoom({
		onChange: ({ scale, offset }) => {
			setZoom(scale)
			setZoomOffset(offset)
		},
		onSwipe: (direction) => {
			if (direction === 'next') goNext()
			else goPrev()
		},
		onDismiss: () => props.onClose(),
		onTap: (e) => {
			// Tapping the empty bars around an unzoomed photo closes, exactly as
			// clicking the backdrop does.
			if (zoomer.scale > ZOOM_MIN) return
			if (imageEl && isLetterboxClick(imageEl, e.clientX, e.clientY)) props.onClose()
		},
		isEnabled: isZoomable,
		getContentSize: paintedImageSize,
	})

	const zoomIn = () => zoomer.zoomIn()
	const zoomOut = () => zoomer.zoomOut()
	const resetZoom = () => zoomer.reset()
	const panBy = (dx, dy) => zoomer.panBy(dx, dy)
	const zoomPercent = () => `${Math.round(zoom() * 100)}%`

	const attachZoom = (el) => {
		zoomSurfaceEl = el
		detachZoom?.()
		detachZoom = el ? zoomer.attach(el) : null
	}

	const releaseZoom = () => {
		detachZoom?.()
		detachZoom = null
		zoomSurfaceEl = undefined
		imageEl = undefined
	}

	// Closing the viewer unmounts the surface, but Solid never calls a ref with
	// null, so without this its listeners and its hold on the decoded photo
	// would outlive it. Reopening runs the ref again and re-attaches.
	createEffect(() => {
		if (!props.open) releaseZoom()
	})

	onCleanup(releaseZoom)

	const inlineUrlFor = async (path) =>
		props.resolveInlineUrl
			? props.resolveInlineUrl(path)
			: await API.files.getInlineMediaUrl(props.storageId, path)

	const previewUrlFor = async (path) =>
		props.resolvePreviewUrl
			? props.resolvePreviewUrl(path)
			: await API.files.getPreviewUrl(props.storageId, path)

	const previewScope = () => props.storageId || 'share'

	/**
	 * Warm the preview for a neighboring photo before the user opens it.
	 * Best-effort and silent: shares the loader (and therefore the in-flight
	 * request) with the open path, so a swipe onto a still-downloading neighbor
	 * joins that download instead of starting a second one. Server and native
	 * caches are both size-capped and LRU-evicted, so over-prefetching just
	 * churns the LRU tail rather than growing unbounded.
	 */
	const prefetchPreview = async (path) => {
		if (!path || prefetchedPreviews.has(path)) return
		prefetchedPreviews.add(path)
		try {
			await loadPreview({
				scope: previewScope(),
				path,
				resolveUrl: () => previewUrlFor(path),
				native: isNative(),
			})
		} catch {
			// Leave the path retryable: a prefetch that failed on a flaky
			// connection must not permanently poison the photo it was warming.
			prefetchedPreviews.delete(path)
		}
	}

	/**
	 * `onProgress` is optional on purpose: the document/text loaders below want
	 * the bytes and nothing else, while the download button wants a bar.
	 * @param {string} path
	 * @param {(progress: import('../api').DownloadProgress) => void} [onProgress]
	 */
	const downloadBlobFor = (path, onProgress) =>
		props.resolveDownload
			? props.resolveDownload(path, onProgress)
			: API.files.downloadWithProgress(props.storageId, path, onProgress)

	createEffect(() => {
		if (!props.open || !props.file?.is_file || !props.storageId) return
		if (props.resolveDownload || props.resolveInlineUrl) return
		const path = props.file.path
		if (!path) return
		API.files.recordRecent(props.storageId, path)
	})

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
		silentBufferKick = false
		setPlaying(false)
		setCurrentTime(0)
		setDuration(0)
		setProgress(0)
		setChromeVisible(true)
		setFirstChunkLoading(false)
		chromePinned = false
		clearHideChromeTimer()
	}

	const applyVolumeToMedia = () => {
		if (!mediaEl) return
		mediaEl.volume = volume()
		mediaEl.muted = muted()
	}

	/**
	 * Browsers often treat preload="auto" as metadata-only until playback is
	 * requested, so Range buffering never starts on open. Kick the media
	 * pipeline with muted play→pause (no audible autoplay). If that is blocked,
	 * issue a tiny Range fetch so the backend can start the first chunk.
	 * @param {HTMLVideoElement} el
	 */
	const kickVideoBuffer = (el) => {
		if (!el) return
		el.preload = 'auto'
		try {
			el.load()
		} catch {
			/* ignore */
		}

		const preferMuted = muted()
		const src = el.currentSrc || el.src
		const warmFirstBytes = () => {
			if (!src) return
			fetch(src, {
				headers: { Range: 'bytes=0-1023' },
				credentials: 'include',
			}).catch(() => {})
		}

		silentBufferKick = true
		el.muted = true
		const attempt = el.play()
		if (!attempt || typeof attempt.then !== 'function') {
			silentBufferKick = false
			el.muted = preferMuted
			applyVolumeToMedia()
			warmFirstBytes()
			return
		}

		attempt
			.then(() => {
				if (mediaEl !== el) return
				if (!silentBufferKick) return
				silentBufferKick = false
				el.pause()
				try {
					el.currentTime = 0
				} catch {
					/* ignore */
				}
				el.muted = preferMuted
				applyVolumeToMedia()
				setPlaying(false)
			})
			.catch(() => {
				if (mediaEl !== el) return
				silentBufferKick = false
				el.muted = preferMuted
				applyVolumeToMedia()
				warmFirstBytes()
			})
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
			resetZoom()
			return
		}

		const file = props.file
		const k = fileKind(file.name, file.is_file)
		let cancelled = false
		/** @type {string | null} */
		let objectUrlKey = null

		setError(null)
		// Cleared here, not just on close: switching between two images in the
		// same session must not let the previous photo's (now revoked) URL
		// leak through the loading gate below.
		setMediaUrl('')
		setTextContent('')
		setDocxHtml('')
		setMarkdownHtml('')
		setHtmlDoc('')
		setOfficeMode(false)
		resetMediaState()
		resetZoom()

		const onKey = (e) => {
			if (e.key === 'Escape') {
				if (document.fullscreenElement) {
					document.exitFullscreen().catch(() => {})
					return
				}
				if (zoomer.scale > ZOOM_MIN) {
					resetZoom()
					return
				}
				props.onClose()
				return
			}
			if (k === 'image') {
				if (e.key === '+' || e.key === '=') {
					e.preventDefault()
					zoomIn()
					return
				}
				if (e.key === '-' || e.key === '_') {
					e.preventDefault()
					zoomOut()
					return
				}
				if (e.key === '0') {
					e.preventDefault()
					resetZoom()
					return
				}
			}
			// While zoomed the arrows drive the pan; stepping to the next file
			// would throw away what the user is looking at.
			if (e.key === 'ArrowLeft') {
				e.preventDefault()
				if (zoomer.scale > ZOOM_MIN) panBy(PAN_STEP_PX, 0)
				else goPrev()
			}
			if (e.key === 'ArrowRight') {
				e.preventDefault()
				if (zoomer.scale > ZOOM_MIN) panBy(-PAN_STEP_PX, 0)
				else goNext()
			}
			if (e.key === 'ArrowUp' && zoomer.scale > ZOOM_MIN) {
				e.preventDefault()
				panBy(0, PAN_STEP_PX)
			}
			if (e.key === 'ArrowDown' && zoomer.scale > ZOOM_MIN) {
				e.preventDefault()
				panBy(0, -PAN_STEP_PX)
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
			if (objectUrlKey) releaseObjectUrl(objectUrlKey)
			clearHideChromeTimer()
			if (document.fullscreenElement && viewerEl?.contains(document.fullscreenElement)) {
				document.exitFullscreen().catch(() => {})
			}
		})

		if (['image', 'video', 'audio', 'pdf'].includes(k)) {
			setLoading(false)
			setFirstChunkLoading(k === 'video')
			if (k === 'image') {
				setLoading(true)
				;(async () => {
					const path = file.path
					try {
						// The loader owns cache lookup, the network fetch and the cache
						// write, and dedupes against a neighbor prefetch already in
						// flight for this same photo.
						const blob = await loadPreview({
							scope: previewScope(),
							path,
							resolveUrl: () => previewUrlFor(path),
							native: isNative(),
						})
						objectUrlKey = `${previewScope()}:${path}`
						const url = acquireObjectUrl(objectUrlKey, blob)
						if (!cancelled) {
							setMediaUrl(url)
						} else {
							releaseObjectUrl(objectUrlKey)
							objectUrlKey = null
						}
					} catch (err) {
						// Last resort: let the browser load <img src=preview?access_token=…>
						// directly, which also covers a blocked fetch/CORS edge case.
						try {
							const url = await previewUrlFor(path)
							if (!cancelled) setMediaUrl(url)
						} catch (urlErr) {
							console.error(urlErr)
							if (!cancelled) {
								setError(t('viewer.openFailed'))
								addAlert(t('viewer.openFailed'), 'error')
							}
						}
					} finally {
						if (!cancelled) setLoading(false)
					}
				})()
				return
			}
			setLoading(true)
			;(async () => {
				const url = await inlineUrlFor(file.path)
				if (cancelled) return
				setMediaUrl(url)
				setLoading(false)
				if (k === 'video') scheduleHideChrome()
			})()
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
				const blob = await downloadBlobFor(file.path)
				if (cancelled) return

				if (k === 'markdown') {
					const { marked } = await import('marked')
					const raw = await blob.text()
					const html = await marked.parse(raw, {
						gfm: true,
						breaks: true,
					})
					if (cancelled) return
					setMarkdownHtml(await sanitizeHtml(html) || `<p><em>${t('viewer.emptyDocument')}</em></p>`)
				} else if (k === 'html') {
					const raw = await blob.text()
					if (cancelled) return
					setHtmlDoc(raw || `<!doctype html><p><em>${t('viewer.emptyDocument')}</em></p>`)
				} else if (k === 'text') {
					setTextContent(await blob.text())
				} else {
					const mammoth = (await import('mammoth')).default
					const arrayBuffer = await blob.arrayBuffer()
					const result = await mammoth.convertToHtml({ arrayBuffer })
					if (cancelled) return
					setDocxHtml(await sanitizeHtml(result.value) || `<p><em>${t('viewer.emptyDocument')}</em></p>`)
				}
			} catch (err) {
				console.error(err)
				if (!cancelled) {
					setError(t('viewer.openFailed'))
					addAlert(t('viewer.openFailed'), 'error')
				}
			} finally {
				if (!cancelled) setLoading(false)
			}
		})()
	})

	// Warm the next couple of photos' previews while the current one is open,
	// so swiping forward rarely hits a cold fetch.
	createEffect(() => {
		if (!props.open || kind() !== 'image') return
		const files = viewableFiles()
		const i = currentIndex()
		if (i < 0) return
		const candidates = [files[i + 1], files[i + 2], files[i - 1]].filter(Boolean)
		for (const f of candidates) {
			if (fileKind(f.name, f.is_file) === 'image') prefetchPreview(f.path)
		}
	})

	// A grid of photos keeps the browser's ~6 connections per origin busy with
	// thumb fetches, each costing a Telegram round trip, and a video opened over
	// that has to queue its Range requests behind them — which the user
	// experiences as playback taking seconds to start. Hold the thumb queue for
	// as long as a video is open; the tiles behind the player are covered
	// anyway, and pausing (rather than aborting) keeps their pending work.
	createEffect(() => {
		if (!props.open || kind() !== 'video') return
		pauseThumbQueue()
		onCleanup(resumeThumbQueue)
	})

	// Start first-chunk buffering as soon as the video element has a URL —
	// do not wait for the user to press Play.
	createEffect(() => {
		const url = mediaUrl()
		const file = props.file
		if (!props.open || !file || !url) return
		if (fileKind(file.name, file.is_file) !== 'video') return

		let cancelled = false
		queueMicrotask(() => {
			if (cancelled || !mediaEl) return
			kickVideoBuffer(mediaEl)
		})
		onCleanup(() => {
			cancelled = true
		})
	})

	const onFirstChunkReady = () => {
		setFirstChunkLoading(false)
	}

	const downloadFile = async () => {
		if (!props.file || isDownloading()) return
		try {
			// The "preparing download" overlay covers the player and eats
			// input, so a still-playing video/audio track would keep running
			// behind a modal the user can no longer reach to pause.
			if (mediaEl && !mediaEl.paused) {
				silentBufferKick = false
				mediaEl.pause()
				setPlaying(false)
			}
			setIsDownloading(true)
			const blob = await downloadBlobFor(props.file.path, setDownloadProgress)
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
			addAlert(t('viewer.downloadStarted'), 'success')
		} catch (err) {
			console.error(err)
			addAlert(t('viewer.downloadFailed'), 'error')
		} finally {
			setIsDownloading(false)
			setDownloadProgress(null)
		}
	}

	const togglePlay = () => {
		if (!mediaEl) return
		// User took over — do not let a pending silent buffer-kick pause/reset.
		silentBufferKick = false
		if (mediaEl.paused) {
			mediaEl.muted = muted()
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
				aria-label={muted() || volume() === 0 ? t('viewer.unmute') : t('viewer.mute')}
			>
				<VolumeGlyph />
			</button>
			<div
				class="file-viewer__volume-slider"
				onClick={seekVolume}
				role="slider"
				aria-label={t('viewer.volume')}
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
						'file-viewer--zoomed': zoom() > ZOOM_MIN,
					}}
					role="dialog"
					aria-modal="true"
					aria-label={props.file?.name}
					onMouseMove={(e) => {
						if (kind() === 'video') revealChrome()
						updateDocNavPeek(e.clientX, e.clientY, e.currentTarget)
					}}
					onMouseLeave={() => {
						setNavPeekLeft(false)
						setNavPeekRight(false)
					}}
				>
					<header
						class="file-viewer__chrome"
						onMouseEnter={() => pinChrome(true)}
						onMouseLeave={() => pinChrome(false)}
					>
						<div class="file-viewer__caption">
							<span class="file-viewer__title">{props.file.name}</span>
							<span class="file-viewer__meta">
								{convertSize(props.file.size || 0)}
								{streamKinds() ? ` · ${t('viewer.streaming')}` : ''}
								{viewableFiles().length > 1
									? ` · ${currentIndex() + 1}/${viewableFiles().length}`
									: ''}
							</span>
						</div>
						<div class="file-viewer__actions">
							<Show when={isZoomable()}>
								{/*
								  * The buttons carry `aria-disabled`, not `disabled`: a real
								  * disabled attribute drops focus to the body the moment the
								  * zoom reaches the end of its range, and a keyboard user has
								  * to tab in from the top of the document all over again.
								  * Every handler already clamps, so a press at the end is a
								  * no-op.
								  */}
								<div class="file-viewer__zoom" role="group" aria-label={t('viewer.zoom')}>
									<button
										type="button"
										class="file-viewer__zoom-btn"
										aria-label={t('viewer.zoomOut')}
										title={t('viewer.zoomOutTitle')}
										aria-disabled={zoom() <= ZOOM_MIN}
										onClick={zoomOut}
									>
										<ZoomOutIcon fontSize="inherit" />
									</button>
									<button
										type="button"
										class="file-viewer__zoom-level"
										aria-label={t('viewer.resetZoom')}
										title={t('viewer.resetZoomTitle')}
										aria-disabled={zoom() === ZOOM_MIN}
										onClick={resetZoom}
									>
										{zoomPercent()}
									</button>
									<button
										type="button"
										class="file-viewer__zoom-btn"
										aria-label={t('viewer.zoomIn')}
										title={t('viewer.zoomInTitle')}
										aria-disabled={zoom() >= ZOOM_MAX}
										onClick={zoomIn}
									>
										<ZoomInIcon fontSize="inherit" />
									</button>
								</div>
							</Show>
							<button
								type="button"
								class="file-viewer__download"
								aria-label={t('viewer.download')}
								title={t('viewer.download')}
								disabled={isDownloading()}
								onClick={downloadFile}
							>
								<DownloadIcon fontSize="inherit" />
							</button>
							<button
								type="button"
								class="file-viewer__close"
								aria-label={t('common.close')}
								title={t('common.close')}
								onClick={props.onClose}
							>
								<CloseIcon fontSize="inherit" />
							</button>
						</div>
					</header>

					<div class="file-viewer__body">
					<Show when={isDocNavKind() && hasPrev()}>
						<div
							class="file-viewer__edge-sense file-viewer__edge-sense--prev"
							aria-hidden="true"
							onMouseEnter={() => setNavPeekLeft(true)}
							onMouseLeave={(e) => {
								if (!e.relatedTarget?.closest?.('.file-viewer__nav--prev')) {
									setNavPeekLeft(false)
								}
							}}
						/>
					</Show>
					<Show when={isDocNavKind() && hasNext()}>
						<div
							class="file-viewer__edge-sense file-viewer__edge-sense--next"
							aria-hidden="true"
							onMouseEnter={() => setNavPeekRight(true)}
							onMouseLeave={(e) => {
								if (!e.relatedTarget?.closest?.('.file-viewer__nav--next')) {
									setNavPeekRight(false)
								}
							}}
						/>
					</Show>

					<Show when={hasPrev()}>
						<button
							type="button"
							class="file-viewer__nav file-viewer__nav--prev"
							classList={{
								'file-viewer__nav--peek': !isDocNavKind() || navPeekLeft(),
							}}
							aria-label={t('viewer.previousFile')}
							title={t('viewer.previousFile')}
							tabIndex={isDocNavKind() && !navPeekLeft() ? -1 : 0}
							onClick={goPrev}
							onMouseEnter={() => {
								if (isDocNavKind()) setNavPeekLeft(true)
								pinChrome(true)
							}}
							onMouseLeave={() => {
								if (isDocNavKind()) setNavPeekLeft(false)
								pinChrome(false)
							}}
						>
							<ChevronLeftIcon fontSize="inherit" />
						</button>
					</Show>

					<Show when={hasNext()}>
						<button
							type="button"
							class="file-viewer__nav file-viewer__nav--next"
							classList={{
								'file-viewer__nav--peek': !isDocNavKind() || navPeekRight(),
							}}
							aria-label={t('viewer.nextFile')}
							title={t('viewer.nextFile')}
							tabIndex={isDocNavKind() && !navPeekRight() ? -1 : 0}
							onClick={goNext}
							onMouseEnter={() => {
								if (isDocNavKind()) setNavPeekRight(true)
								pinChrome(true)
							}}
							onMouseLeave={() => {
								if (isDocNavKind()) setNavPeekRight(false)
								pinChrome(false)
							}}
						>
							<ChevronRightIcon fontSize="inherit" />
						</button>
					</Show>

					<div
						class="file-viewer__stage"
						onClick={(e) => {
							if (e.target === e.currentTarget) props.onClose()
						}}
					>
						<Show when={loading()}>
							<div class="file-viewer__loading">
								<span>
									{t('viewer.loading')}
									<LoadingDots />
								</span>
							</div>
						</Show>

						<Show when={error()}>
							<div class="file-viewer__empty">{error()}</div>
						</Show>

						<Show when={!error() && !loading()}>
							<Show when={kind() === 'image' && mediaUrl()}>
								{/* Pinch, double-tap, drag and swipe all land here; the
								    magnifier buttons drive the same state. */}
								<div
									class="file-viewer__zoom-surface"
									classList={{
										'file-viewer__zoom-surface--zoomed': zoom() > ZOOM_MIN,
									}}
									ref={attachZoom}
									data-zoom={zoom()}
								>
									<img
										ref={(el) => {
											imageEl = el
										}}
										class="file-viewer__image"
										src={mediaUrl()}
										alt={props.file.name}
										draggable={false}
										style={{
											transform: `translate(${zoomOffset().x}px, ${zoomOffset().y}px) scale(${zoom()})`,
										}}
										onError={() => {
											setError(t('viewer.openFailed'))
											addAlert(t('viewer.openFailed'), 'error')
										}}
									/>
								</div>
							</Show>

							<Show when={kind() === 'video' && mediaUrl()}>
								<div class="file-viewer__player">
									<video
										ref={(el) => {
											mediaEl = el
											applyVolumeToMedia()
											if (el && el.readyState >= 2) {
												setFirstChunkLoading(false)
											}
										}}
										src={mediaUrl()}
										playsinline
										preload="auto"
										onTimeUpdate={onMediaTimeUpdate}
										onLoadedMetadata={onMediaMeta}
										onLoadedData={onFirstChunkReady}
										onCanPlay={onFirstChunkReady}
										onError={onFirstChunkReady}
										onPlay={() => {
											if (!silentBufferKick) setPlaying(true)
										}}
										onPause={() => setPlaying(false)}
										onClick={(e) => {
											if (isLetterboxClick(e.currentTarget, e.clientX, e.clientY)) {
												props.onClose()
												return
											}
											togglePlay()
										}}
										class="file-viewer__video"
									/>
									<Show when={firstChunkLoading()}>
										<div
											class="file-viewer__buffering"
											aria-live="polite"
											aria-busy="true"
										>
											<CircularProgress color="secondary" size={48} />
										</div>
									</Show>
									<div
										class="file-viewer__controls"
										onMouseEnter={() => pinChrome(true)}
										onMouseLeave={() => pinChrome(false)}
									>
										<button
											type="button"
											class="file-viewer__ctrl-btn"
											onClick={togglePlay}
											aria-label={playing() ? t('viewer.pause') : t('viewer.play')}
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
												isFullscreen() ? t('viewer.exitFullscreen') : t('viewer.fullscreen')
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
											aria-label={playing() ? t('viewer.pause') : t('viewer.play')}
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
											? t('viewer.officePresentation')
											: kind() === 'spreadsheet'
												? t('viewer.officeSpreadsheet')
												: kind() === 'document'
													? t('viewer.officeDocument')
													: t('viewer.officeGeneric')}
									</p>
									<Button
										variant="contained"
										color="secondary"
										startIcon={<DownloadIcon />}
										onClick={downloadFile}
									>
										{t('viewer.downloadAndOpen')}
									</Button>
								</div>
							</Show>
						</Show>
					</div>
					</div>

					<Show when={isDownloading()}>
						<div class="download-preparing" role="status" aria-live="polite">
							<Show
								when={downloadProgress()?.total}
								fallback={
									<>
										<div class="download-preparing__text">
											{t('viewer.preparingDownload')}
											<LoadingDots />
										</div>
										<div class="download-preparing__hint">
											{t('viewer.preparingDownloadHint')}
										</div>
									</>
								}
							>
								<div class="download-preparing__text">
									{t('viewer.downloadingProgress', {
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
					</Show>
				</div>
			</Portal>
		</Show>
	)
}

export default FileViewer
