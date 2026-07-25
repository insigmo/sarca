import { Show, createEffect, createSignal, onCleanup } from 'solid-js'
import { Portal } from 'solid-js/web'
import { A, useNavigate } from '@solidjs/router'
import IconButton from '@suid/material/IconButton'
import MenuMUI from '@suid/material/Menu'
import MenuItem from '@suid/material/MenuItem'
import ListItemIcon from '@suid/material/ListItemIcon'
import ListItemText from '@suid/material/ListItemText'
import ChevronLeftIcon from '@suid/icons-material/ChevronLeft'
import ChevronRightIcon from '@suid/icons-material/ChevronRight'
import FolderOutlinedIcon from '@suid/icons-material/FolderOutlined'
import StarOutlineIcon from '@suid/icons-material/StarOutline'
import HistoryIcon from '@suid/icons-material/History'
import LinkIcon from '@suid/icons-material/Link'
import DeleteOutlineIcon from '@suid/icons-material/DeleteOutline'
import SettingsOutlinedIcon from '@suid/icons-material/SettingsOutlined'
import LogoutIcon from '@suid/icons-material/Logout'
import AddIcon from '@suid/icons-material/Add'
import CreateNewFolderIcon from '@suid/icons-material/CreateNewFolder'
import UploadFileIcon from '@suid/icons-material/UploadFile'
import DriveFolderUploadIcon from '@suid/icons-material/DriveFolderUpload'
import StorageOutlinedIcon from '@suid/icons-material/StorageOutlined'

import createLocalStore from '../../libs'
import { clearSession } from '../common/auth'
import { settingsStore } from '../common/settings'

const STORAGE_KEY = 'sarca.filesSidebarCollapsed'

/**
 * @typedef {'browse' | 'favorites' | 'recent' | 'shared' | 'trash'} FilesListMode
 * @typedef {'files' | 'storages'} SidebarVariant
 */

const SidebarNewButton = (props) => {
	const [anchorEl, setAnchorEl] = createSignal(null)
	const open = () => Boolean(anchorEl())

	const closeMenu = () => setAnchorEl(null)

	const run = (fn) => {
		closeMenu()
		fn?.()
		props.onAfterAction?.()
	}

	return (
		<>
			<button
				type="button"
				class="files-sidebar__new"
				classList={{ 'files-sidebar__new--collapsed': props.collapsed }}
				aria-label="New"
				title="New"
				aria-haspopup="menu"
				aria-expanded={open()}
				disabled={props.disabled}
				onClick={(e) => {
					if (props.disabled) return
					setAnchorEl(e.currentTarget)
				}}
			>
				<AddIcon fontSize="small" />
				<span class="files-sidebar__new-label">New</span>
			</button>
			<MenuMUI
				anchorEl={anchorEl()}
				open={open()}
				onClose={closeMenu}
				anchorOrigin={{ vertical: 'bottom', horizontal: 'left' }}
				transformOrigin={{ vertical: 'top', horizontal: 'left' }}
			>
				<MenuItem onClick={() => run(props.onCreateFolder)}>
					<ListItemIcon>
						<CreateNewFolderIcon fontSize="small" />
					</ListItemIcon>
					<ListItemText>Create folder</ListItemText>
				</MenuItem>
				<MenuItem onClick={() => run(props.onUploadFile)}>
					<ListItemIcon>
						<UploadFileIcon fontSize="small" />
					</ListItemIcon>
					<ListItemText>Upload file</ListItemText>
				</MenuItem>
				<MenuItem onClick={() => run(props.onUploadFolder)}>
					<ListItemIcon>
						<DriveFolderUploadIcon fontSize="small" />
					</ListItemIcon>
					<ListItemText>Upload folder</ListItemText>
				</MenuItem>
			</MenuMUI>
		</>
	)
}

