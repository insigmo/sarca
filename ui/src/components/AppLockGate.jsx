import { Show, createSignal, onMount } from 'solid-js'
import Button from '@suid/material/Button'
import TextField from '@suid/material/TextField'

import { nativeClientStore } from '../common/nativeClient'
import { nativeInvoke } from '../common/nativeBridge'

/**
 * Blocks the app UI until PIN unlock when app lock is enabled (native only).
 */
const AppLockGate = (props) => {
	const { isNative } = nativeClientStore
	const [needed, setNeeded] = createSignal(false)
	const [pin, setPin] = createSignal('')
	const [error, setError] = createSignal('')

	onMount(async () => {
		if (!isNative()) return
		try {
			if (sessionStorage.getItem('sarca_unlocked') === '1') return
			const prefs = await nativeInvoke('get_client_prefs')
			if (prefs?.app_lock_enabled && prefs?.app_lock_pin) {
				setNeeded(true)
			}
		} catch {
			// Bridge unavailable (browser) — skip.
		}
	})

	const unlock = async () => {
		setError('')
		try {
			const prefs = await nativeInvoke('get_client_prefs')
			if (pin().trim() === prefs?.app_lock_pin) {
				sessionStorage.setItem('sarca_unlocked', '1')
				setNeeded(false)
				return
			}
			setError('Incorrect PIN')
		} catch (e) {
			setError(String(e))
		}
	}

	return (
		<>
			{props.children}
			<Show when={needed()}>
				<div class="app-lock-gate" role="dialog" aria-modal="true" aria-label="App lock">
					<div class="app-lock-gate__card">
						<h2>App locked</h2>
						<p>Enter your PIN to continue.</p>
						<TextField
							label="PIN"
							type="password"
							size="small"
							fullWidth
							value={pin()}
							onChange={(_, v) => setPin(v)}
							inputProps={{ inputMode: 'numeric', maxLength: 8 }}
							onKeyDown={(e) => {
								if (e.key === 'Enter') unlock()
							}}
						/>
						<Button
							variant="contained"
							color="secondary"
							sx={{ mt: 2 }}
							fullWidth
							onClick={unlock}
						>
							Unlock
						</Button>
						<Show when={error()}>
							<p class="app-lock-gate__error" role="status">
								{error()}
							</p>
						</Show>
					</div>
				</div>
			</Show>
		</>
	)
}

export default AppLockGate
