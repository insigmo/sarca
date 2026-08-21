import { For, Show, createEffect, createSignal, onCleanup } from 'solid-js'
import { Portal } from 'solid-js/web'
import { A, useNavigate } from '@solidjs/router'
import IconButton from '@suid/material/IconButton'
import MenuMUI from '@suid/material/Menu'
import MenuItem from '@suid/material/MenuItem'
import ListItemIcon from '@suid/material/ListItemIcon'
import ListItemText from '@suid/material/ListItemText'

import createLocalStore from '../../libs'
import API from '../api'
import { clearSession } from '../common/auth'
import { settingsStore } from '../common/settings'
import { isNativeClient } from '../common/nativeClient'
import { nativeInvoke } from '../common/nativeBridge'
import { busyStore } from '../common/busyStore'
import { i18n, LOCALES, t } from '../common/i18n'
import { alertStore } from './AlertStack'
import ActionConfirmDialog from './ActionConfirmDialog'
import FluentIcon from './FluentIcon'

const STORAGE_KEY = 'sarca.filesSidebarCollapsed'

/** Shown on Log out / Disconnect while a storage is still being created. */
const sessionLockedHint = () => t('sidebar.creatingStorage')

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
				aria-label={t('files.new')}
				title={t('files.new')}
				aria-haspopup="menu"
				aria-expanded={open()}
				disabled={props.disabled}
				onClick={(e) => {
					if (props.disabled) return
					setAnchorEl(e.currentTarget)
				}}
			>
				<FluentIcon name="add" size={18} />
				<span class="files-sidebar__new-label">{t('files.new')}</span>
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
						<FluentIcon name="folderAdd" size={20} />
					</ListItemIcon>
					<ListItemText>{t('files.createFolder')}</ListItemText>
				</MenuItem>
				<MenuItem onClick={() => run(props.onUploadFile)}>
					<ListItemIcon>
						<FluentIcon name="documentArrowUp" size={20} />
					</ListItemIcon>
					<ListItemText>{t('files.uploadFile')}</ListItemText>
				</MenuItem>
				<MenuItem onClick={() => run(props.onUploadFolder)}>
					<ListItemIcon>
						<FluentIcon name="folderArrowUp" size={20} />
					</ListItemIcon>
					<ListItemText>{t('files.uploadFolder')}</ListItemText>
				</MenuItem>
			</MenuMUI>
		</>
	)
}

/** Compact language picker sitting next to the icon-only Settings button. */
const SidebarLanguageSwitcher = () => {
	const [anchorEl, setAnchorEl] = createSignal(null)
	const open = () => Boolean(anchorEl())
	const closeMenu = () => setAnchorEl(null)
	const current = () => LOCALES.find((l) => l.code === i18n.locale()) || LOCALES[0]

	return (
		<>
			<button
				type="button"
				class="files-sidebar__item"
				aria-label={t('sidebar.language')}
				title={current().label}
				aria-haspopup="menu"
				aria-expanded={open()}
				onClick={(e) => setAnchorEl(e.currentTarget)}
			>
				<FluentIcon name="localLanguage" size={20} />
				<span class="files-sidebar__label">{current().label}</span>
			</button>
			<MenuMUI anchorEl={anchorEl()} open={open()} onClose={closeMenu}>
				<For each={LOCALES}>
					{(entry) => (
						<MenuItem
							selected={entry.code === i18n.locale()}
							lang={entry.code}
							onClick={() => {
								i18n.setLocale(entry.code)
								closeMenu()
							}}
						>
							<ListItemText>{entry.label}</ListItemText>
						</MenuItem>
					)}
				</For>
			</MenuMUI>
		</>
	)
}

