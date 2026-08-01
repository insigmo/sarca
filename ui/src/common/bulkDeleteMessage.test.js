import { describe, it, expect } from 'vitest'

import { formatBulkDeleteMessage } from './bulkDeleteMessage'

describe('formatBulkDeleteMessage', () => {
	it('names the single deleted item', () => {
		expect(formatBulkDeleteMessage(1, 'cat.png', false)).toBe(
			'Deleted "cat.png"',
		)
	})

	it('names the single permanently-deleted item', () => {
		expect(formatBulkDeleteMessage(1, 'cat.png', true)).toBe(
			'Permanently deleted "cat.png"',
		)
	})

	it('uses a count for multiple items', () => {
		expect(formatBulkDeleteMessage(3, 'cat.png', false)).toBe('Deleted 3 items')
	})

	// Regression: confirmBulkDelete in Files/index.jsx used to report
	// `items[0].name` whenever exactly one delete succeeded, even when
	// items[0] was the one that *failed* and a later item succeeded instead
	// (e.g. selecting [A, B], A rejects, B succeeds — toast said "Deleted A").
	// The fix passes the name of the item that actually succeeded, which this
	// helper takes as an explicit parameter rather than reaching into a list.
	it('takes the succeeded item name as an explicit argument, not the original selection order', () => {
		const succeededItemName = 'B.png'
		expect(formatBulkDeleteMessage(1, succeededItemName, false)).toBe(
			'Deleted "B.png"',
		)
	})
})
