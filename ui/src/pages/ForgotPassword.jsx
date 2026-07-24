import { Show, createSignal, onMount } from 'solid-js'
import Box from '@suid/material/Box'
import TextField from '@suid/material/TextField'
import Button from '@suid/material/Button'
import Paper from '@suid/material/Paper'
import Stack from '@suid/material/Stack'
import createLocalStore from '../../libs'
import { A, useNavigate } from '@solidjs/router'

import API from '../api'
import { alertStore } from '../components/AlertStack'
import logoUrl from '../assets/logo.svg'

const ForgotPassword = () => {
	const [store] = createLocalStore()
	const { addAlert } = alertStore
	const navigate = useNavigate()
	const [sent, setSent] = createSignal(false)
	const [submitting, setSubmitting] = createSignal(false)

	onMount(() => {
		if (store.access_token) {
			navigate('/')
		}
	})

	/**
	 * @param {SubmitEvent} event
	 */
	const handleSubmit = async (event) => {
		event.preventDefault()
		const data = new FormData(event.currentTarget)
		const email = String(data.get('email') || '').trim()
		if (!email) return

		setSubmitting(true)
		try {
			await API.auth.forgotPassword(email)
			setSent(true)
			addAlert('If an account exists, we sent a reset link', 'success')
		} catch {
			// toast from apiRequest (e.g. backend missing)
		} finally {
			setSubmitting(false)
		}
	}

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
						<p>Reset your password</p>
					</div>

					<Show
						when={!sent()}
						fallback={
							<Stack spacing={1.5}>
								<p class="auth-message">
									Check your email for a reset link. If you don&apos;t see it,
									look in spam or try again in a few minutes.
								</p>
								<A class="default-link" href="/login" style={{ 'text-align': 'center' }}>
									Back to sign in
								</A>
							</Stack>
						}
					>
						<Box
							component="form"
							onSubmit={handleSubmit}
							sx={{ display: 'flex', flexDirection: 'column', gap: 2 }}
						>
							<TextField
								name="email"
								label="Email"
								type="email"
								autoComplete="email"
								required
							/>
							<Stack spacing={1.5} sx={{ mt: 1 }}>
								<Button
									type="submit"
									variant="contained"
									color="secondary"
									size="large"
									disabled={submitting()}
								>
									{submitting() ? 'Sending…' : 'Send reset link'}
								</Button>
								<A
									class="default-link"
									href="/login"
									style={{ 'text-align': 'center' }}
								>
									Back to sign in
								</A>
							</Stack>
						</Box>
					</Show>
				</Box>
			</Paper>
		</div>
	)
}

export default ForgotPassword
