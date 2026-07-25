import { Show, createEffect, createSignal, onCleanup } from 'solid-js'
import IconButton from '@suid/material/IconButton'
import ChevronLeftIcon from '@suid/icons-material/ChevronLeft'
import ChevronRightIcon from '@suid/icons-material/ChevronRight'
import FolderOutlinedIcon from '@suid/icons-material/FolderOutlined'
import StarOutlineIcon from '@suid/icons-material/StarOutline'
import HistoryIcon from '@suid/icons-material/History'
import LinkIcon from '@suid/icons-material/Link'
import DeleteOutlineIcon from '@suid/icons-material/DeleteOutline'
import SettingsOutlinedIcon from '@suid/icons-material/SettingsOutlined'

import { settingsStore } from '../common/settings'

const STORAGE_KEY = 'sarca.filesSidebarCollapsed'

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
		<nav class="files-sidebar__nav" aria-label="Files">
			<div class="files-sidebar__top">
				{item('browse', 'All files', FolderOutlinedIcon)}
				{item('favorites', 'Favorites', StarOutlineIcon)}
				{item('recent', 'Recent', HistoryIcon)}
				{item('shared', 'Shared', LinkIcon)}
				{item('trash', 'Trash', DeleteOutlineIcon)}
			</div>
			<div class="files-sidebar__bottom">
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
			</div>
		</nav>
	)
}

/**
 * @typedef {'browse' | 'favorites' | 'recent' | 'shared' | 'trash'} FilesListMode
 * @param {{
 *   mode: FilesListMode,
 *   onSelectMode: (mode: FilesListMode) => void,
 *   mobileOpen: boolean,
 *   onMobileClose: () => void,
 *   collapsed?: boolean,
 * }} props
 */
const FilesSidebar = (props) => {
	const { openSettings, isOpen } = settingsStore
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
		props.onSelectMode(mode)
		props.onMobileClose?.()
	}

	const openSidebarSettings = () => {
		openSettings()
		props.onMobileClose?.()
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
						onClick={toggleCollapsed}
					>
						<Show when={collapsed()} fallback={<ChevronLeftIcon />}>
							<ChevronRightIcon />
						</Show>
					</IconButton>
				</div>
				<SidebarNav
					mode={props.mode}
					onSelect={select}
					onOpenSettings={openSidebarSettings}
				/>
			</aside>

			<Show when={props.mobileOpen}>
				<div
					class="files-sidebar-backdrop"
					onClick={() => props.onMobileClose?.()}
					role="presentation"
				/>
				<aside
					class="files-sidebar files-sidebar--drawer"
					role="dialog"
					aria-modal="true"
					aria-label="Files navigation"
				>
					<SidebarNav
						mode={props.mode}
						onSelect={select}
						onOpenSettings={openSidebarSettings}
					/>
				</aside>
			</Show>
		</>
	)
}

export default FilesSidebar
