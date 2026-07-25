import AppBar from '@suid/material/AppBar'
import Toolbar from '@suid/material/Toolbar'
import Typography from '@suid/material/Typography'
import IconButton from '@suid/material/IconButton'
import TextField from '@suid/material/TextField'
import InputAdornment from '@suid/material/InputAdornment'
import { A } from '@solidjs/router'
import SearchIcon from '@suid/icons-material/Search'
import ClearIcon from '@suid/icons-material/Clear'
import Box from '@suid/material/Box'
import { Show } from 'solid-js'

import AppIcon from './AppIcon'
import { filesChromeStore } from '../common/filesChrome'

const Header = () => {
	const chrome = filesChromeStore

	return (
		<AppBar position="fixed" elevation={0} class="sarca-appbar">
			<Toolbar
				sx={{
					justifyContent: 'space-between',
					minHeight: { xs: 56, sm: 64 },
					gap: 1.5,
					px: { xs: 1.5, sm: 2 },
				}}
			>
				<A href="/" style={{ 'min-width': 0, 'flex-shrink': 1 }}>
					<Box sx={{ display: 'flex', alignItems: 'center', gap: 1.25, minWidth: 0 }}>
						<AppIcon size={34} />
						<Box class="header-brand-text" sx={{ minWidth: 0 }}>
							<Typography
								variant="h5"
								noWrap
								sx={{
									fontFamily: "'Fraunces', Georgia, serif",
									fontWeight: 600,
									letterSpacing: '-0.02em',
									color: 'var(--sarca-header-ink)',
									lineHeight: 1.15,
								}}
							>
								Sarca
							</Typography>
							<Show when={chrome.active() && chrome.storageName()}>
								<Typography
									class="header-storage-name"
									noWrap
									sx={{
										fontSize: '0.78rem',
										fontWeight: 600,
										color: 'var(--sarca-ink-soft)',
										lineHeight: 1.2,
										maxWidth: { sm: 140, md: 220 },
									}}
								>
									{chrome.storageName()}
								</Typography>
							</Show>
						</Box>
					</Box>
				</A>

				<Show when={chrome.active()}>
					<Box
						class="search-pill header-search"
						sx={{
							flex: 1,
							maxWidth: 480,
							mx: { xs: 0.5, sm: 2 },
							minWidth: 0,
						}}
					>
						<TextField
							fullWidth
							size="small"
							placeholder="Search.."
							value={chrome.searchQuery()}
							onChange={(e) => chrome.setSearchQuery(e.target.value)}
							onKeyDown={(e) => {
								if (e.key === 'Enter') chrome.runSearch()
							}}
							InputProps={{
								startAdornment: (
									<InputAdornment position="start">
										<SearchIcon fontSize="small" />
									</InputAdornment>
								),
								endAdornment: (
									<InputAdornment position="end">
										<Show when={chrome.searchQuery() || chrome.isSearching()}>
											<IconButton size="small" onClick={chrome.clearSearch}>
												<ClearIcon fontSize="small" />
											</IconButton>
										</Show>
									</InputAdornment>
								),
							}}
						/>
					</Box>
				</Show>
			</Toolbar>
		</AppBar>
	)
}

export default Header
