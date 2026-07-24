import { onMount } from 'solid-js'
import { Outlet } from '@solidjs/router'
import Header from '../components/Header'
import BottomNav from '../components/BottomNav'
import SettingsModal from '../components/SettingsModal'
import Box from '@suid/material/Box'
import Container from '@suid/material/Container'
import CssBaseline from '@suid/material/CssBaseline'
import Toolbar from '@suid/material/Toolbar'

import { checkAuth } from '../common/auth_guard'

const BasicLayout = () => {
	onMount(checkAuth)

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
						<Outlet />
					</Container>
				</Box>
			</Box>

			<BottomNav />
			<SettingsModal />
		</>
	)
}

export default BasicLayout
