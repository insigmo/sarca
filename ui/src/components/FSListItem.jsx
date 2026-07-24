import MenuMUI from '@suid/material/Menu'
import MenuItem from '@suid/material/MenuItem'
import ListItemIcon from '@suid/material/ListItemIcon'
import ListItemText from '@suid/material/ListItemText'
import IconButton from '@suid/material/IconButton'
import MoreVertIcon from '@suid/icons-material/MoreVert'
import VisibilityIcon from '@suid/icons-material/Visibility'
import DownloadIcon from '@suid/icons-material/Download'
import InfoIcon from '@suid/icons-material/Info'
import DeleteIcon from '@suid/icons-material/Delete'
import DriveFileRenameOutlineIcon from '@suid/icons-material/DriveFileRenameOutline'
import DriveFileMoveIcon from '@suid/icons-material/DriveFileMove'
import { Show, createEffect, createSignal, onCleanup } from 'solid-js'
import { useNavigate, useParams } from '@solidjs/router'

import API from '../api'
import ActionConfirmDialog from './ActionConfirmDialog'
import FileInfoDialog from './FileInfo'
import FileTypeIcon from './FileTypeIcon'
import { alertStore } from './AlertStack'
import { fileBaseName, fileExtensionLabel } from '../common/fileLabel'

/**
 * @typedef {Object} FSListItemProps
 * @property {import("../api").FSElement} fsElement
 * @property {string} storageId
 * @property {() => {}} onDelete
 * @property {(file: import("../api").FSElement) => void} [onOpen]
 */

/**
 * Grid tile for a file or folder (reference-style 3D icon + name + extension).
 * @param {FSListItemProps} props
 */
