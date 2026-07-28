/**
 * Helpers for Settings → Sync transfer queue display.
 */

const STATUS_ORDER = {
	active: 0,
	waiting: 1,
	done: 2,
}

/**
 * Sort transfers: Active → Waiting → Done, then by name.
 * @param {Array<{ name?: string, status?: string }>} items
 * @returns {typeof items}
 */
export function sortTransferItems(items) {
	return [...(items || [])].sort((a, b) => {
		const sa = STATUS_ORDER[a?.status] ?? 9
		const sb = STATUS_ORDER[b?.status] ?? 9
		if (sa !== sb) return sa - sb
		return String(a?.name || '').localeCompare(String(b?.name || ''), undefined, {
			sensitivity: 'base',
		})
	})
}

/**
 * Count unfinished transfers (active + waiting) for a direction.
 * @param {Array<{ direction?: string, status?: string }>} items
 * @param {'upload' | 'download'} direction
 */
export function countOpenTransfers(items, direction) {
	return (items || []).filter(
		(i) =>
			i?.direction === direction &&
			(i.status === 'active' || i.status === 'waiting'),
	).length
}
