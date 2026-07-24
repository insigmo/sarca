import { createRoot, createSignal } from 'solid-js'

/**
 * Opens StorageSettingsModal from Storages cards, Files header, etc.
 * `storage` is a minimal `{ id, name, ... }` — modal loads full detail itself.
 */
export const storageSettingsStore = createRoot(() => {
	const [storage, setStorage] = createSignal(
		/** @type {import('../api').StorageWithInfo | { id: string, name: string } | null} */ (
			null
		),
	)

	return {
		storage,
		/**
		 * @param {import('../api').StorageWithInfo | { id: string, name: string }} next
		 */
		open: (next) => setStorage(next),
		close: () => setStorage(null),
		/**
		 * @param {{ id: string, name: string }} updated
		 */
		patchName: (updated) => {
			const cur = storage()
			if (cur && cur.id === updated.id) {
				setStorage({ ...cur, name: updated.name })
			}
		},
	}
})
