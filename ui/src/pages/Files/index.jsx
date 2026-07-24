import { useBeforeLeave, useParams } from '@solidjs/router'
import { Show, createMemo, createSignal, mapArray, onCleanup, onMount } from 'solid-js'
import MenuItem from '@suid/material/MenuItem'
import ListItemIcon from '@suid/material/ListItemIcon'
import ListItemText from '@suid/material/ListItemText'
import UploadFileIcon from '@suid/icons-material/UploadFile'
import DriveFolderUploadIcon from '@suid/icons-material/DriveFolderUpload'
import CreateNewFolderIcon from '@suid/icons-material/CreateNewFolder'
import DeleteOutlineIcon from '@suid/icons-material/DeleteOutline'
import ArrowBackIcon from '@suid/icons-material/ArrowBack'
import SortIcon from '@suid/icons-material/Sort'
import StarIcon from '@suid/icons-material/Star'
import HistoryIcon from '@suid/icons-material/History'
import Button from '@suid/material/Button'
import Stack from '@suid/material/Stack'
import Typography from '@suid/material/Typography'
import LinearProgress from '@suid/material/LinearProgress'
import Box from '@suid/material/Box'
import MenuMUI from '@suid/material/Menu'
import Divider from '@suid/material/Divider'

import API from '../../api'
import { formatUploadBytes } from '../../api/request'
import FSListItem from '../../components/FSListItem'
import Menu from '../../components/Menu'
import CreateFolderDialog from '../../components/CreateFolderDialog'
import FolderPickerDialog from '../../components/FolderPickerDialog'
import { alertStore } from '../../components/AlertStack'
import FileViewer from '../../components/FileViewer'
import RestoreConflictDialog from '../../components/RestoreConflictDialog'
import ActionConfirmDialog from '../../components/ActionConfirmDialog'
import { filesChromeStore } from '../../common/filesChrome'
import { sortFsElements, sortLabel } from '../../common/sortFs'

const joinStoragePath = (...parts) =>
	parts
		.filter((p) => p != null && String(p).length > 0)
		.map((p) => String(p).replace(/^\/+|\/+$/g, '').trim())
		.filter(Boolean)
		.join('/')

const shouldSkipUploadEntry = (relativePath) => {
	const base = relativePath.split('/').pop() || ''
	return !base || base === '.DS_Store' || base.startsWith('._')
}

/**
 * @param {import('../../api/request').UploadProgressEvent} ev
 * @param {string} [label]
 */
const describeProgress = (ev, label) => {
	const pct = Math.round(ev.percent || 0)
	const prefix = label ? `${label} · ` : ''
	if (ev.phase === 'server') {
		return `${prefix}Sending to Sarca: ${pct}%`
	}
	const size =
		ev.total != null
			? ` · ${formatUploadBytes(ev.uploaded || 0)} / ${formatUploadBytes(ev.total)}`
			: ''
	const chunk =
		ev.chunk && ev.chunks ? ` · chunk ${ev.chunk}/${ev.chunks}` : ''
	return `${prefix}Uploading to Telegram: ${pct}%${size}${chunk}`
}

