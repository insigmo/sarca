import { onCleanup, onMount } from 'solid-js'
import { Outlet, useNavigate } from '@solidjs/router'
import SettingsModal from '../components/SettingsModal'
import StorageSettingsModal from '../components/StorageSettingsModal'
import Box from '@suid/material/Box'
import CssBaseline from '@suid/material/CssBaseline'

import { checkAuth } from '../common/auth_guard'
import { storageSettingsStore } from '../common/storageSettings'
import { storagesStore } from '../common/storagesStore'
import { filesChromeStore } from '../common/filesChrome'
import { installTextSelectionGuard } from '../common/suppressTextSelection'
import { installAndroidSafeAreaFallbacks } from '../common/androidSafeArea'
import { installWheelScrollFix } from '../common/wheelScroll'

const BasicLayout = () => {
	onMount(() => {
		checkAuth()
		installAndroidSafeAreaFallbacks()
		document.body.classList.add('sarca-no-select')
		const stopGuard = installTextSelectionGuard()
		const stopWheelFix = installWheelScrollFix()
		onCleanup(() => {
			stopGuard()
			stopWheelFix()
			document.body.classList.remove('sarca-no-select')
		})
	})
	const navigate = useNavigate()
	const { storage, close, patchName } = storageSettingsStore
	const { refreshStorages } = storagesStore
	const chrome = filesChromeStore

	return (
		<>
			<CssBaseline />
			{/* The fixed app bar is gone: it spent 56-64px of every screen on a
			    wordmark and a search field, and the search now sits in the files
			    toolbar beside the breadcrumb it filters. The safe-area inset it
			    used to absorb moves onto the shell root. */}
			<Box class="app-shell-root">
				<Box class="app-shell-stage">
					<div class="app-shell-outlet app-shell-main">
						<Outlet />
					</div>
				</Box>
			</Box>

			<SettingsModal />
			<StorageSettingsModal
				storage={storage()}
				onClose={close}
				onRenamed={(updated) => {
					patchName(updated)
					if (chrome.storageId() === updated.id) {
						chrome.setStorageName(updated.name)
					}
				}}
				onDeleted={async (id) => {
					close()
					if (chrome.storageId() === id) {
						navigate('/storages')
					}
					// Storages page reads the same store, so this refresh alone
					// updates its grid; the effect there sends the user to /setup
					// once the list is confirmed empty.
					const list = await refreshStorages()
					if (!list.length) {
						navigate('/setup', { replace: true })
					}
				}}
			/>
		</>
	)
}

export default BasicLayout
