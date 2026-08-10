import { describe, it, expect, beforeEach, afterEach } from 'vitest'

import { i18n, resolveLocale, interpolate, LOCALES, DEFAULT_LOCALE } from './i18n'
import en from '../locales/en.json'

/** Every dot-path that maps to a string in `dict`. */
const flatKeys = (dict, prefix = '') =>
	Object.entries(dict).flatMap(([key, value]) => {
		const path = prefix ? `${prefix}.${key}` : key
		return typeof value === 'string' ? [path] : flatKeys(value, path)
	})

describe('resolveLocale', () => {
	it('matches the full tag before the primary subtag', () => {
		expect(resolveLocale('zh-CN')).toBe('zh-CN')
		expect(resolveLocale('zh-cn')).toBe('zh-CN')
	})

	// A browser asking for ru-RU or es-MX must not fall back to English.
	it('falls back to the primary subtag', () => {
		expect(resolveLocale('ru-RU')).toBe('ru')
		expect(resolveLocale('es-MX')).toBe('es')
		expect(resolveLocale('ar-EG')).toBe('ar')
	})

	it('defaults to English for anything unshipped', () => {
		expect(resolveLocale('ja')).toBe(DEFAULT_LOCALE)
		expect(resolveLocale('')).toBe(DEFAULT_LOCALE)
	})
})

describe('interpolate', () => {
	it('substitutes named placeholders', () => {
		expect(interpolate('{{count}} selected', { count: 3 })).toBe('3 selected')
	})

	it('leaves an unknown placeholder visible rather than blanking it', () => {
		expect(interpolate('{{a}} {{b}}', { a: 1 })).toBe('1 {{b}}')
	})
})

describe('locale dictionaries', () => {
	// A missing key silently falls back to English, which reads as a half
	// translated screen. Catch the gap here instead.
	it.each(LOCALES.map((l) => l.code))('%s covers every English key', (code) => {
		const entry = LOCALES.find((l) => l.code === code)
		const missing = flatKeys(en).filter((key) => {
			let node = entry.dict
			for (const part of key.split('.')) {
				if (!node || typeof node !== 'object') return true
				node = node[part]
			}
			return typeof node !== 'string'
		})
		expect(missing).toEqual([])
	})

	it('marks Arabic, and only Arabic, as right to left', () => {
		expect(LOCALES.filter((l) => l.dir === 'rtl').map((l) => l.code)).toEqual(['ar'])
	})
})

describe('i18n store', () => {
	beforeEach(() => {
		localStorage.clear()
	})

	afterEach(() => {
		i18n.setLocale(DEFAULT_LOCALE)
		localStorage.clear()
	})

	it('translates and interpolates through the active locale', () => {
		i18n.setLocale('ru')
		expect(i18n.t('sidebar.logOut')).toBe('Выйти')
		expect(i18n.t('files.selectedCount', { count: 2 })).toBe('Выбрано: 2')
	})

	it('returns the key itself when nothing matches', () => {
		expect(i18n.t('nope.not.here')).toBe('nope.not.here')
	})

	it('sets lang and dir on the document, and remembers the choice', () => {
		i18n.setLocale('ar')
		expect(document.documentElement.lang).toBe('ar')
		expect(document.documentElement.dir).toBe('rtl')
		expect(i18n.isRtl()).toBe(true)
		expect(localStorage.getItem('sarca.locale')).toBe('ar')

		i18n.setLocale('en')
		expect(document.documentElement.dir).toBe('ltr')
		expect(i18n.isRtl()).toBe(false)
	})
})
