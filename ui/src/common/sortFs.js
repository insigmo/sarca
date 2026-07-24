import { fileExtensionLabel } from './fileLabel'

/**
 * @typedef {'name' | 'size' | 'mtime' | 'type'} SortField
 * @typedef {'asc' | 'desc'} SortDir
 */

/**
 * Parse optional mtime from an FS element (API may not expose it yet).
 * @param {import('../api').FSElement} el
 * @returns {number}
 */
const elementMtime = (el) => {
	const raw =
		el.mtime ??
		el.modified_at ??
		el.updated_at ??
		el.date_modified ??
		null
	if (raw == null) return 0
	const t = typeof raw === 'number' ? raw : Date.parse(raw)
	return Number.isFinite(t) ? t : 0
}

/**
 * Client-side sort for the Files listing.
 * Always keeps `..` first; then folders before files; then the chosen field.
 *
 * @param {import('../api').FSElement[]} items
 * @param {SortField} field
 * @param {SortDir} dir
 * @returns {import('../api').FSElement[]}
 */
export const sortFsElements = (items, field = 'name', dir = 'asc') => {
	const mult = dir === 'desc' ? -1 : 1
	const copy = [...items]

	copy.sort((a, b) => {
		if (a.name === '..') return -1
		if (b.name === '..') return 1

		if (a.is_file !== b.is_file) {
			return a.is_file ? 1 : -1
		}

		let cmp = 0
		switch (field) {
			case 'size':
				cmp = (a.size || 0) - (b.size || 0)
				break
			case 'mtime':
				cmp = elementMtime(a) - elementMtime(b)
				break
			case 'type': {
				const ta = fileExtensionLabel(a.name, a.is_file)
				const tb = fileExtensionLabel(b.name, b.is_file)
				cmp = ta.localeCompare(tb, undefined, { sensitivity: 'base' })
				break
			}
			case 'name':
			default:
				cmp = a.name.localeCompare(b.name, undefined, {
					sensitivity: 'base',
					numeric: true,
				})
				break
		}

		if (cmp === 0 && field !== 'name') {
			cmp = a.name.localeCompare(b.name, undefined, {
				sensitivity: 'base',
				numeric: true,
			})
		}

		return cmp * mult
	})

	return copy
}

/**
 * Human label for the current sort control.
 * @param {SortField} field
 * @param {SortDir} dir
 */
export const sortLabel = (field, dir) => {
	const fields = {
		name: 'Name',
		size: 'Size',
		mtime: 'Date modified',
		type: 'File type',
	}
	const arrow = dir === 'asc' ? '↑' : '↓'
	return `Sort: ${fields[field] || 'Name'} ${arrow}`
}
