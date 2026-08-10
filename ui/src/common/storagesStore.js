import { createRoot, createSignal } from 'solid-js'

import API from '../api'

/**
 * Shared storages list so the Storages page and the delete flow in
 * BasicLayout read and refresh the same state. Without this, deleting a
 * storage from the settings modal left the page showing a stale list until
 * a manual reload.
 */
function createStoragesStore() {
	/**
	 * @type {[import("solid-js").Accessor<import("../api").StorageWithInfo[]>, any]}
	 */
	const [storages, setStorages] = createSignal([])
	const [loaded, setLoaded] = createSignal(false)

	/**
	 * @returns {Promise<import("../api").StorageWithInfo[]>}
	 */
	const refreshStorages = async () => {
		const storagesSchema = await API.storages.listStorages()
		const list = storagesSchema.storages || []
		setStorages(list)
		setLoaded(true)
		return list
	}

	return {
		storages,
		loaded,
		refreshStorages,
	}
}

export const storagesStore = createRoot(createStoragesStore)
