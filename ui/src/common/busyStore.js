import { createRoot, createSignal } from 'solid-js'

/**
 * Tiny shared flag for "a critical operation is in flight and the app must
 * not let the user tear down the session mid-way". Storage creation makes
 * several sequential Telegram calls server-side and can take seconds — if
 * the user logs out or disconnects while it's running, the in-flight
 * request can outlive the session it was authenticated with.
 */
function createBusyStore() {
	const [storageCreating, setStorageCreating] = createSignal(false)

	const isStorageCreating = () => storageCreating()

	return {
		isStorageCreating,
		setStorageCreating,
	}
}

export const busyStore = createRoot(createBusyStore)
