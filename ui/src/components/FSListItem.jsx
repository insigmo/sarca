import ContentCopyIcon from '@suid/icons-material/ContentCopy'
import DriveFileMoveIcon from '@suid/icons-material/DriveFileMove'
import DriveFileRenameOutlineIcon from '@suid/icons-material/DriveFileRenameOutline'
import DeleteIcon from '@suid/icons-material/Delete'
import DeleteForeverIcon from '@suid/icons-material/DeleteForever'
import DownloadIcon from '@suid/icons-material/Download'
import InfoIcon from '@suid/icons-material/Info'
import LinkIcon from '@suid/icons-material/Link'
import MoreVertIcon from '@suid/icons-material/MoreVert'
import RestoreFromTrashIcon from '@suid/icons-material/RestoreFromTrash'
import StarIcon from '@suid/icons-material/Star'
import StarBorderIcon from '@suid/icons-material/StarBorder'
import VisibilityIcon from '@suid/icons-material/Visibility'
import CircularProgress from '@suid/material/CircularProgress'
import IconButton from '@suid/material/IconButton'
import ListItemIcon from '@suid/material/ListItemIcon'
import ListItemText from '@suid/material/ListItemText'
import MenuItem from '@suid/material/MenuItem'
import MenuMUI from '@suid/material/Menu'
import { Show, createEffect, createSignal, onCleanup } from 'solid-js'
import { Portal } from 'solid-js/web'
import { useNavigate, useParams } from '@solidjs/router'

import API from '../api'
import { fileBaseName, fileExtensionLabel } from '../common/fileLabel'
import ActionConfirmDialog from './ActionConfirmDialog'
import FileInfoDialog from './FileInfo'
import FileTypeIcon from './FileTypeIcon'
import ShareLinkDialog from './ShareLinkDialog'
import { alertStore } from './AlertStack'

