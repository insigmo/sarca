import { Show, createSignal, onMount } from 'solid-js'
import Alert from '@suid/material/Alert'
import Button from '@suid/material/Button'
import Stack from '@suid/material/Stack'

import API from '../api'
import createLocalStore from '../../libs'
import { alertStore } from './AlertStack'

/**
 * Soft email-verification reminder when `email_verified === false`.
 */
const EmailVerifyBanner = () => {
	const [store, setStore] = createLocalStore()
	const { addAlert } = alertStore
	const [sending, setSending] = createSignal(false)

	onMount(async () => {
		if (!store.access_token) return
		const me = await API.auth.meSilent()
		if (!me) return
		setStore('user', {
			email: me.email,
			email_verified: me.email_verified,
		})
	})

	const resend = async () => {
		if (sending()) return
		setSending(true)
		try {
			await API.auth.requestVerify()
			addAlert('Verification email sent — check your inbox', 'success')
		} catch (err) {
			// apiRequest already toasts; ensure a clear message if body empty
			if (!err?.message) {
				addAlert('Could not send verification email', 'error')
			}
		} finally {
			setSending(false)
		}
	}

	return (
		<Show when={store.user?.email_verified === false}>
			<Alert
				severity="warning"
				class="email-verify-banner"
				action={
					<Button
						color="inherit"
						size="small"
						disabled={sending()}
						onClick={resend}
					>
						{sending() ? 'Sending…' : 'Resend'}
					</Button>
				}
				sx={{ mb: 2 }}
			>
				<Stack spacing={0.25}>
					<span>Verify your email to unlock full account recovery.</span>
					<Show when={store.user?.email}>
						<span class="email-verify-banner__hint">{store.user.email}</span>
					</Show>
				</Stack>
			</Alert>
		</Show>
	)
}

export default EmailVerifyBanner
