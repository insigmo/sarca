import { createRoot, createSignal } from 'solid-js'

import ar from '../locales/ar.json'
import en from '../locales/en.json'
import es from '../locales/es.json'
import ru from '../locales/ru.json'
import zh from '../locales/zh-CN.json'

const STORAGE_KEY = 'sarca.locale'

/**
 * Every shipped locale. `dir` drives the document's writing direction; Arabic
 * is the only right-to-left one, and the layout uses logical CSS properties
 * (`margin-inline-start` and friends) so it mirrors without a second sheet.
 */
export const LOCALES = [
	{ code: 'en', label: 'English', dir: 'ltr', dict: en },
	{ code: 'ru', label: 'Русский', dir: 'ltr', dict: ru },
	{ code: 'zh-CN', label: '简体中文', dir: 'ltr', dict: zh },
	{ code: 'es', label: 'Español', dir: 'ltr', dict: es },
	{ code: 'ar', label: 'العربية', dir: 'rtl', dict: ar },
]

export const DEFAULT_LOCALE = 'en'

const byCode = new Map(LOCALES.map((l) => [l.code, l]))

/**
 * Best shipped locale for a browser language tag.
 *
 * Matches the full tag first (`zh-CN`), then the primary subtag, so a browser
 * asking for `ru-RU` or `es-MX` still gets a translated UI instead of English.
 * @param {string} tag
 * @returns {string}
 */
export function resolveLocale(tag) {
	const wanted = String(tag || '').trim()
	if (!wanted) return DEFAULT_LOCALE
	const exact = LOCALES.find((l) => l.code.toLowerCase() === wanted.toLowerCase())
	if (exact) return exact.code
	const primary = wanted.split('-')[0].toLowerCase()
	const loose = LOCALES.find((l) => l.code.split('-')[0].toLowerCase() === primary)
	return loose ? loose.code : DEFAULT_LOCALE
}

/**
 * Read a dot-separated key out of a dictionary.
 * @param {Record<string, unknown>} dict
 * @param {string} key
 * @returns {string | null}
 */
function lookup(dict, key) {
	let node = dict
	for (const part of key.split('.')) {
		if (!node || typeof node !== 'object') return null
		node = node[part]
	}
	return typeof node === 'string' ? node : null
}

/**
 * Substitute `{{name}}` placeholders.
 * @param {string} template
 * @param {Record<string, unknown>} [params]
 * @returns {string}
 */
export function interpolate(template, params) {
	if (!params) return template
	return template.replace(/\{\{(\w+)\}\}/g, (match, name) =>
		Object.hasOwn(params, name) ? String(params[name]) : match,
	)
}

function detectInitialLocale() {
	if (typeof localStorage !== 'undefined') {
		const saved = localStorage.getItem(STORAGE_KEY)
		if (saved && byCode.has(saved)) return saved
	}
	if (typeof navigator !== 'undefined') {
		for (const tag of navigator.languages || [navigator.language]) {
			const resolved = resolveLocale(tag)
			if (resolved !== DEFAULT_LOCALE) return resolved
		}
		return resolveLocale(navigator.language)
	}
	return DEFAULT_LOCALE
}

function createI18n() {
	const [locale, setLocaleSignal] = createSignal(detectInitialLocale())

	const applyDocumentAttributes = (code) => {
		if (typeof document === 'undefined') return
		const entry = byCode.get(code) || byCode.get(DEFAULT_LOCALE)
		document.documentElement.lang = entry.code
		document.documentElement.dir = entry.dir
	}

	/**
	 * Translate `key`, falling back to English and finally to the key itself.
	 *
	 * Returning the key rather than an empty string keeps a missing entry
	 * visible during development instead of silently blanking the UI.
	 * @param {string} key
	 * @param {Record<string, unknown>} [params]
	 * @returns {string}
	 */
	const t = (key, params) => {
		const entry = byCode.get(locale()) || byCode.get(DEFAULT_LOCALE)
		const hit = lookup(entry.dict, key) ?? lookup(en, key)
		return hit === null ? key : interpolate(hit, params)
	}

	/** @param {string} code */
	const setLocale = (code) => {
		const resolved = byCode.has(code) ? code : resolveLocale(code)
		setLocaleSignal(resolved)
		applyDocumentAttributes(resolved)
		if (typeof localStorage !== 'undefined') {
			localStorage.setItem(STORAGE_KEY, resolved)
		}
	}

	/** Push the detected locale onto `<html>` once at startup. */
	const install = () => applyDocumentAttributes(locale())

	const isRtl = () => (byCode.get(locale()) || byCode.get(DEFAULT_LOCALE)).dir === 'rtl'

	return { locale, setLocale, t, install, isRtl }
}

export const i18n = createRoot(createI18n)

/** Shorthand so components read `t('files.trash')` and stay reactive. */
export const t = (key, params) => i18n.t(key, params)
