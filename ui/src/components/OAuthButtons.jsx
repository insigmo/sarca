import { Show, createSignal, onMount } from 'solid-js'
import Button from '@suid/material/Button'
import Stack from '@suid/material/Stack'

import API from '../api'

/**
 * Google / GitHub buttons — only rendered when `/api/auth/providers` says enabled.
 */
const OAuthButtons = () => {
	const [providers, setProviders] = createSignal({
		google: false,
		github: false,
	})

	onMount(async () => {
		const p = await API.auth.getProviders()
		setProviders({ google: p.google, github: p.github })
	})

	const start = (provider) => {
		window.location.assign(API.auth.oauthStartUrl(provider))
	}

	return (
		<Show when={providers().google || providers().github}>
			<div class="auth-divider" role="separator">
				<span>or continue with</span>
			</div>
			<Stack spacing={1.25}>
				<Show when={providers().google}>
					<Button
						type="button"
						variant="outlined"
						color="primary"
						size="large"
						class="oauth-btn oauth-btn--google"
						onClick={() => start('google')}
					>
						Google
					</Button>
				</Show>
				<Show when={providers().github}>
					<Button
						type="button"
						variant="outlined"
						color="primary"
						size="large"
						class="oauth-btn oauth-btn--github"
						onClick={() => start('github')}
					>
						GitHub
					</Button>
				</Show>
			</Stack>
		</Show>
	)
}

export default OAuthButtons
