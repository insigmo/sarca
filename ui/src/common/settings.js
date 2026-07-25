import { createRoot, createSignal } from 'solid-js'

/**
 * Shared open state for the Settings modal / bottom sheet.
 * @typedef {'access' | 'trash' | 'account' | 'storage'} SettingsTab
 */
export const settingsStore = createRoot(() => {
	const [isOpen, setIsOpen] = createSignal(false)
	const [tab, setTab] = createSignal(/** @type {SettingsTab} */ ('access'))

	return {
		isOpen,
		tab,
		setTab,
		/**
		 * @param {SettingsTab} [nextTab]
		 */
		openSettings: (nextTab = 'access') => {
			setTab(nextTab)
			setIsOpen(true)
		},
		closeSettings: () => setIsOpen(false),
	}
})
