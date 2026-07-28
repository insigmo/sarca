import { onCleanup, onMount } from 'solid-js'
import { Outlet, useNavigate } from '@solidjs/router'
import Header from '../components/Header'
import SettingsModal from '../components/SettingsModal'
import StorageSettingsModal from '../components/StorageSettingsModal'
import EmailVerifyBanner from '../components/EmailVerifyBanner'
import Box from '@suid/material/Box'
import CssBaseline from '@suid/material/CssBaseline'
import Toolbar from '@suid/material/Toolbar'

import { checkAuth } from '../common/auth_guard'
import { storageSettingsStore } from '../common/storageSettings'
import { filesChromeStore } from '../common/filesChrome'
import { installTextSelectionGuard } from '../common/suppressTextSelection'
import { installAndroidSafeAreaFallbacks } from '../common/androidSafeArea'

const BasicLayout = () => {
	onMount(() => {
		checkAuth()
		installAndroidSafeAreaFallbacks()
		document.body.classList.add('sarca-no-select')
		const stopGuard = installTextSelectionGuard()
		onCleanup(() => {
			stopGuard()
			document.body.classList.remove('sarca-no-select')
		})
	})
	const navigate = useNavigate()
	const { storage, close, patchName } = storageSettingsStore
	const chrome = filesChromeStore

	return (
		<>
			<CssBaseline />
			<Header />
			<Box class="app-shell-root">
				<Toolbar
					class="app-shell-toolbar-spacer"
					sx={{
						minHeight: { xs: 56, sm: 64 },
						paddingTop: 'max(var(--sarca-safe-top), var(--sarca-android-top))',
						boxSizing: 'content-box',
					}}
				/>

				<Box class="app-shell-stage">
					<div class="app-shell-banner">
						<EmailVerifyBanner />
					</div>
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
				onDeleted={(id) => {
					close()
					if (chrome.storageId() === id) {
						navigate('/storages')
					}
				}}
			/>
		</>
	)
}

export default BasicLayout
