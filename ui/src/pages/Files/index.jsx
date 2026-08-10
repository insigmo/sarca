import { useBeforeLeave, useNavigate, useParams } from '@solidjs/router'
import {
	For,
	Show,
	createEffect,
	createMemo,
	createSignal,
	mapArray,
	onCleanup,
	onMount,
} from 'solid-js'
import MenuItem from '@suid/material/MenuItem'
import Button from '@suid/material/Button'
import IconButton from '@suid/material/IconButton'
import Stack from '@suid/material/Stack'
import Typography from '@suid/material/Typography'
import MenuMUI from '@suid/material/Menu'
import Divider from '@suid/material/Divider'
import ListItemIcon from '@suid/material/ListItemIcon'
import ListItemText from '@suid/material/ListItemText'

import API from '../../api'
import FSListItem from '../../components/FSListItem'
import CreateFolderDialog from '../../components/CreateFolderDialog'
import FolderPickerDialog from '../../components/FolderPickerDialog'
import { alertStore } from '../../components/AlertStack'
import FileViewer from '../../components/FileViewer'
import RestoreConflictDialog from '../../components/RestoreConflictDialog'
import ActionConfirmDialog from '../../components/ActionConfirmDialog'
import FilesSidebar from '../../components/FilesSidebar'
import FilesSearch from '../../components/FilesSearch'
import FluentIcon from '../../components/FluentIcon'
import SharedLinksPanel from '../../components/SharedLinksPanel'
import { filesChromeStore } from '../../common/filesChrome'
import { attachPullToRefresh } from '../../common/pullToRefresh'
import { clearThumbQueue } from '../../common/thumbQueue'
import { warmPreview } from '../../common/previewLoader'
import { fileKind } from '../../common/fileKind'
import { isNativeClient } from '../../common/nativeClient'
import { sortFsElements, sortLabel } from '../../common/sortFs'
import { formatBulkDeleteMessage } from '../../common/bulkDeleteMessage'
import { createRafBatcher } from '../../common/rafBatch'
import { shouldRefreshOnVisibilityEvent } from '../../common/visibilityRefresh'
import { startAutoRefresh } from '../../common/autoRefresh'
import {
	fsLayerCacheKey,
	readFsLayerCache,
	writeFsLayerCache,
} from '../../common/filesListCache'
import {
	pushViewerHistory,
	shouldCloseViewerOnPopstate,
} from '../../common/viewerHistory'
import {
	MAX_FILE_PICKER,
	uploadQueueStore,
} from '../../common/uploadQueue'

const joinStoragePath = (...parts) =>
	parts
		.filter((p) => p != null && String(p).length > 0)
		.map((p) => String(p).replace(/^\/+|\/+$/g, '').trim())
		.filter(Boolean)
		.join('/')

/**
 * Decode a single URL path segment for display (and API path use).
 * Treats `+` as space; peels limited double-encoding until stable.
 * @param {string} segment
 */
const decodePathSegment = (segment) => {
	let s = String(segment ?? '').replace(/\+/g, ' ')
	for (let i = 0; i < 3; i++) {
		if (!/%[0-9A-Fa-f]{2}/.test(s)) break
		try {
			const next = decodeURIComponent(s)
			if (next === s) break
			s = next
		} catch {
			break
		}
	}
	return s
}

/**
 * Decode each segment of a storage path from the router splat.
 * @param {string} path
 */
const decodeStoragePath = (path) => {
	const raw = String(path || '')
	const trailing = raw.endsWith('/') && raw.replace(/\/+$/, '').length > 0
	const decoded = raw
		.split('/')
		.filter((p) => p.length)
		.map(decodePathSegment)
		.join('/')
	return trailing ? `${decoded}/` : decoded
}

/**
 * Encode each path segment for in-app navigation URLs.
 * @param {string} path
 */
const encodeStoragePath = (path) => {
	const raw = String(path || '')
	const trailing = raw.endsWith('/')
	const encoded = raw
		.split('/')
		.filter((p) => p.length)
		.map(encodeURIComponent)
		.join('/')
	return trailing && encoded ? `${encoded}/` : encoded
}

const shouldSkipUploadEntry = (relativePath) => {
	const base = relativePath.split('/').pop() || ''
	return !base || base === '.DS_Store' || base.startsWith('._')
}

const itemNormalizedPath = (el) => {
	if (el.is_file) return el.path
	return el.path.endsWith('/') ? el.path : `${el.path}/`
}

const SARCA_DRAG_MIME = 'application/x-sarca-paths'

/** Destination folder path for API (no trailing slash). */
const folderDestPath = (path) => String(path || '').replace(/\/+$/, '')

/**
 * @param {string} sourcePath Normalized source (folders end with /)
 * @param {string} dest Destination folder without trailing /
 */
const isBlockedDest = (sourcePath, dest) => {
	if (!sourcePath.endsWith('/')) return false
	const d = dest.endsWith('/') ? dest : `${dest}/`
	return d === sourcePath || d.startsWith(sourcePath)
}

/**
 * @param {FileSystemDirectoryEntry} dirEntry
 * @returns {Promise<FileSystemEntry[]>}
 */
const readAllDirectoryEntries = (dirEntry) =>
	new Promise((resolve, reject) => {
		const reader = dirEntry.createReader()
		/** @type {FileSystemEntry[]} */
		const all = []
		const readBatch = () => {
			reader.readEntries((batch) => {
				if (!batch.length) {
					resolve(all)
					return
				}
				all.push(...batch)
				readBatch()
			}, reject)
		}
		readBatch()
	})

/**
 * @param {FileSystemEntry} entry
 * @param {string} prefix
 * @returns {Promise<{ file: File, relativePath: string }[]>}
 */
const collectEntryFiles = async (entry, prefix = '') => {
	if (entry.isFile) {
		const file = await new Promise((resolve, reject) => {
			/** @type {FileSystemFileEntry} */
			const fileEntry = /** @type {any} */ (entry)
			fileEntry.file(resolve, reject)
		})
		const relativePath = prefix ? `${prefix}${file.name}` : file.name
		if (shouldSkipUploadEntry(relativePath)) return []
		return [{ file, relativePath }]
	}
	if (!entry.isDirectory) return []
	const dirPrefix = `${prefix}${entry.name}/`
	const children = await readAllDirectoryEntries(
		/** @type {FileSystemDirectoryEntry} */ (/** @type {any} */ (entry)),
	)
	/** @type {{ file: File, relativePath: string }[]} */
	const out = []
	for (const child of children) {
		out.push(...(await collectEntryFiles(child, dirPrefix)))
	}
	return out
}

/**
 * @param {DataTransfer} dt
 * @returns {Promise<{ file: File, relativePath: string }[]>}
 */
const filesFromDataTransfer = async (dt) => {
	const items = dt.items
	if (items?.length && typeof items[0].webkitGetAsEntry === 'function') {
		/** @type {{ file: File, relativePath: string }[]} */
		const out = []
		for (let i = 0; i < items.length; i++) {
			const entry = items[i].webkitGetAsEntry?.()
			if (!entry) continue
			out.push(...(await collectEntryFiles(entry)))
		}
		if (out.length) return out
	}
	return Array.from(dt.files || [])
		.filter((file) => !shouldSkipUploadEntry(file.name))
		.map((file) => ({
			file,
			relativePath: file.webkitRelativePath || file.name,
		}))
}

const VIEW_MODE_KEY = 'sarca.filesViewMode'

/** @returns {'tiles' | 'list'} */
const readStoredViewMode = () => {
	try {
		const v = localStorage.getItem(VIEW_MODE_KEY)
		if (v === 'list' || v === 'tiles') return v
	} catch {
		/* ignore */
	}
	return 'tiles'
}

