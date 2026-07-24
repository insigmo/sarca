import { createRoot, createSignal } from 'solid-js'

/**
 * Shared open state for the Settings modal / bottom sheet.
 */
export const settingsStore = createRoot(() => {
	const [isOpen, setIsOpen] = createSignal(false)
	const [tab, setTab] = createSignal(/** @type {'access' | 'trash'} */ ('access'))

	return {
		isOpen,
		tab,
		setTab,
		/**
		 * @param {'access' | 'trash'} [nextTab]
		 */
		openSettings: (nextTab = 'access') => {
			setTab(nextTab)
			setIsOpen(true)
		},
		closeSettings: () => setIsOpen(false),
	}
})