const itemNormalizedPath = (el) => {
	if (el.is_file) return el.path
	return el.path.endsWith('/') ? el.path : `${el.path}/`
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
	const [uploadProgress, setUploadProgress] = createSignal(0)
	const [uploadStatus, setUploadStatus] = createSignal('')
	const [isUploading, setIsUploading] = createSignal(false)
	/**
	 * @type {[import("solid-js").Accessor<import("../../api").FSElement | null>, any]}
	 */
	const [viewerFile, setViewerFile] = createSignal(null)
	/** @type {[import('solid-js').Accessor<'browse'|'trash'|'favorites'|'recent'>, any]} */
	const [listMode, setListMode] = createSignal('browse')
	const [trashPath, setTrashPath] = createSignal('')
	const [emptyTrashOpen, setEmptyTrashOpen] = createSignal(false)
	const [restoreConflictPath, setRestoreConflictPath] = createSignal(null)
	/** @type {[import('solid-js').Accessor<Record<string, boolean>>, any]} */
	const [favoritePaths, setFavoritePaths] = createSignal({})

	/** @type {[import('solid-js').Accessor<'name'|'size'|'mtime'|'type'>, any]} */
	const [sortField, setSortField] = createSignal('name')
	/** @type {[import('solid-js').Accessor<'asc'|'desc'>, any]} */
	const [sortDir, setSortDir] = createSignal('asc')
	const [sortMenuAnchor, setSortMenuAnchor] = createSignal(null)

	/**
	 * @type {[import('solid-js').Accessor<null | { mode: 'copy'|'move', el: import('../../api').FSElement }>, any]}
	 */
	const [folderPicker, setFolderPicker] = createSignal(null)
	/**
	 * Pending copy/move waiting on conflict resolution.
	 * @type {[import('solid-js').Accessor<null | { mode: 'copy'|'move', path: string, destination: string, name: string }>, any]}
	 */
	const [pathConflict, setPathConflict] = createSignal(null)

	const params = useParams()
	const basePath = `/storages/${params.id}/files`

	let uploadFileInputElement
	/** @type {HTMLInputElement} */
	let uploadFolderInputElement

	const trashMode = () => listMode() === 'trash'
	const flatMode = () =>
		listMode() === 'favorites' || listMode() === 'recent'
	const browseMode = () => listMode() === 'browse'

	const sortedFsLayer = createMemo(() => {
		// Favorites / Recent keep API order (starred newest / viewed_at desc).
		if (flatMode()) return fsLayer()
		return sortFsElements(fsLayer(), sortField(), sortDir())
	})

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

	const fetchFSLayer = async (path = params.path) => {
		const fsLayerRes = await API.files.getFSLayer(params.id, path)

		if (path.length) {
			const parentPath = path.split('/').slice(0, -1).join('/')
			const backToParent = {
				is_file: false,
				name: '..',
				path: parentPath,
				has_thumb: false,
				size: 0,
			}

			fsLayerRes.splice(0, 0, backToParent)
		}

		setFsLayer(fsLayerRes)
		chrome.setIsSearching(false)
		chrome.setSearchQuery('')
	}

	const fetchTrashLayer = async (path = trashPath()) => {
		const fsLayerRes = await API.files.listTrash(params.id, path)

		if (path.length) {
			const parentPath = path.split('/').slice(0, -1).join('/')
			const backToParent = {
				is_file: false,
				name: '..',
				path: parentPath,
				has_thumb: false,
				size: 0,
			}
			fsLayerRes.splice(0, 0, backToParent)
		}

		setFsLayer(fsLayerRes)
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
		} else {
			await fetchFSLayer()
		}
	}

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

	const exitSpecialMode = async () => {
		setListMode('browse')
		setTrashPath('')
		await fetchFSLayer()
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

	const restoreItem = async (el, onConflict) => {
		const path = trashItemPath(el)
		try {
			await API.files.restoreTrash(params.id, path, onConflict)
			addAlert(`Restored "${el.name}"`, 'success')
			setRestoreConflictPath(null)
			await fetchTrashLayer()
		} catch (err) {
			if (err.status === 409 && !onConflict) {
				setRestoreConflictPath(path)
				return
			}
			throw err
		}
	}

	const deleteForeverItem = async (el) => {
		await API.files.deleteForever(params.id, trashItemPath(el))
		addAlert(`Permanently deleted "${el.name}"`, 'success')
		await fetchTrashLayer()
	}

	const confirmEmptyTrash = async () => {
		setEmptyTrashOpen(false)
		await API.files.emptyTrash(params.id)
		addAlert('Trash emptied', 'success')
		await fetchTrashLayer('')
	}

	const openCopyTo = (el) => setFolderPicker({ mode: 'copy', el })
	const openMoveTo = (el) => setFolderPicker({ mode: 'move', el })

	/**
	 * @param {string} destination
	 * @param {'replace'|'rename'} [onConflict]
	 */
	const runTransfer = async (destination, onConflict) => {
		const pending = pathConflict()
		const picker = folderPicker()
		const mode = pending?.mode || picker?.mode
		const path = pending?.path || (picker ? itemNormalizedPath(picker.el) : null)
		const name = pending?.name || picker?.el?.name
		const dest = pending?.destination ?? destination

		if (!mode || path == null) return

		const apiCall =
			mode === 'copy' ? API.files.copyFile : API.files.moveFile

		try {
			await apiCall(params.id, path, dest, onConflict)
			addAlert(
				mode === 'copy' ? `Copied "${name}"` : `Moved "${name}"`,
				'success',
			)
			setFolderPicker(null)
			setPathConflict(null)
			await refreshCurrent()
		} catch (err) {
			if (err.status === 409 && !onConflict) {
				setFolderPicker(null)
				setPathConflict({ mode, path, destination: dest, name })
				return
			}
			/* alerted by API helper */
		}
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

		const results = await API.files.search(params.id, params.path || '', q)
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
			await fetchFSLayer()
		}
	}

	onMount(() => {
		chrome.activate({
			storageId: params.id,
			storageName: '',
			onSearch: runSearch,
			onClear: clearSearch,
		})
		Promise.all([fetchStorage(), fetchFSLayer(), loadFavoritePaths()]).then()
		window.addEventListener('popstate', reload, false)
	})

	onCleanup(() => {
		window.removeEventListener('popstate', reload, false)
		chrome.deactivate()
	})

	useBeforeLeave(async (e) => {
		if (e.to.startsWith(basePath)) {
			let newPath = e.to.slice(basePath.length)

			if (newPath.startsWith('/')) {
				newPath = newPath.slice(1)
			}

			await fetchFSLayer(newPath)
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
		const folderBase = params.path.endsWith('/')
			? params.path.slice(0, -1)
			: params.path

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
	 * @param {Event} event
	 */
	const uploadFile = async (event) => {
		const file = event.target.files[0]
		if (file === undefined) {
			return
		}

		event.target.value = null

		try {
			setIsUploading(true)
			setUploadStatus(`Sending ${file.name}`)
			const parentPath = (params.path || '').replace(/\/+$/, '')
			await API.files.uploadFile(params.id, parentPath, file, (ev) => {
				setUploadProgress(ev.percent || 0)
				setUploadStatus(describeProgress(ev, file.name))
			})
			addAlert(`Uploaded file "${file.name}"`, 'success')
			await fetchFSLayer()
		} catch (error) {
			console.error(error)
		} finally {
			setIsUploading(false)
			setUploadProgress(0)
			setUploadStatus('')
		}
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

		const currentPath = (params.path || '').replace(/\/+$/, '')
		const rootName = (files[0].webkitRelativePath || files[0].name).split('/')[0]
		let uploaded = 0
		let failed = 0

		try {
			setIsUploading(true)
			setUploadProgress(0)

			for (let i = 0; i < files.length; i++) {
				const file = files[i]
				const relativePath = file.webkitRelativePath || file.name
				const segments = relativePath.split('/')
				segments.pop()
				const parentPath = joinStoragePath(currentPath, ...segments)

				setUploadStatus(`Uploading ${i + 1}/${files.length}: ${relativePath}`)

				try {
					await API.files.uploadFile(
						params.id,
						parentPath,
						file,
						(ev) => {
							const fileShare = 1 / files.length
							const base = i * fileShare
							const phaseShare =
								ev.phase === 'server' ? 0.15 : 0.85
							const phaseOffset = ev.phase === 'server' ? 0 : 0.15
							const overall =
								(base +
									(phaseOffset +
										(phaseShare * (ev.percent || 0)) / 100) *
										fileShare) *
								100
							setUploadProgress(overall)
							setUploadStatus(
								describeProgress(
									ev,
									`${i + 1}/${files.length} ${relativePath}`,
								),
							)
						},
						{ silent: true },
					)
					uploaded++
				} catch (error) {
					console.error(error)
					failed++
				}
			}

			setUploadProgress(100)

			if (failed === 0) {
				addAlert(
					`Uploaded folder "${rootName}" (${uploaded} ${uploaded === 1 ? 'file' : 'files'})`,
					'success',
				)
			} else if (uploaded === 0) {
				addAlert(`Failed to upload folder "${rootName}"`, 'error')
			} else {
				addAlert(
					`Uploaded ${uploaded} of ${files.length} files from "${rootName}" (${failed} failed)`,
					'error',
				)
			}

			await fetchFSLayer()
		} finally {
			setIsUploading(false)
			setUploadProgress(0)
			setUploadStatus('')
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
			<Stack class="files-page" spacing={1.5}>
				<div class="files-page__toolbar">
					<Show when={browseMode()}>
						<Button
							variant="outlined"
							color="inherit"
							size="small"
							startIcon={<SortIcon />}
							onClick={(e) => setSortMenuAnchor(e.currentTarget)}
							sx={{ mr: 'auto' }}
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

					<Show
						when={!browseMode()}
						fallback={
							<>
								<Button
									variant="outlined"
									color="inherit"
									startIcon={<StarIcon />}
									onClick={enterFavorites}
									sx={{ mr: 1 }}
								>
									Favorites
								</Button>
								<Button
									variant="outlined"
									color="inherit"
									startIcon={<HistoryIcon />}
									onClick={enterRecent}
									sx={{ mr: 1 }}
								>
									Recent
								</Button>
								<Button
									variant="outlined"
									color="inherit"
									startIcon={<DeleteOutlineIcon />}
									onClick={enterTrash}
									sx={{ mr: 1 }}
								>
									Trash
								</Button>
								<Menu button_title="Create">
									<MenuItem onClick={openCreateFolderDialog}>
										<ListItemIcon>
											<CreateNewFolderIcon />
										</ListItemIcon>
										<ListItemText>Create folder</ListItemText>
									</MenuItem>
									<MenuItem onClick={uploadFileClickHandler}>
										<ListItemIcon>
											<UploadFileIcon />
										</ListItemIcon>
										<ListItemText>Upload file</ListItemText>
									</MenuItem>
									<MenuItem onClick={uploadFolderClickHandler}>
										<ListItemIcon>
											<DriveFolderUploadIcon />
										</ListItemIcon>
										<ListItemText>Upload folder</ListItemText>
									</MenuItem>
								</Menu>
							</>
						}
					>
						<Show when={listMode() === 'favorites' || listMode() === 'recent'}>
							<Typography
								variant="body2"
								color="text.secondary"
								sx={{ mr: 'auto' }}
							>
								{listMode() === 'favorites' ? 'Favorites' : 'Recent'}
							</Typography>
						</Show>
						<Button
							variant="outlined"
							color="inherit"
							startIcon={<ArrowBackIcon />}
							onClick={exitSpecialMode}
							sx={{ mr: 1 }}
						>
							Back
						</Button>
						<Show when={trashMode()}>
							<Button
								variant="contained"
								color="warning"
								onClick={() => setEmptyTrashOpen(true)}
							>
								Empty trash
							</Button>
						</Show>
					</Show>
				</div>

				<Show when={isUploading()}>
					<Box sx={{ width: '100%', maxWidth: 520 }}>
						<Typography variant="caption" display="block" gutterBottom>
							{uploadStatus() || `Uploading: ${Math.round(uploadProgress())}%`}
						</Typography>
						<LinearProgress
							variant="determinate"
							value={uploadProgress()}
							sx={{
								height: 10,
								borderRadius: 999,
								background: 'var(--sarca-progress-track)',
								'& .MuiLinearProgress-bar': {
									borderRadius: 999,
									background: 'var(--sarca-progress-fill)',
								},
							}}
						/>
					</Box>
				</Show>

				<div class="files-canvas glass-panel">
					<Show
						when={sortedFsLayer().length}
						fallback={
							<div class="files-canvas__empty">
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
						<div class="files-grid">
							{mapArray(sortedFsLayer, (fsElement) => (
								<FSListItem
									fsElement={fsElement}
									storageId={params.id}
									onDelete={refreshCurrent}
									onOpen={(file) => setViewerFile(file)}
									trashMode={trashMode()}
									flatMode={flatMode()}
									isFavorite={() => isFavorite(fsElement)}
									onToggleFavorite={toggleFavorite}
									onRestore={(el) => restoreItem(el)}
									onDeleteForever={deleteForeverItem}
									onTrashNavigate={onTrashNavigate}
									onCopyTo={openCopyTo}
									onMoveTo={openMoveTo}
								/>
							))}
						</div>
					</Show>
				</div>

				<FileViewer
					open={Boolean(viewerFile()) && !trashMode()}
					file={viewerFile()}
					files={sortedFsLayer()}
					storageId={params.id}
					onClose={() => setViewerFile(null)}
					onNavigate={(file) => setViewerFile(file)}
				/>

				<CreateFolderDialog
					isOpened={isCreateFolderDialogOpen()}
					onCreate={createFolder}
					onClose={closeCreateFolderDialog}
				/>

				<FolderPickerDialog
					isOpened={Boolean(folderPicker())}
					storageId={params.id}
					mode={folderPicker()?.mode || 'copy'}
					sourcePath={
						folderPicker() ? itemNormalizedPath(folderPicker().el) : ''
					}
					itemName={folderPicker()?.el?.name}
					onCancel={() => setFolderPicker(null)}
					onConfirm={(destination) => runTransfer(destination)}
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
						try {
							await API.files.restoreTrash(params.id, path, choice)
							addAlert('Restored', 'success')
							setRestoreConflictPath(null)
							await fetchTrashLayer()
						} catch {
							/* alerted by API */
						}
					}}
				/>
				<input
					ref={uploadFileInputElement}
					type="file"
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
		</>
	)
}

export default Files