const Files = () => {
	const { addAlert } = alertStore
	const chrome = filesChromeStore
	/**
	 * @type {[import("solid-js").Accessor<import("../../api").FSElement[]>, any]}
	 */
	const [fsLayer, setFsLayer] = createSignal([])
	const [isCreateFolderDialogOpen, setIsCreateFolderDialogOpen] =
		createSignal(false)
	/**
	 * @type {[import("solid-js").Accessor<import("../../api").FSElement | null>, any]}
	 */
	const [viewerFile, setViewerFile] = createSignal(null)
	/** @type {[import('solid-js').Accessor<'browse'|'trash'|'favorites'|'recent'|'shared'>, any]} */
	const [listMode, setListMode] = createSignal('browse')
	const [mobileNavOpen, setMobileNavOpen] = createSignal(false)
	const [trashPath, setTrashPath] = createSignal('')
	const [emptyTrashOpen, setEmptyTrashOpen] = createSignal(false)
	const [restoreConflictPath, setRestoreConflictPath] = createSignal(null)
	/**
	 * Remaining trash items to restore after resolving a path conflict.
	 * @type {[import('solid-js').Accessor<{ path: string, name: string }[]>, any]}
	 */
	const [restoreRemaining, setRestoreRemaining] = createSignal([])
	/** @type {[import('solid-js').Accessor<Record<string, boolean>>, any]} */
	const [favoritePaths, setFavoritePaths] = createSignal({})

	/** @type {[import('solid-js').Accessor<'name'|'size'|'mtime'|'type'>, any]} */
	const [sortField, setSortField] = createSignal('name')
	/** @type {[import('solid-js').Accessor<'asc'|'desc'>, any]} */
	const [sortDir, setSortDir] = createSignal('asc')
	const [sortMenuAnchor, setSortMenuAnchor] = createSignal(null)
	/** @type {[import('solid-js').Accessor<'tiles'|'list'>, any]} */
	const [viewMode, setViewMode] = createSignal(readStoredViewMode())
	const [newFabAnchor, setNewFabAnchor] = createSignal(null)
	const [ptrPull, setPtrPull] = createSignal(0)
	const [ptrRefreshing, setPtrRefreshing] = createSignal(false)
	const [filesCanvas, setFilesCanvas] = createSignal(null)

	let viewerHistoryPushed = false
	let ignoringPopstate = false
	let viewerFolderPath = ''

	const setAndPersistViewMode = (mode) => {
		setViewMode(mode)
		try {
			localStorage.setItem(VIEW_MODE_KEY, mode)
		} catch {
			/* ignore */
		}
	}

	/**
	 * @type {[import('solid-js').Accessor<null | { mode: 'copy'|'move', items: { path: string, name: string }[] }>, any]}
	 */
	const [folderPicker, setFolderPicker] = createSignal(null)
	/**
	 * Internal file clipboard for Ctrl+C / Ctrl+X / Ctrl+V.
	 * @type {[import('solid-js').Accessor<null | { mode: 'copy'|'cut', items: { path: string, name: string }[] }>, any]}
	 */
	const [fileClipboard, setFileClipboard] = createSignal(null)
	/**
	 * Pending copy/move waiting on conflict resolution (may include remaining queue).
	 * @type {[import('solid-js').Accessor<null | { mode: 'copy'|'move', path: string, destination: string, name: string, remaining: { path: string, name: string }[] }>, any]}
	 */
	const [pathConflict, setPathConflict] = createSignal(null)
	/** @type {[import('solid-js').Accessor<Record<string, true>>, any]} */
	const [selectedPaths, setSelectedPaths] = createSignal({})
	const [bulkDeleteOpen, setBulkDeleteOpen] = createSignal(false)
	const [dropTargetPath, setDropTargetPath] = createSignal(null)
	const [canvasDropActive, setCanvasDropActive] = createSignal(false)
	/** Breadcrumb drop highlight: destination folder path ('' = root). */
	const [crumbDropPath, setCrumbDropPath] = createSignal(null)
	/**
	 * Marquee rectangle in canvas-local CSS pixels.
	 * @type {[import('solid-js').Accessor<null | { left: number, top: number, width: number, height: number }>, any]}
	 */
	const [marqueeBox, setMarqueeBox] = createSignal(null)

	/** @type {string | null} */
	let selectionAnchor = null
	/** @type {HTMLDivElement | undefined} */
	let filesCanvasEl
	/** @type {null | { x0: number, y0: number, additive: boolean, moved: boolean }} */
	let marqueeGesture = null

	const params = useParams()
	const navigate = useNavigate()
	const basePath = `/storages/${params.id}/files`

	/**
	 * Warm a photo's preview on hover / press-down, before the viewer mounts.
	 * The viewer's own loader shares the in-flight request, so this only ever
	 * moves the download earlier — it never causes a second one.
	 * @param {import("../../api").FSElement} el
	 */
	const prefetchViewerPreview = (el) => {
		if (!el?.is_file || !params.id) return
		if (fileKind(el.name, el.is_file) !== 'image') return
		warmPreview({
			scope: params.id,
			path: el.path,
			resolveUrl: () => API.files.getPreviewUrl(params.id, el.path),
			native: isNativeClient(),
		})
	}

	let uploadFileInputElement
	/** @type {HTMLInputElement} */
	let uploadFolderInputElement

	const trashMode = () => listMode() === 'trash'
	const flatMode = () =>
		listMode() === 'favorites' || listMode() === 'recent'
	const sharedMode = () => listMode() === 'shared'
	const browseMode = () => listMode() === 'browse'
	const selectionModeEnabled = () =>
		browseMode() ||
		trashMode() ||
		listMode() === 'favorites' ||
		listMode() === 'recent'

	const selectedCount = createMemo(
		() => Object.keys(selectedPaths()).length,
	)
	const selectionActive = () => selectedCount() > 0

	const clearSelection = () => {
		setSelectedPaths({})
		selectionAnchor = null
	}

	const selectablePathList = () =>
		sortedFsLayer()
			.filter((el) => el.name !== '..')
			.map((el) => itemNormalizedPath(el))

	const toggleSelectPath = (path) => {
		if (!path) return
		setSelectedPaths((prev) => {
			const next = { ...prev }
			if (next[path]) delete next[path]
			else next[path] = true
			return next
		})
		selectionAnchor = path
	}

	/**
	 * @param {string} path
	 * @param {{ ctrlKey?: boolean, metaKey?: boolean, shiftKey?: boolean }} event
	 */
	const selectPathWithModifiers = (path, event) => {
		if (!path || !selectionModeEnabled()) return
		const paths = selectablePathList()
		const idx = paths.indexOf(path)
		if (idx < 0) return

		const ctrl = Boolean(event.ctrlKey || event.metaKey)
		const shift = Boolean(event.shiftKey)

		if (shift && selectionAnchor != null) {
			const anchorIdx = paths.indexOf(selectionAnchor)
			if (anchorIdx >= 0) {
				const lo = Math.min(anchorIdx, idx)
				const hi = Math.max(anchorIdx, idx)
				const range = paths.slice(lo, hi + 1)
				if (ctrl) {
					setSelectedPaths((prev) => {
						const next = { ...prev }
						for (const p of range) next[p] = true
						return next
					})
				} else {
					setSelectedPaths(
						Object.fromEntries(range.map((p) => [p, true])),
					)
				}
				return
			}
		}

		if (ctrl) {
			toggleSelectPath(path)
			return
		}

		setSelectedPaths({ [path]: true })
		selectionAnchor = path
	}

	/**
	 * @param {import('../../api').FSElement} el
	 * @param {MouseEvent | KeyboardEvent} event
	 */
	const onSelectItem = (el, event) => {
		selectPathWithModifiers(itemNormalizedPath(el), event)
	}

	const selectAllVisible = () => {
		if (!selectionModeEnabled()) return
		const paths = selectablePathList()
		setSelectedPaths(Object.fromEntries(paths.map((p) => [p, true])))
		if (paths.length) selectionAnchor = paths[paths.length - 1]
	}

	/**
	 * @param {string[]} paths
	 * @param {boolean} additive
	 */
	const applyMarqueeSelection = (paths, additive) => {
		if (additive) {
			setSelectedPaths((prev) => {
				const next = { ...prev }
				for (const p of paths) next[p] = true
				return next
			})
		} else {
			setSelectedPaths(
				Object.fromEntries(paths.map((p) => [p, true])),
			)
		}
		if (paths.length) selectionAnchor = paths[paths.length - 1]
	}

	const pathsIntersectingClientRect = (rect) => {
		if (!filesCanvasEl) return []
		const nodes = filesCanvasEl.querySelectorAll('[data-fs-path]')
		/** @type {string[]} */
		const hit = []
		for (const node of nodes) {
			const path = node.getAttribute('data-fs-path')
			if (!path) continue
			const r = node.getBoundingClientRect()
			const overlaps =
				r.left < rect.right &&
				r.right > rect.left &&
				r.top < rect.bottom &&
				r.bottom > rect.top
			if (overlaps) hit.push(path)
		}
		return hit
	}

	/**
	 * @param {MouseEvent} event
	 */
	const onCanvasMouseDown = (event) => {
		if (!selectionModeEnabled() || event.button !== 0) return
		const target = /** @type {HTMLElement} */ (event.target)
		if (
			target.closest?.(
				'.fs-grid-item, .fs-list-item, button, a, input, textarea, .MuiIconButton-root',
			)
		) {
			return
		}
		if (!filesCanvasEl) return

		// Kill native browser text selection during marquee drag.
		event.preventDefault()
		window.getSelection?.()?.removeAllRanges?.()

		const canvasRect = filesCanvasEl.getBoundingClientRect()
		const additive = Boolean(event.ctrlKey || event.metaKey)
		/** Snapshot of selection when starting an additive marquee. */
		const baseSelection = additive ? { ...selectedPaths() } : {}

		marqueeGesture = {
			x0: event.clientX,
			y0: event.clientY,
			additive,
			moved: false,
		}
		setMarqueeBox({
			left: event.clientX - canvasRect.left + filesCanvasEl.scrollLeft,
			top: event.clientY - canvasRect.top + filesCanvasEl.scrollTop,
			width: 0,
			height: 0,
		})

		if (!additive) {
			clearSelection()
		}

		const applyLiveHit = (clientX, clientY) => {
			const clientRect = {
				left: Math.min(marqueeGesture.x0, clientX),
				top: Math.min(marqueeGesture.y0, clientY),
				right: Math.max(marqueeGesture.x0, clientX),
				bottom: Math.max(marqueeGesture.y0, clientY),
			}
			const hit = pathsIntersectingClientRect(clientRect)
			if (additive) {
				const next = { ...baseSelection }
				for (const p of hit) next[p] = true
				setSelectedPaths(next)
				if (hit.length) selectionAnchor = hit[hit.length - 1]
			} else {
				applyMarqueeSelection(hit, false)
			}
		}

		// Hit-testing walks every visible tile's getBoundingClientRect() — too
		// expensive to run on every raw mousemove in a large (photo) folder.
		// Batch it to at most once per animation frame.
		const hitTestBatcher = createRafBatcher((x, y) => {
			if (marqueeGesture) applyLiveHit(x, y)
		})

		const onMove = (ev) => {
			if (!marqueeGesture || !filesCanvasEl) return
			ev.preventDefault()
			window.getSelection?.()?.removeAllRanges?.()
			const dx = ev.clientX - marqueeGesture.x0
			const dy = ev.clientY - marqueeGesture.y0
			if (Math.abs(dx) > 3 || Math.abs(dy) > 3) {
				marqueeGesture.moved = true
			}
			const cr = filesCanvasEl.getBoundingClientRect()
			const left =
				Math.min(marqueeGesture.x0, ev.clientX) -
				cr.left +
				filesCanvasEl.scrollLeft
			const top =
				Math.min(marqueeGesture.y0, ev.clientY) -
				cr.top +
				filesCanvasEl.scrollTop
			const width = Math.abs(ev.clientX - marqueeGesture.x0)
			const height = Math.abs(ev.clientY - marqueeGesture.y0)
			setMarqueeBox({ left, top, width, height })

			if (marqueeGesture.moved) {
				hitTestBatcher.schedule(ev.clientX, ev.clientY)
			}
		}

		const onUp = (ev) => {
			window.removeEventListener('mousemove', onMove)
			window.removeEventListener('mouseup', onUp)
			hitTestBatcher.cancel()
			const gesture = marqueeGesture
			marqueeGesture = null
			setMarqueeBox(null)
			if (!gesture) return

			if (!gesture.moved) {
				// Click on empty canvas — already cleared if non-additive.
				return
			}

			// Final pass so release position is included.
			applyLiveHit(ev.clientX, ev.clientY)
		}

		window.addEventListener('mousemove', onMove)
		window.addEventListener('mouseup', onUp)
	}

	const selectedItems = () => {
		const map = selectedPaths()
		return sortedFsLayer().filter(
			(el) => el.name !== '..' && map[itemNormalizedPath(el)],
		)
	}

	createEffect(() => {
		listMode()
		params.path
		clearSelection()
		setMarqueeBox(null)
		marqueeGesture = null
		setRestoreRemaining([])
	})

	/** Decoded current folder from the `*path` splat (router keeps it encoded). */
	const routerFolderPath = () => decodeStoragePath(params.path || '')

	/** Monotonic id so a slow list response cannot overwrite a newer folder. */
	let fsLayerFetchGen = 0
	/** Last path we painted — skip empty flash on same-folder refresh. */
	let fsLayerPaintedPath = null

	const sortedFsLayer = createMemo(() => {
		const items = fsLayer().filter((el) => el.name !== '..')
		// Favorites / Recent keep API order (starred newest / viewed_at desc).
		if (flatMode()) return items
		return sortFsElements(items, sortField(), sortDir())
	})

	/**
	 * Breadcrumb segments for browse mode.
	 * Labels are decoded for display; `path` is decoded for API / drag-drop.
	 * @returns {{ label: string, path: string }[]}
	 */
	const pathCrumbs = createMemo(() => {
		const raw = routerFolderPath()
			.replace(/^\/+/, '')
			.replace(/\/+$/, '')
		/** @type {{ label: string, path: string }[]} */
		const crumbs = [{ label: 'All files', path: '' }]
		if (!raw) return crumbs
		let acc = ''
		for (const part of raw.split('/').filter(Boolean)) {
			acc = acc ? `${acc}/${part}` : part
			crumbs.push({ label: part, path: acc })
		}
		return crumbs
	})

	const goToFolder = (folderPath) => {
		const dest = folderDestPath(folderPath)
		if (dest) {
			navigate(
				`/storages/${params.id}/files/${encodeStoragePath(dest)}`,
			)
		} else {
			navigate(`/storages/${params.id}/files`)
		}
	}

	const syncFavoritePaths = (items) => {
		const map = {}
		for (const el of items || []) {
			if (el?.is_file && el.path) map[el.path] = true
		}
		setFavoritePaths(map)
	}

	const loadFavoritePaths = async () => {
		try {
			const items = await API.files.listFavorites(params.id, { quiet: true })
			syncFavoritePaths(items)
			return items
		} catch {
			/* backend may not expose favorites yet — silent on browse load */
			return null
		}
	}

	const fetchStorage = async () => {
		const storage = await API.storages.getStorage(params.id)
		chrome.setStorageName(storage.name)
	}

	const fetchFSLayer = async (path = routerFolderPath()) => {
		// Abort thumbs first so list HTTP is not stuck behind saturated connections.
		clearThumbQueue()
		const gen = ++fsLayerFetchGen
		const pathAtCall = path
		const pathChanged = fsLayerPaintedPath !== pathAtCall
		const cacheKey = fsLayerCacheKey(params.id, pathAtCall)
		if (pathChanged) {
			// First paint this session (e.g. a full page reload): repaint the last-known
			// snapshot instead of blanking, so the list does not flash empty while the
			// fresh fetch is in flight.
			setFsLayer(fsLayerPaintedPath === null ? readFsLayerCache(cacheKey) || [] : [])
		}
		try {
			const fsLayerRes = await API.files.getFSLayer(params.id, path)
			if (gen !== fsLayerFetchGen) {
				return
			}
			fsLayerPaintedPath = pathAtCall
			const items = (fsLayerRes || []).filter((el) => el.name !== '..')
			setFsLayer(items)
			writeFsLayerCache(cacheKey, items)
			chrome.setIsSearching(false)
			chrome.setSearchQuery('')
		} catch (err) {
			if (gen !== fsLayerFetchGen) return
			throw err
		}
	}

	/** Keep browse listing tied to the router folder (browser back included). */
	createEffect(() => {
		const mode = listMode()
		const path = routerFolderPath()
		if (mode !== 'browse') return
		if (chrome.isSearching()) return
		void fetchFSLayer(path)
	})

	const fetchTrashLayer = async (path = trashPath()) => {
		const fsLayerRes = await API.files.listTrash(params.id, path)
		setFsLayer((fsLayerRes || []).filter((el) => el.name !== '..'))
		chrome.setIsSearching(false)
		chrome.setSearchQuery('')
	}

	const fetchFavorites = async () => {
		const items = await API.files.listFavorites(params.id)
		setFsLayer(items || [])
		syncFavoritePaths(items)
		chrome.setIsSearching(false)
		chrome.setSearchQuery('')
	}

	const fetchRecent = async () => {
		const items = await API.files.listRecent(params.id)
		setFsLayer(items || [])
		chrome.setIsSearching(false)
		chrome.setSearchQuery('')
	}

	const refreshCurrent = async () => {
		const mode = listMode()
		if (mode === 'trash') {
			await fetchTrashLayer()
		} else if (mode === 'favorites') {
			await fetchFavorites()
		} else if (mode === 'recent') {
			await fetchRecent()
		} else if (mode === 'shared') {
			/* SharedLinksPanel reloads when active. */
		} else {
			await fetchFSLayer()
		}
	}

	createEffect(() => {
		const el = filesCanvas()
		if (!el) return
		const detach = attachPullToRefresh(el, {
			isEnabled: () => window.matchMedia('(max-width: 840px)').matches,
			onPullChange: setPtrPull,
			onRefreshingChange: setPtrRefreshing,
			onRefresh: () => refreshCurrent(),
		})
		onCleanup(detach)
	})

	const enterTrash = async () => {
		setListMode('trash')
		setTrashPath('')
		setViewerFile(null)
		await fetchTrashLayer('')
	}

	const enterFavorites = async () => {
		setListMode('favorites')
		setViewerFile(null)
		try {
			await fetchFavorites()
		} catch {
			setListMode('browse')
			await fetchFSLayer()
		}
	}

	const enterRecent = async () => {
		setListMode('recent')
		setViewerFile(null)
		try {
			await fetchRecent()
		} catch {
			setListMode('browse')
			await fetchFSLayer()
		}
	}

	const enterShared = () => {
		setListMode('shared')
		setTrashPath('')
		setViewerFile(null)
		chrome.setIsSearching(false)
		chrome.setSearchQuery('')
	}

	const exitSpecialMode = async () => {
		setListMode('browse')
		setTrashPath('')
		await fetchFSLayer()
	}

	const onSelectMode = async (mode) => {
		if (mode === 'browse') {
			await exitSpecialMode()
			return
		}
		if (mode === 'favorites') return enterFavorites()
		if (mode === 'recent') return enterRecent()
		if (mode === 'trash') return enterTrash()
		if (mode === 'shared') return enterShared()
	}

	const onTrashNavigate = async (el) => {
		if (el.name === '..') {
			setTrashPath(el.path)
			await fetchTrashLayer(el.path)
			return
		}
		if (!el.is_file) {
			setTrashPath(el.path)
			await fetchTrashLayer(el.path)
		}
	}

	const trashItemPath = (el) => itemNormalizedPath(el)

	const isFavorite = (el) =>
		Boolean(el?.path && (favoritePaths()[el.path] || el.is_favorite))

	const toggleFavorite = async (el) => {
		if (!el?.is_file || !el.path) return
		const starred = isFavorite(el)
		try {
			if (starred) {
				await API.files.removeFavorite(params.id, el.path)
				setFavoritePaths((prev) => {
					const next = { ...prev }
					delete next[el.path]
					return next
				})
				addAlert(`Removed "${el.name}" from favorites`, 'success')
				if (listMode() === 'favorites') {
					setFsLayer((prev) => prev.filter((f) => f.path !== el.path))
				}
			} else {
				await API.files.addFavorite(params.id, el.path)
				setFavoritePaths((prev) => ({ ...prev, [el.path]: true }))
				addAlert(`Added "${el.name}" to favorites`, 'success')
			}
		} catch {
			/* alerted by API */
		}
	}

	/**
	 * @param {{ path: string, name: string }[]} items
	 * @param {'replace'|'rename'} [onConflict]
	 */
	const restoreItems = async (items, onConflict) => {
		if (!items.length) return
		/** @type {string[]} */
		const done = []

		for (let i = 0; i < items.length; i++) {
			const item = items[i]
			try {
				await API.files.restoreTrash(params.id, item.path, onConflict)
				done.push(item.name)
			} catch (err) {
				if (err.status === 409 && !onConflict) {
					setRestoreConflictPath(item.path)
					setRestoreRemaining(items.slice(i + 1))
					return
				}
				/* alerted by API — continue remaining */
			}
		}

		setRestoreConflictPath(null)
		setRestoreRemaining([])
		clearSelection()
		if (done.length === 1) {
			addAlert(`Restored "${done[0]}"`, 'success')
		} else if (done.length > 1) {
			addAlert(`Restored ${done.length} items`, 'success')
		}
		await fetchTrashLayer()
	}

	const restoreItem = async (el, onConflict) => {
		await restoreItems(
			[{ path: trashItemPath(el), name: el.name }],
			onConflict,
		)
	}

	const deleteForeverItem = async (el) => {
		await API.files.deleteForever(params.id, trashItemPath(el))
		addAlert(`Permanently deleted "${el.name}"`, 'success')
		await fetchTrashLayer()
	}

	const confirmBulkRestore = async () => {
		const items = selectedItems().map((el) => ({
			path: trashItemPath(el),
			name: el.name,
		}))
		await restoreItems(items)
	}

	const confirmEmptyTrash = async () => {
		setEmptyTrashOpen(false)
		// Clear immediately so the list does not stay stale while the API
		// finishes DB purge (+ background Telegram GC).
		setTrashPath('')
		setFsLayer([])
		clearSelection()
		try {
			await API.files.emptyTrash(params.id)
			addAlert('Trash emptied', 'success')
		} catch {
			/* apiRequest already alerted */
			await fetchTrashLayer('')
		}
	}

	/** Snapshot of selected items for bulk copy/move. */
	const selectedTransferItems = () =>
		selectedItems().map((el) => ({
			path: itemNormalizedPath(el),
			name: el.name,
		}))

	/**
	 * Prefer the whole selection when the context-menu target is selected.
	 * @param {import('../../api').FSElement} el
	 * @param {'copy'|'move'} mode
	 */
	const openTransferForItem = (el, mode) => {
		const path = itemNormalizedPath(el)
		const selected = selectedTransferItems()
		const items =
			selected.length > 1 && selected.some((item) => item.path === path)
				? selected
				: [{ path, name: el.name }]
		setFolderPicker({ mode, items })
	}

	const openCopyTo = (el) => openTransferForItem(el, 'copy')
	const openMoveTo = (el) => openTransferForItem(el, 'move')

	const openBulkCopy = () => {
		const items = selectedTransferItems()
		if (!items.length) return
		setFolderPicker({ mode: 'copy', items })
	}

	const openBulkMove = () => {
		const items = selectedTransferItems()
		if (!items.length) return
		setFolderPicker({ mode: 'move', items })
	}

	/**
	 * Copy or cut current selection into the in-app clipboard.
	 * @param {'copy'|'cut'} mode
	 */
	const clipboardCapture = (mode) => {
		if (trashMode() || sharedMode()) return
		const items = selectedTransferItems()
		if (!items.length) return
		setFileClipboard({ mode, items })
		addAlert(
			mode === 'copy'
				? items.length === 1
					? `Copied "${items[0].name}"`
					: `Copied ${items.length} items`
				: items.length === 1
					? `Cut "${items[0].name}"`
					: `Cut ${items.length} items`,
			'info',
		)
	}

	/** Paste clipboard into the current browse folder. */
	const clipboardPaste = async () => {
		if (!browseMode()) return
		const clip = fileClipboard()
		if (!clip?.items?.length) return
		const mode = clip.mode === 'cut' ? 'move' : 'copy'
		const items = clip.items.map((item) => ({ ...item }))
		await transferItems(mode, items, routerFolderPath())
	}

	/**
	 * Prompt and rename a file/folder. Used by context menu and F2.
	 * @param {import('../../api').FSElement} el
	 */
	const renameItem = async (el) => {
		if (!el || el.name === '..' || trashMode()) return
		const path = itemNormalizedPath(el)
		const currentName = el.is_file
			? el.name.includes('/')
				? el.name.split('/').pop()
				: el.name
			: el.name.replace(/\/$/, '').split('/').pop() || el.name
		const newName = window.prompt('New name', currentName)
		if (!newName || newName === currentName) return
		try {
			await API.files.rename(params.id, path, newName)
			addAlert(`Renamed to "${newName}"`, 'success')
			clearSelection()
			await refreshCurrent()
		} catch {
			/* alerted by API */
		}
	}

	/** F2: rename the sole selected item (or the selection anchor). */
	const renameSelected = async () => {
		if (trashMode() || sharedMode()) return
		const items = selectedItems()
		if (!items.length) return
		let el = items[0]
		if (items.length > 1 && selectionAnchor != null) {
			const anchored = items.find(
				(item) => itemNormalizedPath(item) === selectionAnchor,
			)
			if (anchored) el = anchored
		}
		await renameItem(el)
	}

	/**
	 * Right-click: select the item if it is not already in the selection.
	 * @param {import('../../api').FSElement} el
	 */
	const onContextMenuItem = (el) => {
		if (!selectionModeEnabled()) return
		const path = itemNormalizedPath(el)
		if (!path) return
		if (!selectedPaths()[path]) {
			setSelectedPaths({ [path]: true })
			selectionAnchor = path
		}
	}

	/**
	 * @param {'copy'|'move'} mode
	 * @param {{ path: string, name: string }[]} items
	 * @param {string} destination
	 * @param {'replace'|'rename'} [onConflict]
	 */
	const transferItems = async (mode, items, destination, onConflict) => {
		if (!items.length) return
		const apiCall =
			mode === 'copy' ? API.files.copyFile : API.files.moveFile
		const dest = folderDestPath(destination)
		/** @type {string[]} */
		const done = []

		for (let i = 0; i < items.length; i++) {
			const item = items[i]
			if (mode === 'move' && isBlockedDest(item.path, dest)) {
				addAlert(
					`Cannot move "${item.name}" into itself or a subfolder`,
					'error',
				)
				continue
			}
			try {
				// Apply conflict strategy to every item in the batch — not only the first.
				await apiCall(params.id, item.path, dest, onConflict)
				done.push(item.name)
			} catch (err) {
				if (err.status === 409 && !onConflict) {
					setFolderPicker(null)
					setPathConflict({
						mode,
						path: item.path,
						destination: dest,
						name: item.name,
						remaining: items.slice(i + 1),
					})
					return
				}
				/* alerted by API helper — continue remaining */
			}
		}

		setFolderPicker(null)
		setPathConflict(null)
		clearSelection()

		// Cut clipboard is spent after a successful move paste / move transfer.
		const clip = fileClipboard()
		if (
			mode === 'move' &&
			clip?.mode === 'cut' &&
			items.some((item) => clip.items.some((c) => c.path === item.path))
		) {
			setFileClipboard(null)
		}

		if (done.length === 1) {
			addAlert(
				mode === 'copy'
					? `Copied "${done[0]}"`
					: `Moved "${done[0]}"`,
				'success',
			)
		} else if (done.length > 1) {
			addAlert(
				mode === 'copy'
					? `Copied ${done.length} items`
					: `Moved ${done.length} items`,
				'success',
			)
		}

		await refreshCurrent()
	}

	/**
	 * @param {string} destination
	 * @param {'replace'|'rename'} [onConflict]
	 */
	const runTransfer = async (destination, onConflict) => {
		const pending = pathConflict()
		if (pending) {
			const current = {
				path: pending.path,
				name: pending.name,
			}
			const rest = pending.remaining || []
			await transferItems(
				pending.mode,
				[current, ...rest],
				pending.destination,
				onConflict,
			)
			return
		}
		const picker = folderPicker()
		if (!picker?.items?.length) return
		// Snapshot so the async loop is not tied to dialog state.
		const items = picker.items.map((item) => ({ ...item }))
		await transferItems(picker.mode, items, destination, onConflict)
	}

	const confirmBulkDelete = async () => {
		setBulkDeleteOpen(false)
		const items = selectedItems()
		/** @type {typeof items} */
		const succeeded = []
		for (const el of items) {
			try {
				if (trashMode()) {
					await API.files.deleteForever(params.id, trashItemPath(el))
				} else {
					await API.files.deleteFile(params.id, itemNormalizedPath(el))
				}
				succeeded.push(el)
			} catch {
				/* alerted */
			}
		}
		if (succeeded.length) {
			addAlert(
				formatBulkDeleteMessage(
					succeeded.length,
					succeeded[0].name,
					trashMode(),
				),
				'success',
			)
		}
		clearSelection()
		await refreshCurrent()
	}

	/**
	 * @param {string} query
	 */
	const runSearch = async (query) => {
		if (!browseMode()) {
			return
		}
		const q = query.trim()
		if (!q) {
			await fetchFSLayer()
			return
		}

		const results = await API.files.search(params.id, routerFolderPath(), q)
		const mapped = results.map((el) => ({
			path: el.path,
			name: el.path,
			is_file: el.is_file,
			size: 0,
			has_thumb: false,
		}))
		setFsLayer(mapped)
		chrome.setIsSearching(true)
	}

	const clearSearch = async () => {
		chrome.setSearchQuery('')
		chrome.setIsSearching(false)
		await fetchFSLayer()
	}

	const reload = async () => {
		if (window.location.pathname.startsWith(basePath)) {
			const path = window.location.pathname
				.slice(basePath.length)
				.replace(/^\/+/, '')
			await fetchFSLayer(decodeStoragePath(path))
		}
	}

	createEffect(() => {
		if (viewerFile() && !viewerHistoryPushed) {
			viewerFolderPath = routerFolderPath()
			pushViewerHistory(window.history, window.location.href)
			viewerHistoryPushed = true
			return
		}
		if (!viewerFile() && viewerHistoryPushed) {
			ignoringPopstate = true
			viewerHistoryPushed = false
			history.back()
		}
	})

	const closeViewer = () => {
		setViewerFile(null)
	}

	const onPopState = async (event) => {
		if (ignoringPopstate) {
			ignoringPopstate = false
			return
		}
		if (
			shouldCloseViewerOnPopstate({
				viewerOpen: Boolean(viewerFile()),
				state: event.state,
			})
		) {
			viewerHistoryPushed = false
			setViewerFile(null)
			const currentFolderPath = folderDestPath(viewerFolderPath)
			const currentFolderLocation = currentFolderPath
				? `${basePath}/${encodeStoragePath(currentFolderPath)}`
				: basePath
			if (window.location.pathname !== currentFolderLocation) {
				await reload()
			}
			return
		}
		await reload()
	}

	onMount(() => {
		chrome.activate({
			storageId: params.id,
			storageName: '',
			onSearch: runSearch,
			onClear: clearSearch,
		})
		Promise.all([fetchStorage(), fetchFSLayer(), loadFavoritePaths()]).then()
		window.addEventListener('popstate', onPopState, false)

		const mobileMediaQuery = window.matchMedia('(max-width: 840px)')
		const closeMobileNavOnDesktop = (event) => {
			if (!event.matches) setMobileNavOpen(false)
		}
		mobileMediaQuery.addEventListener('change', closeMobileNavOnDesktop)

		onCleanup(() => {
			mobileMediaQuery.removeEventListener('change', closeMobileNavOnDesktop)
		})
	})

	/**
	 * File-manager shortcuts. Use KeyboardEvent.code so they work on any
	 * layout (e.g. Russian: physical A/C/X/V still match KeyA/KeyC/…).
	 * @param {KeyboardEvent} e
	 */
	const onFilesKeyDown = (e) => {
		const target = /** @type {HTMLElement | null} */ (e.target)
		if (
			target?.closest?.(
				'input, textarea, select, [contenteditable="true"]',
			)
		) {
			return
		}
		// Ignore while modal dialogs own the page.
		if (
			target?.closest?.(
				'.MuiDialog-root, .MuiModal-root, [role="dialog"]',
			)
		) {
			return
		}

		if (e.key === 'Escape' && selectionActive()) {
			e.preventDefault()
			clearSelection()
			return
		}

		// F2 — rename (no Ctrl/meta required)
		if (e.code === 'F2' && !e.ctrlKey && !e.metaKey && !e.altKey) {
			if (trashMode() || sharedMode() || !selectionActive()) return
			e.preventDefault()
			renameSelected()
			return
		}

		const mod = e.ctrlKey || e.metaKey
		if (!mod) return

		switch (e.code) {
			case 'KeyA':
				if (!selectionModeEnabled()) return
				e.preventDefault()
				selectAllVisible()
				return
			case 'KeyC':
				if (!selectionModeEnabled() || trashMode()) return
				e.preventDefault()
				clipboardCapture('copy')
				return
			case 'KeyX':
				if (!browseMode()) return
				e.preventDefault()
				clipboardCapture('cut')
				return
			case 'KeyV':
				if (!browseMode()) return
				e.preventDefault()
				clipboardPaste()
				return
			default:
				break
		}
	}

	onMount(() => {
		window.addEventListener('keydown', onFilesKeyDown, true)

		/** Debounced so rapid consecutive completes share one list refetch. */
		let refreshTimer = 0
		const scheduleListRefreshAfterUpload = () => {
			window.clearTimeout(refreshTimer)
			refreshTimer = window.setTimeout(() => {
				// Keep folder/list-mode context: only refresh views that show uploads.
				if (browseMode()) {
					void fetchFSLayer()
				} else if (listMode() === 'recent') {
					void fetchRecent()
				}
			}, 300)
		}

		// Per successful upload (while others may still be queued/uploading).
		const unsubItemDone = uploadQueueStore.onItemDone(() => {
			scheduleListRefreshAfterUpload()
		})
		// Full queue idle — safety net if a done event was missed.
		const unsubIdle = uploadQueueStore.onIdle(() => {
			scheduleListRefreshAfterUpload()
		})

		// Native/background auto-upload (Camera sync on mobile clients) runs
		// entirely on the Rust side and never touches uploadQueueStore — there
		// is no native→web push channel for it. Refresh on return to the app
		// so newly auto-uploaded files show up without a manual pull-to-refresh.
		const onVisibilityOrFocus = () => {
			if (!shouldRefreshOnVisibilityEvent(document.visibilityState)) return
			scheduleListRefreshAfterUpload()
		}
		document.addEventListener('visibilitychange', onVisibilityOrFocus)
		window.addEventListener('focus', onVisibilityOrFocus)

		// Silent background refresh: another device (or the native Camera
		// auto-upload) can change this listing at any time, and there is no push
		// channel from the server. The jittered 5-15s cadence keeps the view
		// current without a spinner and without every client polling in lockstep.
		const stopAutoRefresh = startAutoRefresh({
			run: () => refreshCurrent(),
			// Refreshing under the user's hands is worse than being slightly
			// stale: it would drop a selection, close a menu, or swap the list
			// out from under a drag. Uploads already schedule their own refresh.
			isPaused: () =>
				uploadQueueStore.hasActiveWork() ||
				ptrRefreshing() ||
				Boolean(viewerFile()) ||
				Boolean(marqueeBox()) ||
				Boolean(dropTargetPath()) ||
				canvasDropActive() ||
				Boolean(crumbDropPath()) ||
				Boolean(sortMenuAnchor()) ||
				Boolean(newFabAnchor()) ||
				Boolean(folderPicker()) ||
				Boolean(pathConflict()) ||
				Boolean(restoreConflictPath()) ||
				bulkDeleteOpen() ||
				emptyTrashOpen() ||
				Object.keys(selectedPaths()).length > 0,
		})

		onCleanup(() => {
			window.clearTimeout(refreshTimer)
			unsubItemDone()
			unsubIdle()
			document.removeEventListener('visibilitychange', onVisibilityOrFocus)
			window.removeEventListener('focus', onVisibilityOrFocus)
			stopAutoRefresh()
		})
	})
	onCleanup(() => {
		window.removeEventListener('keydown', onFilesKeyDown, true)
	})

	onCleanup(() => {
		window.removeEventListener('popstate', onPopState, false)
		chrome.deactivate()
	})

	useBeforeLeave(async (e) => {
		if (e.to.startsWith(basePath)) {
			// Drop in-flight thumbs immediately; browse listing is loaded by the
			// params.path effect once the router commits the new folder.
			clearThumbQueue()
		}
	})

	const openCreateFolderDialog = () => {
		setIsCreateFolderDialogOpen(true)
	}
	const closeCreateFolderDialog = () => {
		setIsCreateFolderDialogOpen(false)
	}

	/**
	 * @param {string} folderName
	 */
	const createFolder = async (folderName) => {
		const folderBase = folderDestPath(routerFolderPath())

		await API.files.createFolder(params.id, folderBase, folderName)
		addAlert(`Created folder "${folderName}"`, 'success')
		await fetchFSLayer()
	}

	const uploadFileClickHandler = () => {
		uploadFileInputElement.click()
	}

	const uploadFolderClickHandler = () => {
		uploadFolderInputElement.click()
	}

	/**
	 * Enqueue files into the Drive-style upload manager (pipeline: 1 spool + 1 telegram, smaller first).
	 * @param {{ file: File, relativePath: string }[]} entries
	 * @param {string} baseParentPath Destination folder without trailing /
	 */
	const uploadEntries = (entries, baseParentPath) => {
		if (!entries.length) {
			addAlert('No files to upload', 'error')
			return
		}
		const currentPath = folderDestPath(baseParentPath)
		uploadQueueStore.enqueue(
			entries.map(({ file, relativePath }) => {
				const segments = relativePath.split('/')
				segments.pop()
				const parentPath = joinStoragePath(currentPath, ...segments)
				return {
					storageId: params.id,
					parentPath,
					file,
					name: relativePath,
				}
			}),
		)
	}

	/**
	 * @param {Event} event
	 */
	const uploadFile = (event) => {
		/** @type {File[]} */
		let files = Array.from(event.target.files || [])
		event.target.value = null
		if (!files.length) return

		if (files.length > MAX_FILE_PICKER) {
			addAlert(
				`You can upload up to ${MAX_FILE_PICKER} files at a time`,
				'warning',
			)
			files = files.slice(0, MAX_FILE_PICKER)
		}

		uploadEntries(
			files.map((file) => ({
				file,
				relativePath: file.name,
			})),
			routerFolderPath(),
		)
	}

	/**
	 * @param {Event} event
	 */
	const uploadFolder = async (event) => {
		/** @type {File[]} */
		const rawFiles = Array.from(event.target.files || [])
		event.target.value = null
		if (!rawFiles.length) return

		const files = rawFiles.filter((file) => {
			const rel = file.webkitRelativePath || file.name
			return !shouldSkipUploadEntry(rel)
		})
		if (!files.length) {
			addAlert('No files to upload in the selected folder', 'error')
			return
		}

		uploadEntries(
			files.map((file) => ({
				file,
				relativePath: file.webkitRelativePath || file.name,
			})),
			routerFolderPath(),
		)
	}

	const hasSarcaDrag = (dt) =>
		Boolean(dt?.types && [...dt.types].includes(SARCA_DRAG_MIME))

	const hasFileDrag = (dt) =>
		Boolean(dt?.types && [...dt.types].includes('Files'))

	/**
	 * @param {import('../../api').FSElement} el
	 * @param {DragEvent} event
	 */
	const onDragStartItem = (el, event) => {
		const path = itemNormalizedPath(el)
		const selected = selectedPaths()
		const paths = selected[path]
			? Object.keys(selected)
			: [path]
		event.dataTransfer?.setData(SARCA_DRAG_MIME, JSON.stringify(paths))
		if (event.dataTransfer) {
			event.dataTransfer.effectAllowed = 'move'
		}
	}

	/**
	 * @param {string} destFolder
	 * @param {string[]} paths
	 */
	const movePathsToFolder = async (destFolder, paths) => {
		const dest = folderDestPath(destFolder)
		const items = paths.map((path) => {
			const el = fsLayer().find((e) => itemNormalizedPath(e) === path)
			return {
				path,
				name: el?.name || path.split('/').filter(Boolean).pop() || path,
			}
		})
		await transferItems('move', items, dest)
	}

	/**
	 * @param {import('../../api').FSElement} folderEl
	 * @param {DragEvent} event
	 */
	const onDragOverFolder = (folderEl, event) => {
		if (!browseMode()) return
		const dt = event.dataTransfer
		if (!dt) return
		if (hasSarcaDrag(dt) || hasFileDrag(dt)) {
			event.preventDefault()
			event.stopPropagation()
			dt.dropEffect = hasSarcaDrag(dt) ? 'move' : 'copy'
			setDropTargetPath(itemNormalizedPath(folderEl))
		}
	}

	const onDragLeaveFolder = (folderEl, event) => {
		const related = /** @type {Node|null} */ (event.relatedTarget)
		const current = /** @type {Node|null} */ (event.currentTarget)
		if (related && current?.contains?.(related)) return
		if (dropTargetPath() === itemNormalizedPath(folderEl)) {
			setDropTargetPath(null)
		}
	}

	/**
	 * @param {import('../../api').FSElement} folderEl
	 * @param {DragEvent} event
	 */
	const onDropOnFolder = async (folderEl, event) => {
		if (!browseMode()) return
		event.preventDefault()
		event.stopPropagation()
		setDropTargetPath(null)
		setCanvasDropActive(false)
		const dest = folderDestPath(folderEl.path)
		const dt = event.dataTransfer
		if (!dt) return

		const raw = dt.getData(SARCA_DRAG_MIME)
		if (raw) {
			try {
				const paths = JSON.parse(raw)
				if (Array.isArray(paths) && paths.length) {
					await movePathsToFolder(dest, paths)
				}
			} catch {
				/* ignore */
			}
			return
		}

		if (hasFileDrag(dt)) {
			const entries = await filesFromDataTransfer(dt)
			await uploadEntries(entries, dest)
		}
	}

	const onCanvasDragOver = (event) => {
		if (!browseMode()) return
		const dt = event.dataTransfer
		if (!dt || hasSarcaDrag(dt) || !hasFileDrag(dt)) return
		event.preventDefault()
		dt.dropEffect = 'copy'
		setCanvasDropActive(true)
	}

	const onCanvasDragLeave = (event) => {
		const related = /** @type {Node|null} */ (event.relatedTarget)
		const current = /** @type {Node|null} */ (event.currentTarget)
		if (related && current?.contains?.(related)) return
		setCanvasDropActive(false)
	}

	const onCanvasDrop = async (event) => {
		if (!browseMode()) return
		const dt = event.dataTransfer
		if (!dt || hasSarcaDrag(dt) || !hasFileDrag(dt)) return
		event.preventDefault()
		setCanvasDropActive(false)
		setDropTargetPath(null)
		const entries = await filesFromDataTransfer(dt)
		await uploadEntries(entries, routerFolderPath())
	}

	/**
	 * @param {string} destPath
	 * @param {DragEvent} event
	 */
	const onCrumbDragOver = (destPath, event) => {
		if (!browseMode()) return
		const dt = event.dataTransfer
		if (!dt) return
		if (hasSarcaDrag(dt) || hasFileDrag(dt)) {
			event.preventDefault()
			event.stopPropagation()
			dt.dropEffect = hasSarcaDrag(dt) ? 'move' : 'copy'
			setCrumbDropPath(folderDestPath(destPath))
		}
	}

	/**
	 * @param {string} destPath
	 * @param {DragEvent} event
	 */
	const onCrumbDragLeave = (destPath, event) => {
		const related = /** @type {Node|null} */ (event.relatedTarget)
		const current = /** @type {Node|null} */ (event.currentTarget)
		if (related && current?.contains?.(related)) return
		if (crumbDropPath() === folderDestPath(destPath)) {
			setCrumbDropPath(null)
		}
	}

	/**
	 * @param {string} destPath
	 * @param {DragEvent} event
	 */
	const onCrumbDrop = async (destPath, event) => {
		if (!browseMode()) return
		event.preventDefault()
		event.stopPropagation()
		const dest = folderDestPath(destPath)
		setCrumbDropPath(null)
		setDropTargetPath(null)
		setCanvasDropActive(false)
		const dt = event.dataTransfer
		if (!dt) return

		const raw = dt.getData(SARCA_DRAG_MIME)
		if (raw) {
			try {
				const paths = JSON.parse(raw)
				if (Array.isArray(paths) && paths.length) {
					await movePathsToFolder(dest, paths)
				}
			} catch {
				/* ignore */
			}
			return
		}

		if (hasFileDrag(dt)) {
			const entries = await filesFromDataTransfer(dt)
			await uploadEntries(entries, dest)
		}
	}

	/**
	 * @param {'name'|'size'|'mtime'|'type'} field
	 */
	const chooseSortField = (field) => {
		if (sortField() === field) {
			setSortDir((d) => (d === 'asc' ? 'desc' : 'asc'))
		} else {
			setSortField(field)
			setSortDir('asc')
		}
		setSortMenuAnchor(null)
	}

	const conflictDialogOpen = () =>
		Boolean(restoreConflictPath()) || Boolean(pathConflict())

	const conflictPath = () =>
		pathConflict()?.path || restoreConflictPath() || ''

	const conflictMessage = () => {
		const pending = pathConflict()
		if (pending) {
			const verb = pending.mode === 'copy' ? 'copy' : 'move'
			return `A live file or folder already exists at the destination for “${pending.name}”. Replace it, ${verb} under a new name, or cancel?`
		}
		return undefined
	}

	const conflictRenameLabel = () =>
		pathConflict() ? 'Keep both' : 'Rename'

	return (
		<>
			<div class="files-shell">
				<FilesSidebar
					variant="files"
					mode={listMode()}
					onSelectMode={onSelectMode}
					mobileOpen={mobileNavOpen()}
					onMobileClose={() => setMobileNavOpen(false)}
					createDisabled={!browseMode()}
					onCreateFolder={openCreateFolderDialog}
					onUploadFile={uploadFileClickHandler}
					onUploadFolder={uploadFolderClickHandler}
				/>
				<div class="files-shell__main">
					<Stack
						class="files-page"
						spacing={0}
						sx={{ flex: 1, minHeight: 0, height: '100%', gap: '12px' }}
					>
				<div class="files-page__toolbar">
					<IconButton
						class="files-page__nav-toggle"
						aria-label="Open files menu"
						onClick={() => setMobileNavOpen(true)}
					>
						<FluentIcon name="navigation" size={22} />
					</IconButton>

					<Show when={browseMode()}>
						<nav class="files-breadcrumb" aria-label="Folder path">
							<For each={pathCrumbs()}>
								{(crumb, index) => (
									<>
										<Show when={index() > 0}>
											<FluentIcon
												name="chevronRight"
												class="files-breadcrumb__sep"
												size={16}
											/>
										</Show>
										<button
											type="button"
											class="files-breadcrumb__crumb"
											classList={{
												'files-breadcrumb__crumb--current':
													index() === pathCrumbs().length - 1,
												'files-breadcrumb__crumb--drop':
													crumbDropPath() === crumb.path,
											}}
											title={crumb.path || 'All files'}
											onClick={() => goToFolder(crumb.path)}
											onDragOver={(e) =>
												onCrumbDragOver(crumb.path, e)
											}
											onDragLeave={(e) =>
												onCrumbDragLeave(crumb.path, e)
											}
											onDrop={(e) => onCrumbDrop(crumb.path, e)}
										>
											{crumb.label}
										</button>
									</>
								)}
							</For>
						</nav>
					</Show>

					<Show when={!browseMode()}>
						<Typography
							variant="body2"
							color="text.secondary"
							sx={{ mr: 'auto' }}
						>
							{listMode() === 'favorites'
								? 'Favorites'
								: listMode() === 'recent'
									? 'Recent'
									: listMode() === 'shared'
										? 'Shared links'
										: 'Trash'}
						</Typography>
					</Show>

					<FilesSearch />

					<Show when={selectionModeEnabled() && selectionActive()}>
						<div class="files-bulk-bar">
							<span class="files-bulk-bar__count">
								{selectedCount()} selected
							</span>
							<Show
								when={trashMode()}
								fallback={
									<>
										<Button
											variant="outlined"
											color="inherit"
											size="small"
											startIcon={<FluentIcon name="copy" size={18} />}
											onClick={openBulkCopy}
										>
											Copy
										</Button>
										<Button
											variant="outlined"
											color="inherit"
											size="small"
											startIcon={<FluentIcon name="arrowMove" size={18} />}
											onClick={openBulkMove}
										>
											Move
										</Button>
										<Button
											variant="outlined"
											color="warning"
											size="small"
											startIcon={<FluentIcon name="delete" size={18} />}
											onClick={() => setBulkDeleteOpen(true)}
										>
											Delete
										</Button>
									</>
								}
							>
								<Button
									variant="outlined"
									color="inherit"
									size="small"
									startIcon={<FluentIcon name="arrowUndo" size={18} />}
									onClick={confirmBulkRestore}
								>
									Restore
								</Button>
								<Button
									variant="outlined"
									color="warning"
									size="small"
									startIcon={<FluentIcon name="deleteDismiss" size={18} />}
									onClick={() => setBulkDeleteOpen(true)}
								>
									Delete forever
								</Button>
							</Show>
							<IconButton
								size="small"
								aria-label="Clear selection"
								onClick={clearSelection}
							>
								<FluentIcon name="dismiss" size={18} />
							</IconButton>
						</div>
					</Show>

					<Show when={browseMode()}>
						<Button
							variant="outlined"
							color="inherit"
							size="small"
							startIcon={<FluentIcon name="arrowSort" size={18} />}
							onClick={(e) => setSortMenuAnchor(e.currentTarget)}
						>
							{sortLabel(sortField(), sortDir())}
						</Button>
						<MenuMUI
							anchorEl={sortMenuAnchor()}
							open={Boolean(sortMenuAnchor())}
							onClose={() => setSortMenuAnchor(null)}
						>
							<MenuItem
								selected={sortField() === 'name'}
								onClick={() => chooseSortField('name')}
							>
								Name
							</MenuItem>
							<MenuItem
								selected={sortField() === 'size'}
								onClick={() => chooseSortField('size')}
							>
								Size
							</MenuItem>
							<MenuItem
								selected={sortField() === 'mtime'}
								onClick={() => chooseSortField('mtime')}
							>
								Date modified
							</MenuItem>
							<MenuItem
								selected={sortField() === 'type'}
								onClick={() => chooseSortField('type')}
							>
								File type
							</MenuItem>
							<Divider />
							<MenuItem
								selected={sortDir() === 'asc'}
								onClick={() => {
									setSortDir('asc')
									setSortMenuAnchor(null)
								}}
							>
								Ascending
							</MenuItem>
							<MenuItem
								selected={sortDir() === 'desc'}
								onClick={() => {
									setSortDir('desc')
									setSortMenuAnchor(null)
								}}
							>
								Descending
							</MenuItem>
						</MenuMUI>
					</Show>

					<Show when={!sharedMode()}>
						<div
							class="files-view-toggle"
							role="group"
							aria-label="View mode"
						>
							<IconButton
								size="small"
								class="files-view-toggle__btn"
								classList={{
									'files-view-toggle__btn--active':
										viewMode() === 'list',
								}}
								aria-label="List view"
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
								aria-label="Tiles view"
								aria-pressed={viewMode() === 'tiles'}
								onClick={() => setAndPersistViewMode('tiles')}
							>
								<FluentIcon
									name={viewMode() === 'tiles' ? 'gridFilled' : 'grid'}
									size={18}
								/>
							</IconButton>
						</div>
					</Show>

					<Show when={trashMode()}>
						<Button
							variant="contained"
							color="warning"
							onClick={() => setEmptyTrashOpen(true)}
						>
							Empty trash
						</Button>
					</Show>
				</div>

				<Show
					when={sharedMode()}
					fallback={<div
						ref={(el) => {
							filesCanvasEl = el
							setFilesCanvas(el)
						}}
						class="files-canvas"
						classList={{
							'files-canvas--list': viewMode() === 'list',
							'files-canvas--selecting': selectionActive(),
							'files-canvas--drop-active': canvasDropActive(),
							'files-canvas--marquee': Boolean(marqueeBox()),
						}}
						onMouseDown={onCanvasMouseDown}
						onDragOver={onCanvasDragOver}
						onDragLeave={onCanvasDragLeave}
						onDrop={onCanvasDrop}
					>
					<div
						class="files-ptr-indicator"
						classList={{
							'files-ptr-indicator--visible':
								ptrPull() > 0 || ptrRefreshing(),
							'files-ptr-indicator--spin': ptrRefreshing(),
						}}
						style={{
							height: ptrRefreshing()
								? '48px'
								: ptrPull() > 0
									? `${Math.min(96, ptrPull())}px`
									: '0px',
						}}
						aria-hidden="true"
					>
						<span class="files-ptr-indicator__icon">
							<FluentIcon name="arrowSync" size={22} />
						</span>
					</div>
					<Show when={marqueeBox()}>
						<div
							class="files-marquee"
							style={{
								left: `${marqueeBox().left}px`,
								top: `${marqueeBox().top}px`,
								width: `${marqueeBox().width}px`,
								height: `${marqueeBox().height}px`,
							}}
						/>
					</Show>
					<Show
						when={sortedFsLayer().length}
						fallback={
							<div
								class="files-canvas__empty"
								onMouseDown={(e) => e.preventDefault()}
							>
								{listMode() === 'trash'
									? 'Trash is empty'
									: listMode() === 'favorites'
										? 'No favorites yet — star a file to pin it here'
										: listMode() === 'recent'
											? 'No recently opened files'
											: chrome.isSearching()
												? 'No search results'
												: 'No files yet'}
							</div>
						}
					>
						<div
							class={
								viewMode() === 'list' ? 'files-list' : 'files-grid'
							}
						>
							{mapArray(sortedFsLayer, (fsElement) => {
								const pathKey = itemNormalizedPath(fsElement)
								const canSelect =
									selectionModeEnabled() &&
									fsElement.name !== '..'
								const isFolderDrop =
									browseMode() &&
									!fsElement.is_file &&
									fsElement.name !== '..'
								return (
								<FSListItem
									fsElement={fsElement}
									storageId={params.id}
									onDelete={refreshCurrent}
									onOpen={(file) => setViewerFile(file)}
									onPrefetch={prefetchViewerPreview}
									trashMode={trashMode()}
									flatMode={flatMode()}
									layout={viewMode}
									selectable={canSelect}
									selected={() =>
										Boolean(selectedPaths()[pathKey])
									}
									onSelectItem={onSelectItem}
									draggableItem={browseMode() && canSelect}
									onDragStartItem={onDragStartItem}
									dropTarget={isFolderDrop}
									dropActive={() =>
										isFolderDrop &&
										dropTargetPath() === pathKey
									}
									onDragOverItem={(e) =>
										onDragOverFolder(fsElement, e)
									}
									onDragLeaveItem={(e) =>
										onDragLeaveFolder(fsElement, e)
									}
									onDropItem={(e) =>
										onDropOnFolder(fsElement, e)
									}
									isFavorite={() => isFavorite(fsElement)}
									onToggleFavorite={toggleFavorite}
									onRestore={(el) => restoreItem(el)}
									onDeleteForever={deleteForeverItem}
									onTrashNavigate={onTrashNavigate}
									onCopyTo={openCopyTo}
									onMoveTo={openMoveTo}
									onRename={renameItem}
									onContextMenuItem={onContextMenuItem}
								/>
								)
							})}
						</div>
					</Show>
					</div>}
				>
					<div class="files-canvas">
						<SharedLinksPanel storageId={params.id} active={sharedMode()} />
					</div>
				</Show>

				<FileViewer
					open={Boolean(viewerFile()) && !trashMode()}
					file={viewerFile()}
					files={sortedFsLayer()}
					storageId={params.id}
					onClose={closeViewer}
					onNavigate={(file) => setViewerFile(file)}
				/>

				<CreateFolderDialog
					isOpened={isCreateFolderDialogOpen()}
					onCreate={createFolder}
					onClose={closeCreateFolderDialog}
				/>

				{/* New FAB — same actions as sidebar New, visible on all viewport widths */}
				<button
					type="button"
					class="files-new-fab"
					aria-label="New"
					title="New"
					aria-haspopup="menu"
					aria-expanded={Boolean(newFabAnchor())}
					disabled={!browseMode()}
					onClick={(e) => {
						if (!browseMode()) return
						setNewFabAnchor(e.currentTarget)
					}}
				>
					<FluentIcon name="add" size={26} />
				</button>
				<MenuMUI
					anchorEl={newFabAnchor()}
					open={Boolean(newFabAnchor())}
					onClose={() => setNewFabAnchor(null)}
					anchorOrigin={{ vertical: 'top', horizontal: 'center' }}
					transformOrigin={{ vertical: 'bottom', horizontal: 'center' }}
				>
					<MenuItem
						onClick={() => {
							setNewFabAnchor(null)
							openCreateFolderDialog()
						}}
					>
						<ListItemIcon>
							<FluentIcon name="folderAdd" size={20} />
						</ListItemIcon>
						<ListItemText>Create folder</ListItemText>
					</MenuItem>
					<MenuItem
						onClick={() => {
							setNewFabAnchor(null)
							uploadFileClickHandler()
						}}
					>
						<ListItemIcon>
							<FluentIcon name="documentArrowUp" size={20} />
						</ListItemIcon>
						<ListItemText>Upload file</ListItemText>
					</MenuItem>
					<MenuItem
						onClick={() => {
							setNewFabAnchor(null)
							uploadFolderClickHandler()
						}}
					>
						<ListItemIcon>
							<FluentIcon name="folderArrowUp" size={20} />
						</ListItemIcon>
						<ListItemText>Upload folder</ListItemText>
					</MenuItem>
				</MenuMUI>

				<FolderPickerDialog
					isOpened={Boolean(folderPicker())}
					storageId={params.id}
					mode={folderPicker()?.mode || 'copy'}
					sourcePath={
						folderPicker()?.items?.find((i) => i.path.endsWith('/'))
							?.path || ''
					}
					itemName={
						folderPicker()?.items?.length === 1
							? folderPicker().items[0].name
							: folderPicker()?.items?.length
								? `${folderPicker().items.length} items`
								: undefined
					}
					onCancel={() => setFolderPicker(null)}
					onConfirm={(destination) => runTransfer(destination)}
				/>

				<ActionConfirmDialog
					action={trashMode() ? 'Delete forever' : 'Delete'}
					entity={
						selectedCount() === 1 ? 'item' : `${selectedCount()} items`
					}
					actionDescription={
						trashMode()
							? selectedCount() === 1
								? 'permanently delete this item (including Telegram copies)'
								: `permanently delete ${selectedCount()} items (including Telegram copies)`
							: selectedCount() === 1
								? 'move this item to trash'
								: `move ${selectedCount()} items to trash`
					}
					isOpened={bulkDeleteOpen()}
					onConfirm={confirmBulkDelete}
					onCancel={() => setBulkDeleteOpen(false)}
				/>

				<ActionConfirmDialog
					action="Empty"
					entity="trash"
					actionDescription="permanently delete all files in the trash, including Telegram copies"
					isOpened={emptyTrashOpen()}
					onConfirm={confirmEmptyTrash}
					onCancel={() => setEmptyTrashOpen(false)}
				/>

				<RestoreConflictDialog
					isOpened={conflictDialogOpen()}
					path={conflictPath()}
					message={conflictMessage()}
					renameLabel={conflictRenameLabel()}
					onCancel={() => {
						setRestoreConflictPath(null)
						setRestoreRemaining([])
						setPathConflict(null)
					}}
					onChoose={async (choice) => {
						const pending = pathConflict()
						if (pending) {
							await runTransfer(pending.destination, choice)
							return
						}
						const path = restoreConflictPath()
						if (!path) return
						const current = {
							path,
							name: path.split('/').filter(Boolean).pop() || path,
						}
						const rest = restoreRemaining()
						setRestoreRemaining([])
						await restoreItems([current, ...rest], choice)
					}}
				/>
				<input
					ref={uploadFileInputElement}
					type="file"
					multiple
					style="display: none"
					onChange={uploadFile}
				/>
				<input
					ref={(el) => {
						uploadFolderInputElement = el
						if (el) {
							el.setAttribute('webkitdirectory', '')
							el.setAttribute('directory', '')
							// @ts-ignore non-standard
							el.webkitdirectory = true
						}
					}}
					type="file"
					multiple
					style="display: none"
					onChange={uploadFolder}
				/>
					</Stack>
				</div>
			</div>
		</>
	)
}

export default Files
