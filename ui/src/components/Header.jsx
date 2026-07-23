import AppBar from '@suid/material/AppBar'
import Toolbar from '@suid/material/Toolbar'
import Typography from '@suid/material/Typography'
import IconButton from '@suid/material/IconButton'
import { A, useNavigate } from '@solidjs/router'
import LogoutIcon from '@suid/icons-material/Logout'
import Box from '@suid/material/Box'

import AppIcon from './AppIcon'
import createLocalStore from '../../libs'

const Header = () => {
	const [_store, setStore] = createLocalStore()
	const navigate = useNavigate()

	const logout = (_) => {
		setStore('access_token')
		setStore('refresh_token')
		setStore('redirect', '/')

		navigate('/login')
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

				<IconButton
					aria-label="Log out"
					onClick={logout}
					sx={{
						color: '#E8F5F2',
						bgcolor: 'rgba(255,255,255,0.08)',
						'&:hover': { bgcolor: 'rgba(255,255,255,0.16)' },
					}}
				>
					<LogoutIcon />
				</IconButton>
			</Toolbar>
		</AppBar>
	)
}

export default Header
