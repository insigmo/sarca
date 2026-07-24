import { Routes, Route, Navigate } from '@solidjs/router'
import { ThemeProvider, createTheme } from '@suid/material'
import { Show, onMount } from 'solid-js'

import Login from './pages/Login'
import BasicLayout from './layouts/Basic'
import Storages from './pages/Storages'
import StorageCreateForm from './pages/Storages/StorageCreateForm'
import AlertStack from './components/AlertStack'
import Files from './pages/Files'
import UploadFileTo from './pages/Files/UploadFileTo'
import Register from './pages/Register'
import NotFound from './pages/404'
import { initTheme, useThemeMode } from './common/theme'
import { settingsStore } from './common/settings'

/** Opens settings modal then redirects to storages (workers live in Settings now). */
const WorkersRedirect = () => {
	onMount(() => settingsStore.openSettings())
	return <Navigate href="/storages" />
}

/**
 * Palette colors must stay parseable by SUID's colorManipulator (hex/rgb/rgba).
 * CSS vars / color-mix() throw at runtime (Button/IconButton call alpha() on them).
 * Theme toggle remounts ThemeProvider (keyed Show) because @suid/system styled()
 * caches the first theme object in a local closure.
 */
const lightTheme = createTheme({
	palette: {
		mode: 'light',
		primary: {
			main: '#5B6CFF',
			dark: '#3D4AD6',
			light: '#8B9BFF',
			contrastText: '#F7F5FB',
		},
		secondary: {
			main: '#D9A441',
			dark: '#B8862E',
			light: '#F0D089',
			contrastText: '#1A1408',
		},
		background: {
			default: '#ECE8F4',
			paper: '#F7F5FB',
		},
		text: {
			primary: '#1A1F36',
			secondary: '#6B7190',
		},
		divider: 'rgba(26, 31, 54, 0.08)',
	},
	typography: {
		fontFamily: "'Plus Jakarta Sans', 'Segoe UI', sans-serif",
		h1: { fontFamily: "'Fraunces', Georgia, serif", fontWeight: 600 },
		h2: { fontFamily: "'Fraunces', Georgia, serif", fontWeight: 600 },
		h3: { fontFamily: "'Fraunces', Georgia, serif", fontWeight: 600 },
		h4: { fontFamily: "'Fraunces', Georgia, serif", fontWeight: 600 },
		h5: { fontFamily: "'Fraunces', Georgia, serif", fontWeight: 600 },
		h6: { fontFamily: "'Fraunces', Georgia, serif", fontWeight: 600 },
		button: { textTransform: 'none', fontWeight: 700, letterSpacing: 0.2 },
	},
	shape: { borderRadius: 20 },
	components: sharedComponents('light'),
})

const darkTheme = createTheme({
	palette: {
		mode: 'dark',
		primary: {
			main: '#8B9BFF',
			dark: '#5B6CFF',
			light: '#B0BBFF',
			contrastText: '#0A0E28',
		},
		secondary: {
			main: '#D9A441',
			dark: '#B8862E',
			light: '#F0C56A',
			contrastText: '#1A1408',
		},
		background: {
			default: '#0D1230',
			paper: '#141A3A',
		},
		text: {
			primary: '#EEF0FF',
			secondary: '#9AA0C4',
		},
		divider: 'rgba(255, 255, 255, 0.1)',
	},
	typography: {
		fontFamily: "'Plus Jakarta Sans', 'Segoe UI', sans-serif",
		h1: { fontFamily: "'Fraunces', Georgia, serif", fontWeight: 600 },
		h2: { fontFamily: "'Fraunces', Georgia, serif", fontWeight: 600 },
		h3: { fontFamily: "'Fraunces', Georgia, serif", fontWeight: 600 },
		h4: { fontFamily: "'Fraunces', Georgia, serif", fontWeight: 600 },
		h5: { fontFamily: "'Fraunces', Georgia, serif", fontWeight: 600 },
		h6: { fontFamily: "'Fraunces', Georgia, serif", fontWeight: 600 },
		button: { textTransform: 'none', fontWeight: 700, letterSpacing: 0.2 },
	},
	shape: { borderRadius: 20 },
	components: sharedComponents('dark'),
})

/**
 * @param {'light' | 'dark'} mode
 */
function sharedComponents(mode) {
	const isDark = mode === 'dark'
	return {
		MuiButton: {
			styleOverrides: {
				root: {
					borderRadius: 16,
					paddingInline: 18,
					boxShadow: 'none',
					'&:hover': {
						boxShadow: isDark
							? '0 8px 24px rgba(139, 155, 255, 0.22)'
							: '0 8px 24px rgba(91, 108, 255, 0.18)',
					},
				},
			},
		},
		MuiPaper: {
			styleOverrides: {
				root: { backgroundImage: 'none' },
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
					borderRadius: 18,
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
	onMount(initTheme)

	return (
		<Show when={mode()} keyed>
			{(m) => (
				<ThemeProvider theme={m === 'dark' ? darkTheme : lightTheme}>
					<Routes>
						<Route path="/login" component={Login} />
						<Route path="/register" component={Register} />

						<Route path="/" component={BasicLayout}>
							<Route path="/" element={<Navigate href="/storages" />} />
							<Route path="/storages" component={Storages} />
							<Route path="/storages/register" component={StorageCreateForm} />
							<Route path="/storages/:id/files/*path" component={Files} />
							<Route path="/storages/:id/upload_to" component={UploadFileTo} />
							<Route path="/storage_workers" component={WorkersRedirect} />
							<Route path="/storage_workers/register" component={WorkersRedirect} />
							<Route path="*404" component={NotFound} />
						</Route>
					</Routes>

					<AlertStack />
				</ThemeProvider>
			)}
		</Show>
	)
}

export default App
