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
import { convertSize } from '../common/size_converter'
import ActionConfirmDialog from './ActionConfirmDialog'
import FileInfoDialog from './FileInfo'
import FileTypeIcon from './FileTypeIcon'
import FluentIcon from './FluentIcon'
import ShareLinkDialog from './ShareLinkDialog'
import { alertStore } from './AlertStack'

const LONG_PRESS_MS = 520

/**
 * @typedef {Object} FSListItemProps
 * @property {import("../api").FSElement} fsElement
 * @property {string} storageId
 * @property {() => {}} onDelete
 * @property {(file: import("../api").FSElement) => void} [onOpen]
 * @property {boolean} [trashMode]
 * @property {boolean} [flatMode] Favorites / Recent: open files only, no folder browse
 * @property {'tiles' | 'list'} [layout]
 * @property {boolean} [selectable]
 * @property {boolean} [selected]
 * @property {(el: import("../api").FSElement, event: MouseEvent) => void} [onSelectItem]
 *   Desktop: row click selects (Ctrl/Shift multi-select). Mobile (≤840px): row tap
 *   opens via onOpen/navigate; long-press / context menu can select. Double-click always opens.
 * @property {(el: import("../api").FSElement) => void} [onContextMenuItem]
 *   Called before opening the context menu (e.g. select the target).
 * @property {boolean} [draggableItem] Browse-mode internal drag source
 * @property {(el: import("../api").FSElement, event: DragEvent) => void} [onDragStartItem]
 * @property {boolean} [dropTarget] Folder accepting drops
 * @property {boolean} [dropActive]
 * @property {(event: DragEvent) => void} [onDragOverItem]
 * @property {(event: DragEvent) => void} [onDragLeaveItem]
 * @property {(event: DragEvent) => void} [onDropItem]
 * @property {boolean | (() => boolean)} [isFavorite]
 * @property {(el: import("../api").FSElement) => void | Promise<void>} [onToggleFavorite]
 * @property {(el: import("../api").FSElement) => void} [onRestore]
 * @property {(el: import("../api").FSElement) => void} [onDeleteForever]
 * @property {(el: import("../api").FSElement) => void} [onTrashNavigate]
 * @property {(el: import("../api").FSElement) => void} [onCopyTo]
 * @property {(el: import("../api").FSElement) => void} [onMoveTo]
 * @property {(el: import("../api").FSElement) => void} [onRename]
 */

/**
 * Grid tile / list row for a file or folder (icon + full name).
 * @param {FSListItemProps} props
 */
