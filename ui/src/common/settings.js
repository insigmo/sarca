import { createRoot, createSignal } from 'solid-js'

/**
 * Shared open state for the Settings modal / bottom sheet.
 * @typedef {'general' | 'sync' | 'access' | 'backup'} SettingsTab
 */

// Old tabs got folded into the ones above (see SettingsModal rework). Deep
// links and the native bridge (bindOpenSettingsDeepLink in nativeClient.js)
// still hand us the old raw strings, so map them here rather than at every
// call site.
const LEGACY_TAB_MAP = {
	security: 'general',
	trash: 'general',
	storage: 'general',
	users: 'access',
}

/**
 * @param {string} tab
 * @returns {SettingsTab}
 */
const normalizeTab = (tab) =>
	/** @type {SettingsTab} */ (LEGACY_TAB_MAP[tab] || tab)

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
			setTab(normalizeTab(nextTab))
			setIsOpen(true)
		},
		closeSettings: () => setIsOpen(false),
	}
})
