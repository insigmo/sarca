/**
 * Display basename without extension (for grid captions).
 * @param {string} name
 * @param {boolean} [isFile=true]
 */
export const fileBaseName = (name, isFile = true) => {
	if (!isFile || name === '..') return name
	const i = name.lastIndexOf('.')
	if (i <= 0) return name
	return name.slice(0, i)
}

/**
 * Extension label like `.folder`, `.jpeg`, `.zip`.
 * @param {string} name
 * @param {boolean} [isFile=true]
 */
export const fileExtensionLabel = (name, isFile = true) => {
	if (!isFile || name === '..') return '.folder'
	const i = name.lastIndexOf('.')
	if (i < 0 || i === name.length - 1) return '.file'
	return `.${name.slice(i + 1).toLowerCase()}`
}
