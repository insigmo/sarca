import { createRoot, createSignal } from 'solid-js'

/**
 * Shared open state for the Settings modal / bottom sheet.
 * @typedef {'general' | 'access' | 'trash' | 'storage'} SettingsTab
 */
export const settingsStore = createRoot(() => {
	const [isOpen, setIsOpen] = createSignal(false)
	const [tab, setTab] = createSignal(/** @type {SettingsTab} */ ('general'))

	return {
		isOpen,
		tab,
		setTab,
		/**
		 * @param {SettingsTab} [nextTab]
		 */
		openSettings: (nextTab = 'general') => {
			setTab(nextTab)
			setIsOpen(true)
		},
		closeSettings: () => setIsOpen(false),
	}
})
