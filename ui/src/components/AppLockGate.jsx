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
		if (sessionStorage.getItem('sarca_unlocked') === '1') return

		// isNative() can be true from a stale localStorage flag before the
		// real bridge (window.__sarcaInvoke / __TAURI_INTERNALS__) is
		// injected — nativeClientStore itself polls up to ~10s for exactly
		// this reason (see nativeClient.js). Treating the first failure as
		// "not native, skip the lock" let a slow-injecting WebView (common on
		// Android) start fully unlocked even with app lock + a PIN set.
		// Retry on the same schedule instead of bypassing the lock outright.
		for (let attempt = 0; attempt < 40 && isNative(); attempt++) {
			try {
				const prefs = await nativeInvoke('get_client_prefs')
				if (prefs?.app_lock_enabled && prefs?.app_lock_pin_set) {
					setNeeded(true)
				}
				return
			} catch {
				await new Promise((resolve) => setTimeout(resolve, 250))
			}
		}
	})

	// The PIN never leaves Rust: `get_client_prefs` only reports whether one is
	// set, and the comparison happens in `verify_app_lock_pin` (salted hash,
	// constant time, throttled). Comparing here would have meant handing the
	// PIN to every page that can reach the bridge.
	const unlock = async () => {
		setError('')
		try {
			const ok = await nativeInvoke('verify_app_lock_pin', { pin: pin().trim() })
			if (ok) {
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
