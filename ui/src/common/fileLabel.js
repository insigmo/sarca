/**
 * Extension / kind label for sorting and metadata (e.g. `jpeg`, `zip`, `folder`).
 * Not shown as a subtitle under file names in the grid/list.
 * @param {string} name
 * @param {boolean} [isFile=true]
 */
export const fileExtensionLabel = (name, isFile = true) => {
	if (!isFile || name === '..') return 'folder'
	const i = name.lastIndexOf('.')
	if (i < 0 || i === name.length - 1) return 'file'
	return name.slice(i + 1).toLowerCase()
}
