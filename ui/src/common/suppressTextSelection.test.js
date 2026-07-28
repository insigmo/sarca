import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import {
	isTextSelectionAllowed,
	onSelectStart,
	clearDomSelection,
	installTextSelectionGuard,
} from './suppressTextSelection'

describe('isTextSelectionAllowed', () => {
	it('allows input/textarea/select and allow-text-select', () => {
		const input = document.createElement('input')
		document.body.appendChild(input)
		expect(isTextSelectionAllowed(input)).toBe(true)

		const wrap = document.createElement('div')
		wrap.className = 'allow-text-select'
		const span = document.createElement('span')
		wrap.appendChild(span)
		document.body.appendChild(wrap)
		expect(isTextSelectionAllowed(span)).toBe(true)

		input.remove()
		wrap.remove()
	})

	it('blocks plain UI labels', () => {
		const label = document.createElement('span')
		label.textContent = 'All files'
		document.body.appendChild(label)
		expect(isTextSelectionAllowed(label)).toBe(false)
		label.remove()
	})
})

describe('onSelectStart', () => {
	it('prevents default for non-editable targets', () => {
		const label = document.createElement('div')
		const event = {
			target: label,
			preventDefault: vi.fn(),
		}
		onSelectStart(/** @type {any} */ (event))
		expect(event.preventDefault).toHaveBeenCalled()
	})

	it('does not prevent default for inputs', () => {
		const input = document.createElement('input')
		const event = {
			target: input,
			preventDefault: vi.fn(),
		}
		onSelectStart(/** @type {any} */ (event))
		expect(event.preventDefault).not.toHaveBeenCalled()
	})
})

describe('clearDomSelection', () => {
	it('clears an active selection range', () => {
		const el = document.createElement('div')
		el.textContent = 'hello'
		document.body.appendChild(el)
		const range = document.createRange()
		range.selectNodeContents(el)
		const sel = window.getSelection()
		sel?.removeAllRanges()
		sel?.addRange(range)
		expect(sel?.rangeCount || 0).toBeGreaterThan(0)
		clearDomSelection()
		expect(sel?.rangeCount || 0).toBe(0)
		el.remove()
	})
})

describe('installTextSelectionGuard', () => {
	beforeEach(() => {
		vi.spyOn(document, 'addEventListener')
		vi.spyOn(document, 'removeEventListener')
	})
	afterEach(() => {
		vi.restoreAllMocks()
	})

	it('registers and unregisters selectstart in capture phase', () => {
		const stop = installTextSelectionGuard()
		expect(document.addEventListener).toHaveBeenCalledWith(
			'selectstart',
			onSelectStart,
			true,
		)
		stop()
		expect(document.removeEventListener).toHaveBeenCalledWith(
			'selectstart',
			onSelectStart,
			true,
		)
	})
})
