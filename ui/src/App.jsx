import { Routes, Route, Navigate } from '@solidjs/router'
import { ThemeProvider, createTheme } from '@suid/material'
import { Show, onMount } from 'solid-js'

import Login from './pages/Login'
import BasicLayout from './layouts/Basic'
import Storages from './pages/Storages'
import SetupWizard from './pages/Setup'
import AlertStack from './components/AlertStack'
import UploadManager from './components/UploadManager'
import Files from './pages/Files'
import PublicShare from './pages/PublicShare'
import NotFound from './pages/404'
import { initTheme, useThemeMode } from './common/theme'
import { bindOpenSettingsDeepLink } from './common/nativeClient'
import { installAndroidSafeAreaFallbacks } from './common/androidSafeArea'
import AppLockGate from './components/AppLockGate'

/** Legacy workers routes → storages (bot is in storage settings). */
const WorkersRedirect = () => {
	return <Navigate href="/storages" />
}

const fontFamily = "'Source Sans 3', 'Segoe UI', system-ui, sans-serif"

/**
 * Palette colors must stay parseable by SUID's colorManipulator (hex/rgb/rgba).
 * CSS vars / color-mix() throw at runtime (Button/IconButton call alpha() on them).
 * Theme toggle remounts ThemeProvider (keyed Show) because @suid/system styled()
 * caches the first theme object in a local closure.
 */
/** Windows File Explorer / Fluent — soft gray shell, white panels, #0078D4. */
const lightTheme = createTheme({
	palette: {
		mode: 'light',
		primary: {
			main: '#0078D4',
			dark: '#005A9E',
			light: '#2B88D8',
			contrastText: '#FFFFFF',
		},
		secondary: {
			main: '#0078D4',
			dark: '#004578',
			light: '#2B88D8',
			contrastText: '#FFFFFF',
		},
		background: {
			default: '#EEF0F2',
			paper: '#FFFFFF',
		},
		text: {
			primary: '#1B1A19',
			secondary: '#605E5C',
		},
		divider: 'rgba(0, 0, 0, 0.08)',
	},
	typography: {
		fontFamily,
		h1: { fontFamily, fontWeight: 600 },
		h2: { fontFamily, fontWeight: 600 },
		h3: { fontFamily, fontWeight: 600 },
		h4: { fontFamily, fontWeight: 600 },
		h5: { fontFamily, fontWeight: 600 },
		h6: { fontFamily, fontWeight: 600 },
		button: { textTransform: 'none', fontWeight: 600, letterSpacing: 0.15 },
	},
	shape: { borderRadius: 8 },
	components: sharedComponents('light'),
})

/** Windows 11 dark File Explorer / Fluent dark — #202020 shell, #60CDFF accent. */
const darkTheme = createTheme({
	palette: {
		mode: 'dark',
		primary: {
			main: '#60CDFF',
			dark: '#0078D4',
			light: '#4CC2FF',
			contrastText: '#0A0A0A',
		},
		secondary: {
			main: '#0078D4',
			dark: '#005A9E',
			light: '#60CDFF',
			contrastText: '#FFFFFF',
		},
		background: {
			default: '#202020',
			paper: '#2B2B2B',
		},
		text: {
			primary: '#FFFFFF',
			secondary: '#E0E0E0',
		},
		divider: 'rgba(255, 255, 255, 0.08)',
	},
	typography: {
		fontFamily,
		h1: { fontFamily, fontWeight: 600 },
		h2: { fontFamily, fontWeight: 600 },
		h3: { fontFamily, fontWeight: 600 },
		h4: { fontFamily, fontWeight: 600 },
		h5: { fontFamily, fontWeight: 600 },
		h6: { fontFamily, fontWeight: 600 },
		button: { textTransform: 'none', fontWeight: 600, letterSpacing: 0.15 },
	},
	shape: { borderRadius: 8 },
	components: sharedComponents('dark'),
})

const muiThemes = {
	light: lightTheme,
	dark: darkTheme,
}

/**
 * @param {'light' | 'dark'} mode
 */
function sharedComponents(mode) {
	const hoverShadow =
		mode === 'dark'
			? '0 4px 14px rgba(96, 205, 255, 0.22)'
			: '0 4px 14px rgba(0, 120, 212, 0.2)'
	return {
		MuiButton: {
			styleOverrides: {
				root: {
					borderRadius: 8,
					paddingInline: 16,
					boxShadow: 'none',
					'&:hover': {
						boxShadow: hoverShadow,
					},
				},
			},
		},
		MuiPaper: {
			styleOverrides: {
				root: {
					backgroundImage: 'none',
					boxShadow:
						mode === 'dark'
							? '0 8px 24px rgba(0, 0, 0, 0.35), 0 1.5px 4px rgba(0, 0, 0, 0.2)'
							: '0 8px 24px rgba(0, 0, 0, 0.06), 0 1.5px 4px rgba(0, 0, 0, 0.04)',
				},
			},
		},
		MuiTextField: {
			defaultProps: {
				variant: 'outlined',
				fullWidth: true,
			},
		},
		MuiFab: {
			styleOverrides: {
				root: {
					borderRadius: 12,
				},
				extended: {
					borderRadius: 999,
				},
			},
		},
		MuiAppBar: {
			styleOverrides: {
				root: {
					background: 'transparent',
					boxShadow: 'none',
				},
			},
		},
	}
}

const App = () => {
	const mode = useThemeMode()
	onMount(() => {
		initTheme()
		installAndroidSafeAreaFallbacks()
		return bindOpenSettingsDeepLink()
	})

	return (
		<Show when={mode()} keyed>
			{(m) => (
				<ThemeProvider theme={muiThemes[m] ?? lightTheme}>
					<AppLockGate>
						<Routes>
							<Route path="/login" component={Login} />
							<Route path="/s/:token" component={PublicShare} />

							<Route path="/" component={BasicLayout}>
								<Route path="/" element={<Navigate href="/storages" />} />
								<Route path="/storages" component={Storages} />
								<Route path="/storages/register" component={SetupWizard} />
								<Route path="/setup" component={SetupWizard} />
								<Route path="/storages/:id/files/*path" component={Files} />
								<Route path="/storage_workers" component={WorkersRedirect} />
								<Route path="/storage_workers/register" component={WorkersRedirect} />
								<Route path="*404" component={NotFound} />
							</Route>
						</Routes>

						<AlertStack />
						<UploadManager />
					</AppLockGate>
				</ThemeProvider>
			)}
		</Show>
	)
}

export default App
