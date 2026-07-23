/**
 * Map filename → icon kind used by FileTypeIcon.
 * @param {string} name
 * @param {boolean} [isFile=true]
 */
export const fileKind = (name, isFile = true) => {
	if (!isFile) return 'folder'
	if (name === '..') return 'folder'

	const ext = name.includes('.')
		? name.slice(name.lastIndexOf('.') + 1).toLowerCase()
		: ''

	if (['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'svg', 'heic', 'avif'].includes(ext)) {
		return 'image'
	}
	if (['mp4', 'mkv', 'webm', 'mov', 'avi', 'm4v'].includes(ext)) {
		return 'video'
	}
	if (ext === 'pdf') return 'pdf'
	if (['mp3', 'wav', 'flac', 'ogg', 'aac', 'm4a'].includes(ext)) return 'audio'
	if (['zip', 'rar', '7z', 'tar', 'gz', 'bz2', 'xz', 'tgz'].includes(ext)) return 'archive'
	if (['xls', 'xlsx', 'ods', 'csv'].includes(ext)) return 'spreadsheet'
	if (['doc', 'docx', 'odt', 'rtf'].includes(ext)) return 'document'
	if (['url', 'webloc', 'desktop'].includes(ext) || name.startsWith('http')) return 'link'
	if (
		[
			'txt',
			'md',
			'json',
			'xml',
			'log',
			'rs',
			'js',
			'ts',
			'tsx',
			'jsx',
			'py',
			'html',
			'css',
			'yml',
			'yaml',
			'toml',
		].includes(ext)
	) {
		return 'text'
	}
	return 'generic'
}
