/**
 * Android WebView often draws edge-to-edge while reporting 0 safe-area insets.
 * Set CSS fallbacks so chrome clears the status / nav bars.
 */

const ANDROID_TOP_FALLBACK = '28px'
const ANDROID_BOTTOM_FALLBACK = '20px'

/**
 * @returns {boolean}
 */
export function isAndroidUa() {
	if (typeof navigator === 'undefined') return false
	return /Android/i.test(navigator.userAgent || '')
}

/**
 * Apply `--sarca-android-top/bottom` when running on Android.
 * No-op on other platforms (vars stay 0px from :root).
 */
export function installAndroidSafeAreaFallbacks() {
	if (typeof document === 'undefined' || !isAndroidUa()) return
	const root = document.documentElement
	root.classList.add('sarca-android')
	root.style.setProperty('--sarca-android-top', ANDROID_TOP_FALLBACK)
	root.style.setProperty('--sarca-android-bottom', ANDROID_BOTTOM_FALLBACK)
}
