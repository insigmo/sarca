import { Show, createSignal, onMount } from 'solid-js'
import Box from '@suid/material/Box'
import Button from '@suid/material/Button'
import Paper from '@suid/material/Paper'
import Stack from '@suid/material/Stack'
import CircularProgress from '@suid/material/CircularProgress'
import { A, useNavigate, useSearchParams } from '@solidjs/router'

import API from '../api'
import createLocalStore from '../../libs'
import { alertStore } from '../components/AlertStack'
import logoUrl from '../assets/logo.svg'

const Verify = () => {
	const { addAlert } = alertStore
	const navigate = useNavigate()
	const [searchParams] = useSearchParams()
	const [store, setStore] = createLocalStore()

	/** @type {[import('solid-js').Accessor<'loading'|'ok'|'fail'>, any]} */
	const [phase, setPhase] = createSignal('loading')
	const [errorMsg, setErrorMsg] = createSignal('')

	onMount(async () => {
		const token = String(searchParams.token || '').trim()
		if (!token) {
			setPhase('fail')
			setErrorMsg('Missing verification token.')
			return
		}

		try {
			await API.auth.verifyEmail(token)
			setPhase('ok')
			addAlert('Email verified', 'success')
			if (store.user) {
				setStore('user', { ...store.user, email_verified: true })
			}
		} catch (err) {
			setPhase('fail')
			setErrorMsg(err?.message || 'Verification failed or the link expired.')
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
						<p>Email verification</p>
					</div>

					<Show when={phase() === 'loading'}>
						<Stack alignItems="center" spacing={2} sx={{ py: 2 }}>
							<CircularProgress color="primary" />
							<p class="auth-message">Verifying your email…</p>
						</Stack>
					</Show>

					<Show when={phase() === 'ok'}>
						<Stack spacing={1.5}>
							<p class="auth-message">Your email is verified. You&apos;re all set.</p>
							<Button
								variant="contained"
								color="secondary"
								size="large"
								onClick={() => navigate(store.access_token ? '/' : '/login')}
							>
								{store.access_token ? 'Continue' : 'Sign in'}
							</Button>
						</Stack>
					</Show>

					<Show when={phase() === 'fail'}>
						<Stack spacing={1.5}>
							<p class="auth-message auth-message--error">{errorMsg()}</p>
							<A
								class="default-link"
								href={store.access_token ? '/' : '/login'}
								style={{ 'text-align': 'center' }}
							>
								{store.access_token ? 'Back to app' : 'Back to sign in'}
							</A>
						</Stack>
					</Show>
				</Box>
			</Paper>
		</div>
	)
}

export default Verify