const FSListItem = (props) => {
	/** @type {[import('solid-js').Accessor<{ top: number, left: number } | null>, any]} */
	const [menuPos, setMenuPos] = createSignal(null)
	const [isActionConfirmDialogOpened, setIsActionConfirmDialogOpened] =
		createSignal(false)
	const [isInfoDialogOpened, setIsInfoDialogOpened] = createSignal(false)
	const [isShareDialogOpened, setIsShareDialogOpened] = createSignal(false)
	const [thumbUrl, setThumbUrl] = createSignal(null)
	const [isDownloading, setIsDownloading] = createSignal(false)
	const { addAlert } = alertStore
	const navigate = useNavigate()
	const params = useParams()

	/** @type {ReturnType<typeof setTimeout> | null} */
	let longPressTimer = null
	let suppressClickAfterLongPress = false
	let suppressDragAfterLongPress = false

	const openMore = () => Boolean(menuPos())

	const handleCloseMore = () => {
		setMenuPos(null)
	}

	const clearLongPress = () => {
		if (longPressTimer != null) {
			clearTimeout(longPressTimer)
			longPressTimer = null
		}
	}

	/**
	 * @param {number} clientX
	 * @param {number} clientY
	 */
	const openContextMenuAt = (clientX, clientY) => {
		if (isParentNav()) return
		suppressDragAfterLongPress = true
		props.onContextMenuItem?.(props.fsElement)
		setMenuPos({ top: clientY, left: clientX })
	}

	/**
	 * @param {MouseEvent} event
	 */
	const handleContextMenu = (event) => {
		event.preventDefault()
		event.stopPropagation()
		clearLongPress()
		openContextMenuAt(event.clientX, event.clientY)
	}

	/**
	 * @param {TouchEvent} event
	 */
	const handleTouchStart = (event) => {
		if (isParentNav() || event.touches.length !== 1) return
		const touch = event.touches[0]
		clearLongPress()
		longPressTimer = setTimeout(() => {
			longPressTimer = null
			suppressClickAfterLongPress = true
			openContextMenuAt(touch.clientX, touch.clientY)
		}, LONG_PRESS_MS)
	}

	const handleTouchEnd = () => {
		clearLongPress()
		if (suppressDragAfterLongPress) {
			window.setTimeout(() => {
				suppressDragAfterLongPress = false
			}, 400)
		}
	}

	const handleTouchMove = () => {
		clearLongPress()
	}

	onCleanup(() => clearLongPress())

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
			const encoded = String(props.fsElement.path || '')
				.split('/')
				.filter(Boolean)
				.map(encodeURIComponent)
				.join('/')
			navigate(`/storages/${props.storageId}/files/${encoded}`)
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

	const rename = () => {
		handleCloseMore()
		if (typeof props.onRename === 'function') {
			props.onRename(props.fsElement)
			return
		}
		const currentName = props.fsElement.name
		const newName = window.prompt('New name', currentName)
		if (!newName || newName === currentName) {
			return
		}
		API.files
			.rename(params.id, normalizedPath(), newName)
			.then(() => {
				addAlert(`Renamed to "${newName}"`, 'success')
				props.onDelete()
			})
			.catch((err) => {
				console.error(err)
			})
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

	/** Full item name as returned by the API (includes extension for files). */
	const displayName = () => props.fsElement.name
	const isList = () => {
		const v = props.layout
		const layout = typeof v === 'function' ? v() : v
		return layout === 'list'
	}

	const sizeLabel = () => {
		if (!props.fsElement.is_file) return 'Folder'
		const size = Number(props.fsElement.size)
		if (!Number.isFinite(size) || size < 0) return '—'
		return convertSize(size)
	}

	const mtimeLabel = () => {
		const raw = props.fsElement.mtime
		if (raw == null || raw === '') return ''
		const d = new Date(typeof raw === 'number' && raw < 1e12 ? raw * 1000 : raw)
		if (Number.isNaN(d.getTime())) return ''
		return d.toLocaleString(undefined, {
			year: 'numeric',
			month: 'short',
			day: 'numeric',
			hour: '2-digit',
			minute: '2-digit',
		})
	}

	const isSelected = () => {
		const v = props.selected
		return typeof v === 'function' ? Boolean(v()) : Boolean(v)
	}

	const isDropActive = () => {
		const v = props.dropActive
		return typeof v === 'function' ? Boolean(v()) : Boolean(v)
	}

	const showSelect = () => Boolean(props.selectable) && !isParentNav()

	const isMobileTapOpen = () => {
		if (typeof window === 'undefined' || !window.matchMedia) return false
		return window.matchMedia('(max-width: 840px)').matches
	}

	const dragEnabled = () =>
		Boolean(props.draggableItem) &&
		!isParentNav() &&
		!suppressDragAfterLongPress &&
		!isMobileTapOpen()

	const handleItemClick = (event) => {
		if (suppressClickAfterLongPress) {
			suppressClickAfterLongPress = false
			event.preventDefault()
			event.stopPropagation()
			return
		}
		if (isMobileTapOpen()) {
			event.preventDefault()
			event.stopPropagation()
			handleNavigate()
			return
		}
		if (showSelect() && typeof props.onSelectItem === 'function') {
			event.preventDefault()
			event.stopPropagation()
			props.onSelectItem(props.fsElement, event)
			return
		}
		handleNavigate()
	}

	const handleItemDblClick = (event) => {
		event.preventDefault()
		event.stopPropagation()
		handleNavigate()
	}

	const handleItemKeyDown = (e) => {
		if (e.key === 'Enter' || e.key === ' ') {
			e.preventDefault()
			handleNavigate()
		}
	}

	const itemClassList = (base) => ({
		[`${base}--selected`]: isSelected(),
		[`${base}--drop-target`]: isDropActive(),
	})

	const itemPathAttr = () =>
		isParentNav() ? undefined : itemNormalizedPathForAttr()

	const itemNormalizedPathForAttr = () => {
		const p = props.fsElement.path
		if (props.fsElement.is_file) return p
		return p.endsWith('/') ? p : `${p}/`
	}

	const showTileStar = () => canFavorite() && !isMobileTapOpen()

	return (
		<>
			<Show
				when={isList()}
				fallback={
					<div
						class="fs-grid-item"
						classList={itemClassList('fs-grid-item')}
						role="button"
						tabIndex={0}
						data-fs-path={itemPathAttr()}
						draggable={dragEnabled()}
						onDragStart={(e) => {
							if (!dragEnabled()) return
							props.onDragStartItem?.(props.fsElement, e)
						}}
						onDragOver={(e) => {
							if (!props.dropTarget) return
							props.onDragOverItem?.(e)
						}}
						onDragLeave={(e) => {
							if (!props.dropTarget) return
							props.onDragLeaveItem?.(e)
						}}
						onDrop={(e) => {
							if (!props.dropTarget) return
							props.onDropItem?.(e)
						}}
						onMouseDown={(e) => {
							// Prevent native text highlight when clicking / dragging tiles.
							if (e.button === 0) e.preventDefault()
						}}
						onClick={handleItemClick}
						onDblClick={handleItemDblClick}
						onContextMenu={handleContextMenu}
						onTouchStart={handleTouchStart}
						onTouchEnd={handleTouchEnd}
						onTouchMove={handleTouchMove}
						onTouchCancel={handleTouchEnd}
						onKeyDown={handleItemKeyDown}
					>
						<Show when={showTileStar()}>
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
										<FluentIcon
											name="starFilled"
											size={18}
											class="fs-star-icon fs-star-icon--active"
										/>
									) : (
										<FluentIcon name="star" size={18} class="fs-star-icon" />
									)}
								</IconButton>
							</div>
						</Show>

						<FileTypeIcon
							name={props.fsElement.name}
							isFile={props.fsElement.is_file}
							thumbUrl={thumbUrl()}
							size={64}
						/>

						<div class="fs-grid-item__name" title={displayName()}>
							{displayName()}
						</div>
					</div>
				}
			>
				<div
					class="fs-list-item"
					classList={itemClassList('fs-list-item')}
					role="button"
					tabIndex={0}
					data-fs-path={itemPathAttr()}
					draggable={dragEnabled()}
					onDragStart={(e) => {
						if (!dragEnabled()) return
						props.onDragStartItem?.(props.fsElement, e)
					}}
					onDragOver={(e) => {
						if (!props.dropTarget) return
						props.onDragOverItem?.(e)
					}}
					onDragLeave={(e) => {
						if (!props.dropTarget) return
						props.onDragLeaveItem?.(e)
					}}
					onDrop={(e) => {
						if (!props.dropTarget) return
						props.onDropItem?.(e)
					}}
					onMouseDown={(e) => {
						if (e.button === 0) e.preventDefault()
					}}
					onClick={handleItemClick}
					onDblClick={handleItemDblClick}
					onContextMenu={handleContextMenu}
					onTouchStart={handleTouchStart}
					onTouchEnd={handleTouchEnd}
					onTouchMove={handleTouchMove}
					onTouchCancel={handleTouchEnd}
					onKeyDown={handleItemKeyDown}
				>
					<FileTypeIcon
						name={props.fsElement.name}
						isFile={props.fsElement.is_file}
						thumbUrl={thumbUrl()}
						size={40}
					/>
					<div class="fs-list-item__body">
						<div class="fs-list-item__name" title={displayName()}>
							{displayName()}
						</div>
						<div class="fs-list-item__meta">
							<span>{sizeLabel()}</span>
							<Show when={mtimeLabel()}>
								<span class="fs-list-item__dot">·</span>
								<span>{mtimeLabel()}</span>
							</Show>
						</div>
					</div>
					<div class="fs-list-item__actions">
						<Show when={canFavorite()}>
							<div
								class="fs-list-item__star"
								classList={{ 'fs-list-item__star--active': favorited() }}
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
										<FluentIcon
											name="starFilled"
											size={18}
											class="fs-star-icon fs-star-icon--active"
										/>
									) : (
										<FluentIcon name="star" size={18} class="fs-star-icon" />
									)}
								</IconButton>
							</div>
						</Show>
						<Show when={!isParentNav()}>
							<div class="fs-list-item__delete">
								<IconButton
									size="small"
									onClick={(e) => {
										e.stopPropagation()
										e.preventDefault()
										openActionConfirmDialog()
									}}
									aria-label={
										props.trashMode ? 'Delete forever' : 'Delete'
									}
									title={
										props.trashMode ? 'Delete forever' : 'Delete'
									}
								>
									{props.trashMode ? (
										<FluentIcon
											name="deleteDismiss"
											size={18}
											class="fs-delete-icon"
										/>
									) : (
										<FluentIcon
											name="delete"
											size={18}
											class="fs-delete-icon"
										/>
									)}
								</IconButton>
							</div>
						</Show>
					</div>
				</div>
			</Show>

			<MenuMUI
				open={openMore()}
				onClose={handleCloseMore}
				anchorReference="anchorPosition"
				anchorPosition={menuPos() || { top: 0, left: 0 }}
				transformOrigin={{ vertical: 'top', horizontal: 'left' }}
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
									<FluentIcon name="eye" size={20} />
								</ListItemIcon>
								<ListItemText>Open</ListItemText>
							</MenuItem>

							<Show when={canFavorite()}>
								<MenuItem onClick={toggleFavorite}>
									<ListItemIcon>
										{favorited() ? (
											<FluentIcon name="starFilled" size={20} />
										) : (
											<FluentIcon name="star" size={20} />
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
									<FluentIcon name="info" size={20} />
								</ListItemIcon>
								<ListItemText>Info</ListItemText>
							</MenuItem>

							<MenuItem
								onClick={download}
								disabled={isParentNav() || isDownloading()}
							>
								<ListItemIcon>
									<FluentIcon name="arrowDownload" size={20} />
								</ListItemIcon>
								<ListItemText>Download</ListItemText>
							</MenuItem>

							<MenuItem onClick={rename}>
								<ListItemIcon>
									<FluentIcon name="rename" size={20} />
								</ListItemIcon>
								<ListItemText>Rename</ListItemText>
							</MenuItem>

							<MenuItem onClick={copyTo}>
								<ListItemIcon>
									<FluentIcon name="copy" size={20} />
								</ListItemIcon>
								<ListItemText>Copy to…</ListItemText>
							</MenuItem>

							<MenuItem onClick={moveTo}>
								<ListItemIcon>
									<FluentIcon name="arrowMove" size={20} />
								</ListItemIcon>
								<ListItemText>Move to…</ListItemText>
							</MenuItem>

							<Show when={canShare()}>
								<MenuItem onClick={openShare}>
									<ListItemIcon>
										<FluentIcon name="link" size={20} />
									</ListItemIcon>
									<ListItemText>Share link…</ListItemText>
								</MenuItem>
							</Show>

							<MenuItem onClick={openActionConfirmDialog}>
								<ListItemIcon>
									<FluentIcon name="delete" size={20} />
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
							<FluentIcon name="arrowUndo" size={20} />
						</ListItemIcon>
						<ListItemText>Restore</ListItemText>
					</MenuItem>
					<MenuItem onClick={openActionConfirmDialog}>
						<ListItemIcon>
							<FluentIcon name="deleteDismiss" size={20} />
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