const SidebarNav = (props) => {
	// `regular` / `filled` are icon *names* from the FluentIcon table, not SVG
	// markup: FluentIcon renders through innerHTML, so nothing but a key it
	// controls itself may reach it.
	const item = (mode, label, regular, filled) => (
		<button
			type="button"
			class="files-sidebar__item"
			classList={{ 'files-sidebar__item--active': props.mode === mode }}
			aria-current={props.mode === mode ? 'page' : undefined}
			aria-label={label}
			title={label}
			onMouseDown={(e) => {
				if (e.button === 0) e.preventDefault()
			}}
			onClick={() => props.onSelect(mode)}
		>
			<FluentIcon
				name={props.mode === mode ? filled : regular}
				size={20}
			/>
			<span class="files-sidebar__label">{label}</span>
		</button>
	)

	return (
		<nav
			class="files-sidebar__nav"
			aria-label={props.variant === 'storages' ? t('storages.title') : t('files.title')}
		>
			<div class="files-sidebar__top">
				<Show when={props.variant === 'files'}>
					<A
						href="/storages"
						class="files-sidebar__item"
						aria-label={t('storages.title')}
						title={t('storages.title')}
						onClick={() => props.onAfterAction?.()}
					>
						<FluentIcon name="storage" size={20} />
						<span class="files-sidebar__label">{t('storages.title')}</span>
					</A>
					<div class="files-sidebar__divider" aria-hidden="true" />
					<SidebarNewButton
						collapsed={props.collapsed}
						disabled={props.createDisabled}
						onCreateFolder={props.onCreateFolder}
						onUploadFile={props.onUploadFile}
						onUploadFolder={props.onUploadFolder}
						onAfterAction={props.onAfterAction}
					/>
					{item('browse', t('files.allFiles'), 'folder', 'folderFilled')}
					{item('favorites', t('files.favorites'), 'star', 'starFilled')}
					{item('recent', t('files.recent'), 'history', 'historyFilled')}
					{item('shared', t('files.sharedShort'), 'link', 'linkFilled')}
					{item('trash', t('files.trash'), 'delete', 'deleteFilled')}
				</Show>
				<Show when={props.variant === 'storages'}>
					<A
						href="/storages"
						class="files-sidebar__item files-sidebar__item--active"
						aria-current="page"
						aria-label={t('storages.title')}
						title={t('storages.title')}
					>
						<FluentIcon name="storageFilled" size={20} />
						<span class="files-sidebar__label">{t('storages.title')}</span>
					</A>
				</Show>
			</div>
			<div class="files-sidebar__bottom">
				<div class="files-sidebar__divider" aria-hidden="true" />
				<div class="files-sidebar__utility-row">
					<SidebarLanguageSwitcher />
				</div>
				<div class="files-sidebar__utility-row">
					<button
						type="button"
						class="files-sidebar__item"
						aria-label={t('sidebar.session')}
						title={t('sidebar.session')}
						aria-haspopup="menu"
						aria-expanded={props.actionsMenuOpen()}
						onClick={(e) => props.onOpenActionsMenu(e.currentTarget)}
					>
						<FluentIcon name="signOut" size={20} />
						<span class="files-sidebar__label">{t('sidebar.session')}</span>
					</button>
					<button
						type="button"
						class="files-sidebar__item files-sidebar__item--icon-only"
						aria-label={t('sidebar.settings')}
						title={t('sidebar.settings')}
						onClick={props.onOpenSettings}
					>
						<FluentIcon name="settings" size={20} />
					</button>
				</div>
				<MenuMUI
					anchorEl={props.actionsMenuAnchor()}
					open={props.actionsMenuOpen()}
					onClose={props.onCloseActionsMenu}
				>
					<Show when={props.showDisconnect}>
						<MenuItem
							disabled={busyStore.isStorageCreating()}
							title={busyStore.isStorageCreating() ? sessionLockedHint() : undefined}
							onClick={() => {
								props.onCloseActionsMenu()
								props.onDisconnect?.()
							}}
						>
							<ListItemIcon>
								<FluentIcon name="plugDisconnected" size={20} />
							</ListItemIcon>
							<ListItemText
								primary={t('sidebar.disconnect')}
								secondary={busyStore.isStorageCreating() ? sessionLockedHint() : undefined}
							/>
						</MenuItem>
					</Show>
					<MenuItem
						disabled={busyStore.isStorageCreating()}
						title={busyStore.isStorageCreating() ? sessionLockedHint() : undefined}
						onClick={() => {
							props.onCloseActionsMenu()
							props.onLogout?.()
						}}
					>
						<ListItemIcon>
							<FluentIcon name="signOut" size={20} />
						</ListItemIcon>
						<ListItemText
							primary={t('sidebar.logOut')}
							secondary={busyStore.isStorageCreating() ? sessionLockedHint() : undefined}
						/>
					</MenuItem>
				</MenuMUI>
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

	const confirmLogout = () => {
		// The dialog may have been opened before storage creation started, so
		// re-check here as well as on the menu item.
		if (busyStore.isStorageCreating()) return
		// Revoke server-side first; the local clear must not wait on the network.
		API.auth.logout()
		props.onMobileClose?.()
		navigate(clearSession(setStore))
	}

	const showDisconnect = isNativeClient()
	const { addAlert } = alertStore
	const [disconnectConfirmOpen, setDisconnectConfirmOpen] = createSignal(false)
	const [disconnecting, setDisconnecting] = createSignal(false)
	const [logoutConfirmOpen, setLogoutConfirmOpen] = createSignal(false)
	const [actionsMenuAnchor, setActionsMenuAnchor] = createSignal(null)
	const actionsMenuOpen = () => Boolean(actionsMenuAnchor())

	const requestDisconnect = () => {
		props.onMobileClose?.()
		setDisconnectConfirmOpen(true)
	}

	const confirmDisconnect = async () => {
		if (disconnecting() || busyStore.isStorageCreating()) return
		setDisconnecting(true)
		try {
			await nativeInvoke('disconnect')
			setDisconnectConfirmOpen(false)
			// Native side navigates the webview to the connect shell on success.
		} catch (e) {
			addAlert(e?.message || t('errors.disconnectFailed'), 'error')
		} finally {
			setDisconnecting(false)
		}
	}

	const requestLogout = () => {
		setActionsMenuAnchor(null)
		props.onMobileClose?.()
		setLogoutConfirmOpen(true)
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
		onLogout: requestLogout,
		showDisconnect,
		onDisconnect: requestDisconnect,
		onCreateFolder: props.onCreateFolder,
		onUploadFile: props.onUploadFile,
		onUploadFolder: props.onUploadFolder,
		onAfterAction: () => props.onMobileClose?.(),
		actionsMenuAnchor,
		actionsMenuOpen,
		onOpenActionsMenu: (el) => setActionsMenuAnchor(el),
		onCloseActionsMenu: () => setActionsMenuAnchor(null),
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
						<Show
							when={collapsed()}
							fallback={<FluentIcon name="chevronLeft" size={20} />}
						>
							<FluentIcon name="chevronRight" size={20} />
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

			<ActionConfirmDialog
				isOpened={disconnectConfirmOpen()}
				entity={t('confirmDialog.disconnectEntity')}
				action={t('confirmDialog.disconnectAction')}
				actionDescription={
					disconnecting()
						? t('confirmDialog.disconnectBusy')
						: t('confirmDialog.disconnectDescription')
				}
				onConfirm={confirmDisconnect}
				onCancel={() => {
					// Cancel is meaningless once the native call is in flight;
					// closing the dialog would hide the fact it is still running.
					if (disconnecting()) return
					setDisconnectConfirmOpen(false)
				}}
			/>

			<ActionConfirmDialog
				isOpened={logoutConfirmOpen()}
				entity={t('confirmDialog.logoutEntity')}
				action={t('confirmDialog.logoutAction')}
				actionDescription={t('confirmDialog.logoutDescription')}
				onConfirm={confirmLogout}
				onCancel={() => setLogoutConfirmOpen(false)}
			/>
		</>
	)
}

export default FilesSidebar
