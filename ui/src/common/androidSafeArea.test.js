import { describe, expect, it, beforeEach, afterEach } from 'vitest'
import {
	isAndroidUa,
	installAndroidSafeAreaFallbacks,
} from './androidSafeArea'

describe('androidSafeArea', () => {
	const originalUa = navigator.userAgent

	beforeEach(() => {
		document.documentElement.classList.remove('sarca-android')
		document.documentElement.style.removeProperty('--sarca-android-top')
		document.documentElement.style.removeProperty('--sarca-android-bottom')
	})

	afterEach(() => {
		Object.defineProperty(navigator, 'userAgent', {
			configurable: true,
			get: () => originalUa,
		})
		document.documentElement.classList.remove('sarca-android')
		document.documentElement.style.removeProperty('--sarca-android-top')
		document.documentElement.style.removeProperty('--sarca-android-bottom')
	})

	it('detects Android UA', () => {
		Object.defineProperty(navigator, 'userAgent', {
			configurable: true,
			get: () => 'Mozilla/5.0 (Linux; Android 14)',
		})
		expect(isAndroidUa()).toBe(true)
	})

	it('sets fallback insets on Android', () => {
		Object.defineProperty(navigator, 'userAgent', {
			configurable: true,
			get: () => 'Mozilla/5.0 (Linux; Android 14)',
		})
		installAndroidSafeAreaFallbacks()
		expect(document.documentElement.classList.contains('sarca-android')).toBe(true)
		// Raised from 28px: devices with a cutout have a taller status bar and
		// still report a 0 inset, which left error toasts under the tray.
		expect(document.documentElement.style.getPropertyValue('--sarca-android-top')).toBe(
			'32px',
		)
		expect(
			document.documentElement.style.getPropertyValue('--sarca-android-bottom'),
		).toBe('20px')
	})

	it('is a no-op on desktop UA', () => {
		Object.defineProperty(navigator, 'userAgent', {
			configurable: true,
			get: () => 'Mozilla/5.0 (X11; Linux x86_64)',
		})
		installAndroidSafeAreaFallbacks()
		expect(document.documentElement.classList.contains('sarca-android')).toBe(false)
		expect(document.documentElement.style.getPropertyValue('--sarca-android-top')).toBe('')
	})
})
