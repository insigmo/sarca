import { onMount } from 'solid-js'
import Box from '@suid/material/Box'
import TextField from '@suid/material/TextField'
import Button from '@suid/material/Button'
import Paper from '@suid/material/Paper'
import Stack from '@suid/material/Stack'
import createLocalStore from '../../libs'
import { A, useNavigate } from '@solidjs/router'

import API from '../api'
import logoUrl from '../assets/logo.svg'

const Login = () => {
	const [store, setStore] = createLocalStore()
	const navigate = useNavigate()

	onMount(() => {
		if (store.access_token) {
			navigate('/')
		}
	})

	/**
	 *
	 * @param {SubmitEvent} event
	 */
	const handleSubmit = async (event) => {
		event.preventDefault()
		const data = new FormData(event.currentTarget)
		const email = data.get('email')
		const password = data.get('password')

		const tokenData = await API.auth.login(email, password)

		setStore('access_token', tokenData.access_token)
		setStore('refresh_token', tokenData.refresh_token)
		setStore('user', { email })

		const redirect_url = store.redirect || '/'
		navigate(redirect_url)
	}

	return (
		<div class="auth-page">
			<Paper class="auth-card" elevation={0}>
				<Box
					component="form"
					onSubmit={handleSubmit}
					sx={{
						px: { xs: 3, sm: 4.5 },
						py: { xs: 3.5, sm: 4 },
						display: 'flex',
						flexDirection: 'column',
						gap: 2,
					}}
				>
					<div class="auth-brand">
						<img src={logoUrl} alt="Sarca" />
						<h1>Sarca</h1>
						<p>Sign in to your cloud storage</p>
					</div>

					<TextField
						name="email"
						label="Email"
						type="email"
						autoComplete="email"
						required
					/>
					<TextField
						name="password"
						label="Password"
						type="password"
						autoComplete="current-password"
						required
					/>

					<Stack spacing={1.5} sx={{ mt: 1 }}>
						<Button type="submit" variant="contained" color="secondary" size="large">
							Sign in
						</Button>
						<A class="default-link" href="/register" style={{ 'text-align': 'center' }}>
							Don't have an account yet? Register
						</A>
					</Stack>
				</Box>
			</Paper>
		</div>
	)
}

export default Login
