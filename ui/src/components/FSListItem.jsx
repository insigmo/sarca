import ListItem from '@suid/material/ListItem'
import ListItemButton from '@suid/material/ListItemButton'
import ListItemIcon from '@suid/material/ListItemIcon'
import ListItemText from '@suid/material/ListItemText'
import MenuMUI from '@suid/material/Menu'
import MenuItem from '@suid/material/MenuItem'
import IconButton from '@suid/material/IconButton'
import Box from '@suid/material/Box'
import FileIcon from '@suid/icons-material/InsertDriveFileOutlined'
import FolderIcon from '@suid/icons-material/Folder'
import ImageIcon from '@suid/icons-material/ImageOutlined'
import VideoIcon from '@suid/icons-material/VideocamOutlined'
import PdfIcon from '@suid/icons-material/PictureAsPdfOutlined'
import AudioIcon from '@suid/icons-material/AudioFileOutlined'
import ArchiveIcon from '@suid/icons-material/FolderZipOutlined'
import TextIcon from '@suid/icons-material/DescriptionOutlined'
import MoreVertIcon from '@suid/icons-material/MoreVert'
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
import { alertStore } from './AlertStack'

const THUMB_SIZE = 48

/**
 * @param {string} name
 */
const fileKind = (name) => {
	const ext = name.includes('.')
		? name.slice(name.lastIndexOf('.') + 1).toLowerCase()
		: ''

	if (['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'svg', 'heic'].includes(ext)) {
		return 'image'
	}
	if (['mp4', 'mkv', 'webm', 'mov', 'avi', 'm4v'].includes(ext)) {
		return 'video'
	}
	if (ext === 'pdf') {
		return 'pdf'
	}
	if (['mp3', 'wav', 'flac', 'ogg', 'aac', 'm4a'].includes(ext)) {
		return 'audio'
	}
	if (['zip', 'rar', '7z', 'tar', 'gz', 'bz2', 'xz'].includes(ext)) {
		return 'archive'
	}
	if (
		['txt', 'md', 'json', 'xml', 'csv', 'log', 'rs', 'js', 'ts', 'py', 'html', 'css'].includes(
			ext,
		)
	) {
		return 'text'
	}
	return 'generic'
}

/**
 * @param {string} kind
 */
const kindIcon = (kind) => {
	switch (kind) {
		case 'image':
			return <ImageIcon fontSize="small" />
		case 'video':
			return <VideoIcon fontSize="small" />
		case 'pdf':
			return <PdfIcon fontSize="small" />
		case 'audio':
			return <AudioIcon fontSize="small" />
		case 'archive':
			return <ArchiveIcon fontSize="small" />
		case 'text':
			return <TextIcon fontSize="small" />
		default:
			return <FileIcon fontSize="small" />
	}
}

/**
 * @param {string} kind
 */
const kindColor = (kind) => {
	switch (kind) {
		case 'image':
			return { bg: '#e3f2fd', fg: '#1565c0' }
		case 'video':
			return { bg: '#fce4ec', fg: '#c2185b' }
		case 'pdf':
			return { bg: '#ffebee', fg: '#c62828' }
		case 'audio':
			return { bg: '#f3e5f5', fg: '#7b1fa2' }
		case 'archive':
			return { bg: '#fff3e0', fg: '#ef6c00' }
		case 'text':
			return { bg: '#e8f5e9', fg: '#2e7d32' }
		default:
			return { bg: '#eceff1', fg: '#546e7a' }
	}
}

/**
 * @typedef {Object} FSListItemProps
 * @property {import("../api").FSElement} fsElement
 * @property {string} storageId
 * @property {() => {}} onDelete
 */

/**
 *
 * @param {FSListItemProps} props
 * @returns
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
		}
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
		const blob = await API.files.download(params.id, props.fsElement.path)

		const href = URL.createObjectURL(blob)
		const a = Object.assign(document.createElement('a'), {
			href,
			style: 'display: none',
			download: props.fsElement.name,
		})
		document.body.appendChild(a)

		a.click()
		URL.revokeObjectURL(href)
		a.remove()
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

	const kind = () => fileKind(props.fsElement.name)
	const colors = () => kindColor(kind())

	return (
		<>
			<ListItem disablePadding>
				<ListItemButton onClick={handleNavigate}>
					<ListItemIcon sx={{ minWidth: THUMB_SIZE + 16 }}>
						<Show
							when={props.fsElement.is_file}
							fallback={
								<Box
									sx={{
										width: THUMB_SIZE,
										height: THUMB_SIZE,
										borderRadius: 1,
										display: 'flex',
										alignItems: 'center',
										justifyContent: 'center',
										bgcolor: '#fff8e1',
										color: '#f9a825',
									}}
								>
									<FolderIcon />
								</Box>
							}
						>
							<Show
								when={thumbUrl()}
								fallback={
									<Box
										sx={{
											width: THUMB_SIZE,
											height: THUMB_SIZE,
											borderRadius: 1,
											display: 'flex',
											alignItems: 'center',
											justifyContent: 'center',
											bgcolor: colors().bg,
											color: colors().fg,
										}}
									>
										{kindIcon(kind())}
									</Box>
								}
							>
								<Box
									component="img"
									src={thumbUrl()}
									alt=""
									sx={{
										width: THUMB_SIZE,
										height: THUMB_SIZE,
										borderRadius: 1,
										objectFit: 'cover',
										bgcolor: '#eee',
									}}
								/>
							</Show>
						</Show>
					</ListItemIcon>
					<ListItemText primary={props.fsElement.name} />
				</ListItemButton>
				<Show when={!isParentNav()}>
					<IconButton
						onClick={(event) => {
							setMoreAnchorEl(event.currentTarget)
						}}
					>
						<MoreVertIcon />
					</IconButton>
				</Show>
			</ListItem>
			<MenuMUI
				id="basic-menu"
				anchorEl={moreAnchorEl()}
				open={openMore()}
				onClose={handleCloseMore}
				MenuListProps={{ 'aria-labelledby': 'basic-button' }}
			>
				<MenuItem onClick={() => setIsInfoDialogOpened(true)}>
					<ListItemIcon>
						<InfoIcon fontSize="small" />
					</ListItemIcon>
					<ListItemText>Info</ListItemText>
				</MenuItem>

				<MenuItem onClick={download} disabled={!props.fsElement.is_file}>
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
