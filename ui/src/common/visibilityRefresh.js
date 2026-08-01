/**
 * Whether a visibilitychange/focus event should trigger a listing refresh.
 * Native/background auto-upload (Camera sync) runs on the Rust side and has
 * no push channel to the web UI, so returning to the app is the only signal
 * available that new files might exist.
 * @param {DocumentVisibilityState} visibilityState
 */
export const shouldRefreshOnVisibilityEvent = (visibilityState) =>
	visibilityState !== 'hidden'
