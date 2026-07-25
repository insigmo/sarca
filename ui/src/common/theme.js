import { createEffect, createSignal, onMount } from 'solid-js'
import createLocalStore from '../../libs'

/**
 * @typedef {'light' | 'dark'} SarcaThemeMode
 */

/** Order: light → dark (picker + toggleThemeMode cycle). */
const THEMES = /** @type {const} */ (['light', 'dark'])

/** Legacy ids remapped onto the Fluent themes that now own those names. */
const LEGACY_THEME_MAP = {
	explorer: 'light',
	'explorer-dark': 'dark',
}

const [store, setStore] = createLocalStore('sarca')

/**
 * @param {unknown} value
 * @returns {value is SarcaThemeMode}
 */
const isThemeMode = (value) =>
	typeof value === 'string' && THEMES.includes(/** @type {SarcaThemeMode} */ (value))

/**
 * @returns {SarcaThemeMode}
 */
export const readThemeMode = () => {
	const raw = store.theme
	if (typeof raw === 'string' && raw in LEGACY_THEME_MAP) {
		return /** @type {SarcaThemeMode} */ (LEGACY_THEME_MAP[raw])
	}
	if (isThemeMode(raw)) return raw
	return 'light'
}

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
	if (!isThemeMode(mode)) return
	setStore('theme', mode)
	applyThemeToDocument(mode)
}

/** Cycles light → dark → light. */
export const toggleThemeMode = () => {
	const cur = readThemeMode()
	const idx = THEMES.indexOf(cur)
	setThemeMode(THEMES[(idx + 1) % THEMES.length])
}

/** Call once at app boot (and when Header mounts as safety). */
export const initTheme = () => {
	const mode = readThemeMode()
	const raw = store.theme
	if (typeof raw === 'string' && raw in LEGACY_THEME_MAP) {
		setStore('theme', mode)
	}
	applyThemeToDocument(mode)
}

export const themeLabels = {
	light: 'Light',
	dark: 'Dark',
}

export const themeHints = {
	light: 'Fluent light',
	dark: 'Fluent dark',
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
		const next = readThemeMode()
		setMode(next)
		applyThemeToDocument(next)
	})

	return mode
}

export { THEMES }
