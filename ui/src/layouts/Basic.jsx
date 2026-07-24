import { onMount } from 'solid-js'
import { Outlet, useNavigate } from '@solidjs/router'
import Header from '../components/Header'
import BottomNav from '../components/BottomNav'
import SettingsModal from '../components/SettingsModal'
import StorageSettingsModal from '../components/StorageSettingsModal'
import EmailVerifyBanner from '../components/EmailVerifyBanner'
import Box from '@suid/material/Box'
import Container from '@suid/material/Container'
import CssBaseline from '@suid/material/CssBaseline'
import Toolbar from '@suid/material/Toolbar'

import { checkAuth } from '../common/auth_guard'
import { storageSettingsStore } from '../common/storageSettings'
import { filesChromeStore } from '../common/filesChrome'

const BasicLayout = () => {
	onMount(checkAuth)
	const navigate = useNavigate()
	const { storage, close, patchName } = storageSettingsStore
	const chrome = filesChromeStore

	return (
		<>
			<CssBaseline />
			<Header />
			<Box>
				<Toolbar />

				<Box sx={{ minHeight: 'calc(100vh - 64px)' }}>
					<Container
						maxWidth="lg"
						class="app-shell-main"
						sx={{ pt: { xs: 1.5, sm: 2 }, pb: 5 }}
					>
						<EmailVerifyBanner />
						<Outlet />
					</Container>
				</Box>
			</Box>

			<BottomNav />
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
