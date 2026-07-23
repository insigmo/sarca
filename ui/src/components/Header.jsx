import AppBar from '@suid/material/AppBar'
import Toolbar from '@suid/material/Toolbar'
import Typography from '@suid/material/Typography'
import IconButton from '@suid/material/IconButton'
import { A, useNavigate } from '@solidjs/router'
import LogoutIcon from '@suid/icons-material/Logout'
import DarkModeOutlinedIcon from '@suid/icons-material/DarkModeOutlined'
import LightModeOutlinedIcon from '@suid/icons-material/LightModeOutlined'
import Box from '@suid/material/Box'
import { Show } from 'solid-js'

import AppIcon from './AppIcon'
import createLocalStore from '../../libs'
import { toggleThemeMode, useThemeMode } from '../common/theme'

const Header = () => {
	const [_store, setStore] = createLocalStore()
	const navigate = useNavigate()
	const mode = useThemeMode()

	const logout = (_) => {
		setStore('access_token')
		setStore('refresh_token')
		setStore('redirect', '/')

		navigate('/login')
	}

	const iconBtnSx = {
		color: '#E8F5F2',
		bgcolor: 'rgba(255,255,255,0.08)',
		'&:hover': { bgcolor: 'rgba(255,255,255,0.16)' },
	}

	return (
		<AppBar position="fixed" elevation={0}>
			<Toolbar sx={{ justifyContent: 'space-between', minHeight: 64 }}>
				<A href="/">
					<Box sx={{ display: 'flex', alignItems: 'center', gap: 1.25 }}>
						<AppIcon size={34} />
						<Typography
							variant="h5"
							noWrap
							sx={{
								fontFamily: "'Fraunces', Georgia, serif",
								fontWeight: 600,
								letterSpacing: '-0.02em',
								color: '#F3F7F5',
							}}
						>
							Sarca
						</Typography>
					</Box>
				</A>

				<Box sx={{ display: 'flex', alignItems: 'center', gap: 0.75 }}>
					<IconButton
						aria-label={
							mode() === 'dark' ? 'Switch to light theme' : 'Switch to dark theme'
						}
						title={mode() === 'dark' ? 'Light theme' : 'Dark theme'}
						onClick={toggleThemeMode}
						sx={iconBtnSx}
					>
						<Show when={mode() === 'dark'} fallback={<DarkModeOutlinedIcon />}>
							<LightModeOutlinedIcon />
						</Show>
					</IconButton>
					<IconButton aria-label="Log out" onClick={logout} sx={iconBtnSx}>
						<LogoutIcon />
					</IconButton>
				</Box>
			</Toolbar>
		</AppBar>
	)
}

export default Header
