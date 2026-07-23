import { Routes, Route, Navigate } from '@solidjs/router'
import { ThemeProvider, createTheme } from '@suid/material'
import { createMemo, onMount } from 'solid-js'

import Login from './pages/Login'
import BasicLayout from './layouts/Basic'
import Storages from './pages/Storages'
import StorageCreateForm from './pages/Storages/StorageCreateForm'
import AlertStack from './components/AlertStack'
import StorageWorkers from './pages/StorageWorkers'
import StorageWorkerCreateForm from './pages/StorageWorkers/StorageWorkerCreateForm'
import Files from './pages/Files'
import UploadFileTo from './pages/Files/UploadFileTo'
import Register from './pages/Register'
import NotFound from './pages/404'
import { initTheme, useThemeMode } from './common/theme'

const lightTheme = createTheme({
	palette: {
		mode: 'light',
		primary: {
			main: '#14635C',
			dark: '#0B3D3A',
			light: '#1F857C',
			contrastText: '#F3F7F5',
		},
		secondary: {
			main: '#E8A838',
			dark: '#C48920',
			light: '#F0C56A',
			contrastText: '#1A1408',
		},
		background: {
			default: '#F3F7F5',
			paper: '#FFFFFF',
		},
		text: {
			primary: '#0F1C1A',
			secondary: '#3D524E',
		},
		divider: 'rgba(15, 28, 26, 0.08)',
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
	shape: { borderRadius: 14 },
	components: sharedComponents('light'),
})

const darkTheme = createTheme({
	palette: {
		mode: 'dark',
		primary: {
			main: '#3DB8A8',
			dark: '#2A8F83',
			light: '#6FD4C6',
			contrastText: '#061412',
		},
		secondary: {
			main: '#E8A838',
			dark: '#C48920',
			light: '#F0C56A',
			contrastText: '#1A1408',
		},
		background: {
			default: '#0B1618',
			paper: '#122226',
		},
		text: {
			primary: '#E8F2F0',
			secondary: '#A8C0BB',
		},
		divider: 'rgba(232, 242, 240, 0.1)',
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
	shape: { borderRadius: 14 },
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
					borderRadius: 12,
					paddingInline: 18,
					boxShadow: 'none',
					'&:hover': {
						boxShadow: isDark
							? '0 8px 20px rgba(61, 184, 168, 0.22)'
							: '0 8px 20px rgba(20, 99, 92, 0.18)',
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
		MuiAppBar: {
			styleOverrides: {
				root: {
					background: isDark
						? 'linear-gradient(120deg, #071314 0%, #0E2A2C 55%, #13403C 100%)'
						: 'linear-gradient(120deg, #0B3D3A 0%, #14635C 55%, #1A7A70 100%)',
					boxShadow: isDark
						? '0 10px 30px rgba(0, 0, 0, 0.45)'
						: '0 10px 30px rgba(11, 61, 58, 0.22)',
				},
			},
		},
	}
}

const App = () => {
	const mode = useThemeMode()
	onMount(initTheme)
	const theme = createMemo(() => (mode() === 'dark' ? darkTheme : lightTheme))

	return (
		<ThemeProvider theme={theme()}>
			<Routes>
				<Route path="/login" component={Login} />
				<Route path="/register" component={Register} />

				<Route path="/" component={BasicLayout}>
					<Route path="/" element={<Navigate href="/storages" />} />
					<Route path="/storages" component={Storages} />
					<Route path="/storages/register" component={StorageCreateForm} />
					<Route path="/storages/:id/files/*path" component={Files} />
					<Route path="/storages/:id/upload_to" component={UploadFileTo} />
					<Route path="/storage_workers" component={StorageWorkers} />
					<Route
						path="/storage_workers/register"
						component={StorageWorkerCreateForm}
					/>
					<Route path="*404" component={NotFound} />
				</Route>
			</Routes>

			<AlertStack />
		</ThemeProvider>
	)
}

export default App
