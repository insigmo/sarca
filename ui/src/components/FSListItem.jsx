import ListItem from '@suid/material/ListItem'
import ListItemButton from '@suid/material/ListItemButton'
import ListItemIcon from '@suid/material/ListItemIcon'
import ListItemText from '@suid/material/ListItemText'
import MenuMUI from '@suid/material/Menu'
import MenuItem from '@suid/material/MenuItem'
import IconButton from '@suid/material/IconButton'
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
import FileTypeIcon, { FILE_TYPE_ICON_SIZE } from './FileTypeIcon'
import { alertStore } from './AlertStack'

/**
 * @typedef {Object} FSListItemProps
 * @property {import("../api").FSElement} fsElement
 * @property {string} storageId
 * @property {() => {}} onDelete
 */

/**
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

	return (
		<>
			<ListItem disablePadding class="fs-list-item">
				<ListItemButton onClick={handleNavigate} sx={{ py: 1.1, borderRadius: 2 }}>
					<ListItemIcon sx={{ minWidth: FILE_TYPE_ICON_SIZE + 16 }}>
						<FileTypeIcon
							name={props.fsElement.name}
							isFile={props.fsElement.is_file}
							thumbUrl={thumbUrl()}
						/>
					</ListItemIcon>
					<ListItemText
						primary={props.fsElement.name}
						primaryTypographyProps={{
							sx: {
								fontWeight: 600,
								letterSpacing: '-0.01em',
								overflow: 'hidden',
								textOverflow: 'ellipsis',
							},
						}}
					/>
				</ListItemButton>
				<Show when={!isParentNav()}>
					<IconButton
						onClick={(event) => {
							setMoreAnchorEl(event.currentTarget)
						}}
						aria-label="More actions"
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
