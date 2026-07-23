import { Routes, Route, Navigate } from '@solidjs/router'
import { ThemeProvider, createTheme } from '@suid/material'

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

const theme = createTheme({
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
	shape: {
		borderRadius: 14,
	},
	components: {
		MuiButton: {
			styleOverrides: {
				root: {
					borderRadius: 12,
					paddingInline: 18,
					boxShadow: 'none',
					'&:hover': { boxShadow: '0 8px 20px rgba(20, 99, 92, 0.18)' },
				},
				containedSecondary: {
					'&:hover': { boxShadow: '0 8px 20px rgba(232, 168, 56, 0.28)' },
				},
			},
		},
		MuiPaper: {
			styleOverrides: {
				root: {
					backgroundImage: 'none',
				},
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
					background:
						'linear-gradient(120deg, #0B3D3A 0%, #14635C 55%, #1A7A70 100%)',
					boxShadow: '0 10px 30px rgba(11, 61, 58, 0.22)',
				},
			},
		},
	},
})

const App = () => {
	return (
		<ThemeProvider theme={theme}>
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
