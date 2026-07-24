import { createRoot, createSignal } from 'solid-js'

/**
 * Shared open state for the Settings modal / bottom sheet.
 */
export const settingsStore = createRoot(() => {
	const [isOpen, setIsOpen] = createSignal(false)
	const [tab, setTab] = createSignal(/** @type {'workers' | 'access' | 'trash'} */ ('workers'))

	return {
		isOpen,
		tab,
		setTab,
		/**
		 * @param {'workers' | 'access' | 'trash'} [nextTab]
		 */
		openSettings: (nextTab = 'workers') => {
			setTab(nextTab)
			setIsOpen(true)
		},
		closeSettings: () => setIsOpen(false),
	}
})
