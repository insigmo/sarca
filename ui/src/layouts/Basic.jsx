import { onMount } from 'solid-js'
import { Outlet } from '@solidjs/router'
import Header from '../components/Header'
import SideBar from '../components/SideBar'
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

				<Box sx={{ display: 'flex', minHeight: 'calc(100vh - 64px)' }}>
					<SideBar />

					<Container
						maxWidth="lg"
						class="app-shell-main"
						sx={{ pt: 3.5, pb: 5, flex: 1 }}
					>
						<Outlet />
					</Container>
				</Box>
			</Box>
		</>
	)
}

export default BasicLayout