const FSListItem = (props) => {
	const [moreAnchorEl, setMoreAnchorEl] = createSignal(null)
	const [isActionConfirmDialogOpened, setIsActionConfirmDialogOpened] =
		createSignal(false)
	const [isInfoDialogOpened, setIsInfoDialogOpened] = createSignal(false)
	const [thumbUrl, setThumbUrl] = createSignal(null)
	const { addAlert } = alertStore
	const navigate = useNavigate()
	const params = useParams()

	const openMore = () => Boolean(moreAnchorEl())

	const handleCloseMore = () => {
		setMoreAnchorEl(null)
	}

	const handleNavigate = () => {
		if (!props.fsElement.is_file) {
			navigate(`/storages/${props.storageId}/files/${props.fsElement.path}`)
		} else {
			props.onOpen?.(props.fsElement)
		}
	}

	const openViewer = () => {
		handleCloseMore()
		props.onOpen?.(props.fsElement)
	}

	const isParentNav = () => props.fsElement.name === '..'

	const normalizedPath = () => {
		const p = props.fsElement.path
		if (props.fsElement.is_file) return p
		return p.endsWith('/') ? p : `${p}/`
	}

	createEffect(() => {
		const el = props.fsElement
		let revoked = false
		let objectUrl = null

		setThumbUrl(null)

		if (el.is_file && el.has_thumb) {
			API.files
				.thumb(props.storageId, el.path)
				.then((blob) => {
					if (revoked) return
					objectUrl = URL.createObjectURL(blob)
					setThumbUrl(objectUrl)
				})
				.catch(() => {
					if (!revoked) setThumbUrl(null)
				})
		}

		onCleanup(() => {
			revoked = true
			if (objectUrl) URL.revokeObjectURL(objectUrl)
		})
	})

	const download = async () => {
		handleCloseMore()

		if (isParentNav()) {
			return
		}

		const isFile = props.fsElement.is_file
		const maxFolderBytes = 10 * 1024 * 1024 * 1024

		if (!isFile && (props.fsElement.size || 0) > maxFolderBytes) {
			addAlert(
				'Folder is larger than 10 GB. Download files in smaller pieces.',
				'error',
			)
			return
		}

		const path = isFile ? props.fsElement.path : normalizedPath()

		try {
			const blob = await API.files.download(params.id, path)
			const href = URL.createObjectURL(blob)
			const filename = isFile
				? props.fsElement.name
				: `${props.fsElement.name}.zip`
			const a = Object.assign(document.createElement('a'), {
				href,
				style: 'display: none',
				download: filename,
			})
			document.body.appendChild(a)
			a.click()
			URL.revokeObjectURL(href)
			a.remove()
		} catch (err) {
			console.error(err)
		}
	}

	const openActionConfirmDialog = () => {
		handleCloseMore()
		setIsActionConfirmDialogOpened(true)
	}
	const closeActionConfirmDialog = () => {
		setIsActionConfirmDialogOpened(false)
	}

	const deleteFile = async () => {
		closeActionConfirmDialog()
		await API.files.deleteFile(params.id, normalizedPath())
		props.onDelete()
	}

	const rename = async () => {
		handleCloseMore()
		const currentName = props.fsElement.name
		const newName = window.prompt('New name', currentName)
		if (!newName || newName === currentName) {
			return
		}
		try {
			await API.files.rename(params.id, normalizedPath(), newName)
			addAlert(`Renamed to "${newName}"`, 'success')
			props.onDelete()
		} catch (err) {
			console.error(err)
		}
	}

	const moveTo = async () => {
		handleCloseMore()
		const destination = window.prompt(
			'Destination folder path (empty for root)',
			'',
		)
		if (destination === null) {
			return
		}
		try {
			await API.files.moveFile(params.id, normalizedPath(), destination.trim())
			addAlert('Moved successfully', 'success')
			props.onDelete()
		} catch (err) {
			console.error(err)
		}
	}

	const displayName = () =>
		fileBaseName(props.fsElement.name, props.fsElement.is_file)
	const displayExt = () =>
		fileExtensionLabel(props.fsElement.name, props.fsElement.is_file)

	return (
		<>
			<div
				class="fs-grid-item"
				role="button"
				tabIndex={0}
				onClick={handleNavigate}
				onKeyDown={(e) => {
					if (e.key === 'Enter' || e.key === ' ') {
						e.preventDefault()
						handleNavigate()
					}
				}}
			>
				<Show when={!isParentNav()}>
					<div class="fs-grid-item__more">
						<IconButton
							size="small"
							onClick={(event) => {
								event.stopPropagation()
								setMoreAnchorEl(event.currentTarget)
							}}
							aria-label="More actions"
						>
							<MoreVertIcon fontSize="small" />
						</IconButton>
					</div>
				</Show>

				<FileTypeIcon
					name={props.fsElement.name}
					isFile={props.fsElement.is_file}
					thumbUrl={thumbUrl()}
					size={64}
				/>

				<div class="fs-grid-item__name" title={props.fsElement.name}>
					{displayName()}
				</div>
				<div class="fs-grid-item__ext">{displayExt()}</div>
			</div>

			<MenuMUI
				id="basic-menu"
				anchorEl={moreAnchorEl()}
				open={openMore()}
				onClose={handleCloseMore}
				MenuListProps={{ 'aria-labelledby': 'basic-button' }}
			>
				<MenuItem
					onClick={openViewer}
					disabled={!props.fsElement.is_file}
				>
					<ListItemIcon>
						<VisibilityIcon fontSize="small" />
					</ListItemIcon>
					<ListItemText>Open</ListItemText>
				</MenuItem>

				<MenuItem onClick={() => setIsInfoDialogOpened(true)}>
					<ListItemIcon>
						<InfoIcon fontSize="small" />
					</ListItemIcon>
					<ListItemText>Info</ListItemText>
				</MenuItem>

				<MenuItem onClick={download} disabled={isParentNav()}>
					<ListItemIcon>
						<DownloadIcon fontSize="small" />
					</ListItemIcon>
					<ListItemText>Download</ListItemText>
				</MenuItem>

				<MenuItem onClick={rename}>
					<ListItemIcon>
						<DriveFileRenameOutlineIcon fontSize="small" />
					</ListItemIcon>
					<ListItemText>Rename</ListItemText>
				</MenuItem>

				<MenuItem onClick={moveTo}>
					<ListItemIcon>
						<DriveFileMoveIcon fontSize="small" />
					</ListItemIcon>
					<ListItemText>Move</ListItemText>
				</MenuItem>

				<MenuItem onClick={openActionConfirmDialog}>
					<ListItemIcon>
						<DeleteIcon fontSize="small" />
					</ListItemIcon>
					<ListItemText>Delete</ListItemText>
				</MenuItem>
			</MenuMUI>

			<ActionConfirmDialog
				action="Delete"
				entity="file"
				actionDescription={`delete file ${props.fsElement.name}`}
				isOpened={isActionConfirmDialogOpened()}
				onConfirm={deleteFile}
				onCancel={closeActionConfirmDialog}
			/>

			<FileInfoDialog
				file={props.fsElement}
				isOpened={isInfoDialogOpened()}
				onClose={() => setIsInfoDialogOpened(false)}
			/>
		</>
	)
}

export default FSListItem
