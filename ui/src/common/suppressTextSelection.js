/**
 * Block native browser text selection in the app chrome.
 * Inputs / editable fields stay selectable so copy-paste still works.
 */

const EDITABLE_SELECTOR =
	'input, textarea, select, [contenteditable=""], [contenteditable="true"], .allow-text-select'

/**
 * @param {EventTarget | null} target
 * @returns {boolean}
 */
export function isTextSelectionAllowed(target) {
	if (!(target instanceof Element)) return false
	return Boolean(target.closest(EDITABLE_SELECTOR))
}

/**
 * @param {Event} event
 */
export function onSelectStart(event) {
	if (isTextSelectionAllowed(event.target)) return
	event.preventDefault()
}

/**
 * Clear any existing DOM selection range (e.g. after a drag gesture).
 */
export function clearDomSelection() {
	const sel = typeof window !== 'undefined' ? window.getSelection?.() : null
	if (sel && sel.rangeCount > 0) sel.removeAllRanges()
}

/**
 * Install document-level selectstart suppression.
 * @returns {() => void} unsubscribe / cleanup
 */
export function installTextSelectionGuard() {
	document.addEventListener('selectstart', onSelectStart, true)
	return () => {
		document.removeEventListener('selectstart', onSelectStart, true)
	}
}
