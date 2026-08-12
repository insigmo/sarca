import { reconcile } from 'solid-js/store'

/**
 * `reconcile` from `solid-js/store` patches its previous state in place and
 * returns that same reference. A plain signal's setter bails out on
 * reference equality, so handing it `reconcile(...)` directly silently
 * drops every update after the first. This keeps `reconcile`'s per-row
 * identity diffing (so unchanged rows don't remount) while still handing
 * the signal setter a fresh top-level array reference to react to.
 * @param {any[]} items
 * @param {Parameters<typeof reconcile>[1]} options
 * @returns {(prev: any[]) => any[]}
 */
export function reconcileSignal(items, options) {
	const patch = reconcile(items, options)
	return (prev) => {
		const next = patch(prev)
		return next === prev ? [...next] : next
	}
}