const SidebarNav = (props) => {
	const item = (mode, label, Icon) => (
		<button
			type="button"
			class="files-sidebar__item"
			classList={{ 'files-sidebar__item--active': props.mode === mode }}
			aria-current={props.mode === mode ? 'page' : undefined}
			aria-label={label}
			title={label}
			onClick={() => props.onSelect(mode)}
		>
			<Icon fontSize="small" />
			<span class="files-sidebar__label">{label}</span>
		</button>
	)

	return (
		<nav
			class="files-sidebar__nav"
			aria-label={props.variant === 'storages' ? 'Storages' : 'Files'}
		>
			<div class="files-sidebar__top">
				<Show when={props.variant === 'files'}>
					<SidebarNewButton
						collapsed={props.collapsed}
						disabled={props.createDisabled}
						onCreateFolder={props.onCreateFolder}
						onUploadFile={props.onUploadFile}
						onUploadFolder={props.onUploadFolder}
						onAfterAction={props.onAfterAction}
					/>
					{item('browse', 'All files', FolderOutlinedIcon)}
					{item('favorites', 'Favorites', StarOutlineIcon)}
					{item('recent', 'Recent', HistoryIcon)}
					{item('shared', 'Shared', LinkIcon)}
					{item('trash', 'Trash', DeleteOutlineIcon)}
				</Show>
				<Show when={props.variant === 'storages'}>
					<A
						href="/storages"
						class="files-sidebar__item files-sidebar__item--active"
						aria-current="page"
						aria-label="Storages"
						title="Storages"
					>
						<StorageOutlinedIcon fontSize="small" />
						<span class="files-sidebar__label">Storages</span>
					</A>
				</Show>
			</div>
			<div class="files-sidebar__bottom">
				<div class="files-sidebar__divider" aria-hidden="true" />
				<button
					type="button"
					class="files-sidebar__item"
					aria-label="Settings"
					title="Settings"
					onClick={props.onOpenSettings}
				>
					<SettingsOutlinedIcon fontSize="small" />
					<span class="files-sidebar__label">Settings</span>
				</button>
				<button
					type="button"
					class="files-sidebar__item files-sidebar__item--danger"
					aria-label="Log out"
					title="Log out"
					onClick={props.onLogout}
				>
					<LogoutIcon fontSize="small" />
					<span class="files-sidebar__label">Log out</span>
				</button>
			</div>
		</nav>
	)
}

/**
 * @param {{
 *   variant?: SidebarVariant,
 *   mode?: FilesListMode,
 *   onSelectMode?: (mode: FilesListMode) => void,
 *   mobileOpen: boolean,
 *   onMobileClose: () => void,
 *   createDisabled?: boolean,
 *   onCreateFolder?: () => void,
 *   onUploadFile?: () => void,
 *   onUploadFolder?: () => void,
 * }} props
 */
const FilesSidebar = (props) => {
	const variant = () => props.variant || 'files'
	const { openSettings, isOpen } = settingsStore
	const navigate = useNavigate()
	const [, setStore] = createLocalStore()
	const [collapsed, setCollapsed] = createSignal(
		typeof localStorage !== 'undefined' && localStorage.getItem(STORAGE_KEY) === '1',
	)

	const toggleCollapsed = () => {
		const next = !collapsed()
		setCollapsed(next)
		if (typeof localStorage !== 'undefined') {
			localStorage.setItem(STORAGE_KEY, next ? '1' : '0')
		}
	}

	const select = (mode) => {
		props.onSelectMode?.(mode)
		props.onMobileClose?.()
	}

	const openSidebarSettings = () => {
		openSettings()
		props.onMobileClose?.()
	}

	const logout = () => {
		props.onMobileClose?.()
		navigate(clearSession(setStore))
	}

	createEffect(() => {
		if (!props.mobileOpen) return

		document.body.style.overflow = 'hidden'

		const closeOnEscape = (event) => {
			if (event.key === 'Escape') props.onMobileClose?.()
		}

		window.addEventListener('keydown', closeOnEscape)
		onCleanup(() => {
			window.removeEventListener('keydown', closeOnEscape)
			if (!isOpen()) {
				document.body.style.overflow = ''
			}
		})
	})

	const navProps = () => ({
		variant: variant(),
		mode: props.mode,
		collapsed: collapsed(),
		createDisabled: props.createDisabled,
		onSelect: select,
		onOpenSettings: openSidebarSettings,
		onLogout: logout,
		onCreateFolder: props.onCreateFolder,
		onUploadFile: props.onUploadFile,
		onUploadFolder: props.onUploadFolder,
		onAfterAction: () => props.onMobileClose?.(),
	})

	return (
		<>
			<aside
				class="files-sidebar files-sidebar--desktop"
				classList={{ 'files-sidebar--collapsed': collapsed() }}
			>
				<div class="files-sidebar__header">
					<IconButton
						size="small"
						aria-label={collapsed() ? 'Expand sidebar' : 'Collapse sidebar'}
						aria-expanded={!collapsed()}
						onClick={toggleCollapsed}
					>
						<Show when={collapsed()} fallback={<ChevronLeftIcon />}>
							<ChevronRightIcon />
						</Show>
					</IconButton>
				</div>
				<SidebarNav {...navProps()} />
			</aside>

			<Show when={props.mobileOpen}>
				<Portal mount={document.body}>
					<div
						class="files-sidebar-backdrop"
						onClick={() => props.onMobileClose?.()}
						role="presentation"
					/>
					<aside
						class="files-sidebar files-sidebar--drawer"
						role="dialog"
						aria-modal="true"
						aria-label={
							variant() === 'storages' ? 'Storages navigation' : 'Files navigation'
						}
					>
						<SidebarNav {...navProps()} collapsed={false} />
					</aside>
				</Portal>
			</Show>
		</>
	)
}

export default FilesSidebar