/**
 * @typedef {Object} FSListItemProps
 * @property {import("../api").FSElement} fsElement
 * @property {string} storageId
 * @property {() => {}} onDelete
 * @property {(file: import("../api").FSElement) => void} [onOpen]
 * @property {boolean} [trashMode]
 * @property {boolean} [flatMode] Favorites / Recent: open files only, no folder browse
 * @property {boolean | (() => boolean)} [isFavorite]
 * @property {(el: import("../api").FSElement) => void | Promise<void>} [onToggleFavorite]
 * @property {(el: import("../api").FSElement) => void} [onRestore]
 * @property {(el: import("../api").FSElement) => void} [onDeleteForever]
 * @property {(el: import("../api").FSElement) => void} [onTrashNavigate]
 * @property {(el: import("../api").FSElement) => void} [onCopyTo]
 * @property {(el: import("../api").FSElement) => void} [onMoveTo]
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
	const [isShareDialogOpened, setIsShareDialogOpened] = createSignal(false)
	const [thumbUrl, setThumbUrl] = createSignal(null)
	const [isDownloading, setIsDownloading] = createSignal(false)
	const { addAlert } = alertStore
	const navigate = useNavigate()
	const params = useParams()

	const openMore = () => Boolean(moreAnchorEl())

	const handleCloseMore = () => {
		setMoreAnchorEl(null)
	}

	const isParentNav = () => props.fsElement.name === '..'

	const handleNavigate = () => {
		if (props.trashMode) {
			if (!props.fsElement.is_file) {
				props.onTrashNavigate?.(props.fsElement)
			}
			return
		}
		if (props.flatMode) {
			if (props.fsElement.is_file) {
				props.onOpen?.(props.fsElement)
			}
			return
		}
		if (!props.fsElement.is_file) {
			navigate(`/storages/${props.storageId}/files/${props.fsElement.path}`)
		} else {
			props.onOpen?.(props.fsElement)
		}
	}

	const canFavorite = () =>
		!props.trashMode &&
		!isParentNav() &&
		props.fsElement.is_file &&
		typeof props.onToggleFavorite === 'function'

	const favorited = () => {
		const v = props.isFavorite
		return typeof v === 'function' ? Boolean(v()) : Boolean(v)
	}

	const toggleFavorite = async (event) => {
		event?.stopPropagation?.()
		handleCloseMore()
		if (!canFavorite()) return
		await props.onToggleFavorite?.(props.fsElement)
	}

	const openViewer = () => {
		handleCloseMore()
		props.onOpen?.(props.fsElement)
	}

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

		if (isParentNav() || isDownloading()) {
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

		setIsDownloading(true)
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
			addAlert(
				isFile ? 'Download started' : 'ZIP ready — download started',
				'success',
			)
		} catch (err) {
			console.error(err)
		} finally {
			setIsDownloading(false)
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

	const confirmDeleteForever = async () => {
		closeActionConfirmDialog()
		props.onDeleteForever?.(props.fsElement)
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

	const copyTo = () => {
		handleCloseMore()
		props.onCopyTo?.(props.fsElement)
	}

	const moveTo = () => {
		handleCloseMore()
		props.onMoveTo?.(props.fsElement)
	}

	const openShare = () => {
		handleCloseMore()
		setIsShareDialogOpened(true)
	}

	const canShare = () => !props.trashMode && !isParentNav()

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
				<Show when={canFavorite()}>
					<div
						class="fs-grid-item__star"
						classList={{ 'fs-grid-item__star--active': favorited() }}
					>
						<IconButton
							size="small"
							onClick={toggleFavorite}
							aria-label={
								favorited()
									? 'Remove from favorites'
									: 'Add to favorites'
							}
							title={
								favorited()
									? 'Remove from favorites'
									: 'Add to favorites'
							}
						>
							{favorited() ? (
								<StarIcon fontSize="small" color="warning" />
							) : (
								<StarBorderIcon fontSize="small" />
							)}
						</IconButton>
					</div>
				</Show>
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
				<Show
					when={props.trashMode}
					fallback={
						<>
							<MenuItem
								onClick={openViewer}
								disabled={!props.fsElement.is_file}
							>
								<ListItemIcon>
									<VisibilityIcon fontSize="small" />
								</ListItemIcon>
								<ListItemText>Open</ListItemText>
							</MenuItem>

							<Show when={canFavorite()}>
								<MenuItem onClick={toggleFavorite}>
									<ListItemIcon>
										{favorited() ? (
											<StarIcon fontSize="small" />
										) : (
											<StarBorderIcon fontSize="small" />
										)}
									</ListItemIcon>
									<ListItemText>
										{favorited()
											? 'Remove from favorites'
											: 'Add to favorites'}
									</ListItemText>
								</MenuItem>
							</Show>

							<MenuItem onClick={() => setIsInfoDialogOpened(true)}>
								<ListItemIcon>
									<InfoIcon fontSize="small" />
								</ListItemIcon>
								<ListItemText>Info</ListItemText>
							</MenuItem>

							<MenuItem
								onClick={download}
								disabled={isParentNav() || isDownloading()}
							>
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

							<MenuItem onClick={copyTo}>
								<ListItemIcon>
									<ContentCopyIcon fontSize="small" />
								</ListItemIcon>
								<ListItemText>Copy to…</ListItemText>
							</MenuItem>

							<MenuItem onClick={moveTo}>
								<ListItemIcon>
									<DriveFileMoveIcon fontSize="small" />
								</ListItemIcon>
								<ListItemText>Move to…</ListItemText>
							</MenuItem>

							<Show when={canShare()}>
								<MenuItem onClick={openShare}>
									<ListItemIcon>
										<LinkIcon fontSize="small" />
									</ListItemIcon>
									<ListItemText>Share link…</ListItemText>
								</MenuItem>
							</Show>

							<MenuItem onClick={openActionConfirmDialog}>
								<ListItemIcon>
									<DeleteIcon fontSize="small" />
								</ListItemIcon>
								<ListItemText>Delete</ListItemText>
							</MenuItem>
						</>
					}
				>
					<MenuItem
						onClick={() => {
							handleCloseMore()
							props.onRestore?.(props.fsElement)
						}}
					>
						<ListItemIcon>
							<RestoreFromTrashIcon fontSize="small" />
						</ListItemIcon>
						<ListItemText>Restore</ListItemText>
					</MenuItem>
					<MenuItem onClick={openActionConfirmDialog}>
						<ListItemIcon>
							<DeleteForeverIcon fontSize="small" />
						</ListItemIcon>
						<ListItemText>Delete forever</ListItemText>
					</MenuItem>
				</Show>
			</MenuMUI>

			<ActionConfirmDialog
				action={props.trashMode ? 'Delete forever' : 'Move to trash'}
				entity={props.fsElement.is_file ? 'file' : 'folder'}
				actionDescription={
					props.trashMode
						? `permanently delete ${props.fsElement.name} (including Telegram copies)`
						: `move ${props.fsElement.name} to trash`
				}
				isOpened={isActionConfirmDialogOpened()}
				onConfirm={props.trashMode ? confirmDeleteForever : deleteFile}
				onCancel={closeActionConfirmDialog}
			/>

			<FileInfoDialog
				file={props.fsElement}
				storageId={props.storageId}
				isOpened={isInfoDialogOpened()}
				onClose={() => setIsInfoDialogOpened(false)}
			/>

			<ShareLinkDialog
				isOpened={isShareDialogOpened()}
				storageId={props.storageId}
				path={normalizedPath()}
				itemName={props.fsElement.name}
				isFile={props.fsElement.is_file}
				onClose={() => setIsShareDialogOpened(false)}
			/>

			<Show when={isDownloading()}>
				<Portal mount={document.body}>
					<div class="download-preparing" role="status" aria-live="polite">
						<CircularProgress color="secondary" size={42} />
						<div class="download-preparing__text">
							{props.fsElement.is_file
								? 'Preparing download…'
								: 'Preparing ZIP archive…'}
						</div>
						<div class="download-preparing__hint">
							This may take a while for large folders
						</div>
					</div>
				</Portal>
			</Show>
		</>
	)
}

export default FSListItem
