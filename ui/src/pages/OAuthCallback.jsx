import { Show, createSignal, onMount } from 'solid-js'
import Box from '@suid/material/Box'
import Button from '@suid/material/Button'
import Paper from '@suid/material/Paper'
import Stack from '@suid/material/Stack'
import CircularProgress from '@suid/material/CircularProgress'
import { useNavigate, useSearchParams } from '@solidjs/router'

import API from '../api'
import createLocalStore from '../../libs'
import { alertStore } from '../components/AlertStack'
import logoUrl from '../assets/logo.svg'

/**
 * Exchanges the OAuth one-time code for JWTs and enters the app.
 * Accepts `?code=` or `?oauth_token=` (backend MVP variants).
 */
const OAuthCallback = () => {
	const { addAlert } = alertStore
	const navigate = useNavigate()
	const [searchParams] = useSearchParams()
	const [, setStore] = createLocalStore()

	/** @type {[import('solid-js').Accessor<'loading'|'fail'>, any]} */
	const [phase, setPhase] = createSignal('loading')
	const [errorMsg, setErrorMsg] = createSignal('')

	onMount(async () => {
		const code = String(
			searchParams.code || searchParams.oauth_token || '',
		).trim()

		if (!code) {
			setPhase('fail')
			setErrorMsg('Missing OAuth code. Try signing in again.')
			return
		}

		try {
			const tokenData = await API.auth.exchangeOAuth(code)
			setStore('access_token', tokenData.access_token)
			setStore('refresh_token', tokenData.refresh_token)
			setStore('user', {
				email: tokenData.email,
				email_verified: tokenData.email_verified,
			})
			addAlert('Signed in', 'success')
			navigate('/', { replace: true })
		} catch (err) {
			setPhase('fail')
			setErrorMsg(err?.message || 'OAuth sign-in failed.')
		}
	})

	return (
		<div class="auth-page">
			<Paper class="auth-card" elevation={0}>
				<Box
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
						<p>Completing sign-in</p>
					</div>

					<Show when={phase() === 'loading'}>
						<Stack alignItems="center" spacing={2} sx={{ py: 2 }}>
							<CircularProgress color="primary" />
							<p class="auth-message">Finishing OAuth…</p>
						</Stack>
					</Show>

					<Show when={phase() === 'fail'}>
						<Stack spacing={1.5}>
							<p class="auth-message auth-message--error">{errorMsg()}</p>
							<Button
								variant="contained"
								color="secondary"
								size="large"
								onClick={() => navigate('/login')}
							>
								Back to sign in
							</Button>
						</Stack>
					</Show>
				</Box>
			</Paper>
		</div>
	)
}

export default OAuthCallback
