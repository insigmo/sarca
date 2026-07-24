import { useBeforeLeave, useNavigate, useParams } from '@solidjs/router'
import { Show, createSignal, mapArray, onCleanup, onMount } from 'solid-js'
import MenuItem from '@suid/material/MenuItem'
import ListItemIcon from '@suid/material/ListItemIcon'
import ListItemText from '@suid/material/ListItemText'
import UploadFileIcon from '@suid/icons-material/UploadFile'
import DriveFolderUploadIcon from '@suid/icons-material/DriveFolderUpload'
import CreateNewFolderIcon from '@suid/icons-material/CreateNewFolder'
import Stack from '@suid/material/Stack'
import Typography from '@suid/material/Typography'
import LinearProgress from '@suid/material/LinearProgress'
import Box from '@suid/material/Box'

import API from '../../api'
import { formatUploadBytes } from '../../api/request'
import FSListItem from '../../components/FSListItem'
import Menu from '../../components/Menu'
import CreateFolderDialog from '../../components/CreateFolderDialog'
import { alertStore } from '../../components/AlertStack'
import FileViewer from '../../components/FileViewer'
import { filesChromeStore } from '../../common/filesChrome'

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

	const navigate = useNavigate()
	const params = useParams()
	const basePath = `/storages/${params.id}/files`

	let uploadFileInputElement
	/** @type {HTMLInputElement} */
	let uploadFolderInputElement

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

	/**
	 * @param {string} query
	 */
	const runSearch = async (query) => {
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
		Promise.all([fetchStorage(), fetchFSLayer()]).then()
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

	return (
		<>
			<Stack class="files-page" spacing={1.5}>
				<div class="files-page__toolbar">
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
						<MenuItem
							onClick={() => navigate(`/storages/${params.id}/upload_to`)}
						>
							<ListItemIcon>
								<UploadFileIcon />
							</ListItemIcon>
							<ListItemText>Upload to folder</ListItemText>
						</MenuItem>
					</Menu>
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
						when={fsLayer().length}
						fallback={
							<div class="files-canvas__empty">
								{chrome.isSearching() ? 'No search results' : 'No files yet'}
							</div>
						}
					>
						<div class="files-grid">
							{mapArray(fsLayer, (fsElement) => (
								<FSListItem
									fsElement={fsElement}
									storageId={params.id}
									onDelete={fetchFSLayer}
									onOpen={(file) => setViewerFile(file)}
								/>
							))}
						</div>
					</Show>
				</div>

				<FileViewer
					open={Boolean(viewerFile())}
					file={viewerFile()}
					files={fsLayer()}
					storageId={params.id}
					onClose={() => setViewerFile(null)}
					onNavigate={(file) => setViewerFile(file)}
				/>

				<CreateFolderDialog
					isOpened={isCreateFolderDialogOpen()}
					onCreate={createFolder}
					onClose={closeCreateFolderDialog}
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
