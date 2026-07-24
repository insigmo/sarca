import { Show, createSignal, onMount } from 'solid-js'
import Box from '@suid/material/Box'
import TextField from '@suid/material/TextField'
import Button from '@suid/material/Button'
import Paper from '@suid/material/Paper'
import Stack from '@suid/material/Stack'
import { A, useNavigate, useSearchParams } from '@solidjs/router'

import API from '../api'
import { alertStore } from '../components/AlertStack'
import logoUrl from '../assets/logo.svg'

const ResetPassword = () => {
	const { addAlert } = alertStore
	const navigate = useNavigate()
	const [searchParams] = useSearchParams()
	const [done, setDone] = createSignal(false)
	const [submitting, setSubmitting] = createSignal(false)
	const [missingToken, setMissingToken] = createSignal(false)

	const token = () => String(searchParams.token || '').trim()

	onMount(() => {
		if (!token()) {
			setMissingToken(true)
		}
	})

	/**
	 * @param {SubmitEvent} event
	 */
	const handleSubmit = async (event) => {
		event.preventDefault()
		const data = new FormData(event.currentTarget)
		const password = String(data.get('password') || '')
		const confirm = String(data.get('confirm') || '')

		if (password.length < 8) {
			addAlert('Password must be at least 8 characters', 'error')
			return
		}
		if (password !== confirm) {
			addAlert('Passwords do not match', 'error')
			return
		}
		if (!token()) {
			addAlert('Missing reset token', 'error')
			return
		}

		setSubmitting(true)
		try {
			await API.auth.resetPassword(token(), password)
			setDone(true)
			addAlert('Password updated — you can sign in now', 'success')
		} catch {
			// toast from apiRequest
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
						<p>Choose a new password</p>
					</div>

					<Show when={missingToken()}>
						<Stack spacing={1.5}>
							<p class="auth-message auth-message--error">
								This reset link is missing a token. Request a new one from the
								forgot-password page.
							</p>
							<A
								class="default-link"
								href="/forgot-password"
								style={{ 'text-align': 'center' }}
							>
								Forgot password
							</A>
						</Stack>
					</Show>

					<Show when={!missingToken() && done()}>
						<Stack spacing={1.5}>
							<p class="auth-message">Your password has been updated.</p>
							<Button
								variant="contained"
								color="secondary"
								size="large"
								onClick={() => navigate('/login')}
							>
								Sign in
							</Button>
						</Stack>
					</Show>

					<Show when={!missingToken() && !done()}>
						<Box
							component="form"
							onSubmit={handleSubmit}
							sx={{ display: 'flex', flexDirection: 'column', gap: 2 }}
						>
							<TextField
								name="password"
								label="New password"
								type="password"
								autoComplete="new-password"
								required
							/>
							<TextField
								name="confirm"
								label="Confirm password"
								type="password"
								autoComplete="new-password"
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
									{submitting() ? 'Saving…' : 'Update password'}
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

export default ResetPassword
