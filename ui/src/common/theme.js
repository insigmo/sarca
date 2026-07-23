import { createEffect, createSignal, onMount } from 'solid-js'
import createLocalStore from '../../libs'

/**
 * @typedef {'light' | 'dark'} SarcaThemeMode
 */

const [store, setStore] = createLocalStore('sarca')

/**
 * @returns {SarcaThemeMode}
 */
export const readThemeMode = () => (store.theme === 'dark' ? 'dark' : 'light')

/**
 * @param {SarcaThemeMode} mode
 */
export const applyThemeToDocument = (mode) => {
	document.documentElement.dataset.theme = mode
	document.documentElement.style.colorScheme = mode
}

/**
 * @param {SarcaThemeMode} mode
 */
export const setThemeMode = (mode) => {
	setStore('theme', mode)
	applyThemeToDocument(mode)
}

export const toggleThemeMode = () => {
	setThemeMode(readThemeMode() === 'dark' ? 'light' : 'dark')
}

/** Call once at app boot (and when Header mounts as safety). */
export const initTheme = () => {
	applyThemeToDocument(readThemeMode())
}

/**
 * Reactive theme mode for ThemeProvider.
 */
export const useThemeMode = () => {
	const [mode, setMode] = createSignal(readThemeMode())

	onMount(() => {
		initTheme()
		setMode(readThemeMode())
	})

	createEffect(() => {
		// Touch store.theme so Solid tracks localStorage proxy updates.
		const next = store.theme === 'dark' ? 'dark' : 'light'
		setMode(next)
		applyThemeToDocument(next)
	})

	return mode
}
