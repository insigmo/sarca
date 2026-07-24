import { createRoot, createSignal } from 'solid-js'

/**
 * Shared chrome for the Files browser: header search + storage title.
 * Files page activates this while mounted.
 */
export const filesChromeStore = createRoot(() => {
	const [active, setActive] = createSignal(false)
	const [storageId, setStorageId] = createSignal(/** @type {string | null} */ (null))
	const [storageName, setStorageName] = createSignal('')
	const [searchQuery, setSearchQuery] = createSignal('')
	const [isSearching, setIsSearching] = createSignal(false)

	/** @type {{ onSearch: ((q: string) => void) | null, onClear: (() => void) | null }} */
	let handlers = { onSearch: null, onClear: null }

	/**
	 * @param {{
	 *   storageId: string,
	 *   storageName?: string,
	 *   onSearch: (q: string) => void,
	 *   onClear: () => void,
	 * }} ctx
	 */
	const activate = (ctx) => {
		setActive(true)
		setStorageId(ctx.storageId)
		setStorageName(ctx.storageName || '')
		handlers = { onSearch: ctx.onSearch, onClear: ctx.onClear }
	}

	const deactivate = () => {
		setActive(false)
		setStorageId(null)
		setStorageName('')
		setSearchQuery('')
		setIsSearching(false)
		handlers = { onSearch: null, onClear: null }
	}

	const runSearch = () => {
		handlers.onSearch?.(searchQuery())
	}

	const clearSearch = () => {
		setSearchQuery('')
		setIsSearching(false)
		handlers.onClear?.()
	}

	return {
		active,
		storageId,
		storageName,
		setStorageName,
		searchQuery,
		setSearchQuery,
		isSearching,
		setIsSearching,
		activate,
		deactivate,
		runSearch,
		clearSearch,
	}
})
