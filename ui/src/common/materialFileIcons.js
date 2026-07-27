/**
 * Material Icon Theme glyphs for the Files UI.
 *
 * Curated SVG imports only (avoids bundling all ~1250 icons and the ~440KB
 * `generateManifest()` JSON that was blowing past Vite's chunk size warning).
 */
import fileUrl from 'material-icon-theme/icons/file.svg?url'
import folderUrl from 'material-icon-theme/icons/folder.svg?url'
import folderOpenUrl from 'material-icon-theme/icons/folder-open.svg?url'
import pdfUrl from 'material-icon-theme/icons/pdf.svg?url'
import imageUrl from 'material-icon-theme/icons/image.svg?url'
import zipUrl from 'material-icon-theme/icons/zip.svg?url'
import videoUrl from 'material-icon-theme/icons/video.svg?url'
import audioUrl from 'material-icon-theme/icons/audio.svg?url'
import wordUrl from 'material-icon-theme/icons/word.svg?url'
import powerpointUrl from 'material-icon-theme/icons/powerpoint.svg?url'
import tableUrl from 'material-icon-theme/icons/table.svg?url'
import markdownUrl from 'material-icon-theme/icons/markdown.svg?url'
import htmlUrl from 'material-icon-theme/icons/html.svg?url'
import jsonUrl from 'material-icon-theme/icons/json.svg?url'
import javascriptUrl from 'material-icon-theme/icons/javascript.svg?url'
import typescriptUrl from 'material-icon-theme/icons/typescript.svg?url'
import reactUrl from 'material-icon-theme/icons/react.svg?url'
import reactTsUrl from 'material-icon-theme/icons/react_ts.svg?url'
import pythonUrl from 'material-icon-theme/icons/python.svg?url'
import cssUrl from 'material-icon-theme/icons/css.svg?url'
import svgUrl from 'material-icon-theme/icons/svg.svg?url'
import xmlUrl from 'material-icon-theme/icons/xml.svg?url'
import yamlUrl from 'material-icon-theme/icons/yaml.svg?url'
import tomlUrl from 'material-icon-theme/icons/toml.svg?url'
import rustUrl from 'material-icon-theme/icons/rust.svg?url'
import logUrl from 'material-icon-theme/icons/log.svg?url'
import documentUrl from 'material-icon-theme/icons/document.svg?url'
import urlUrl from 'material-icon-theme/icons/url.svg?url'
import httpUrl from 'material-icon-theme/icons/http.svg?url'

/** @type {Record<string, string>} */
const ICON_URLS = {
	file: fileUrl,
	folder: folderUrl,
	'folder-open': folderOpenUrl,
	pdf: pdfUrl,
	image: imageUrl,
	zip: zipUrl,
	video: videoUrl,
	audio: audioUrl,
	word: wordUrl,
	powerpoint: powerpointUrl,
	table: tableUrl,
	markdown: markdownUrl,
	html: htmlUrl,
	json: jsonUrl,
	javascript: javascriptUrl,
	typescript: typescriptUrl,
	react: reactUrl,
	react_ts: reactTsUrl,
	python: pythonUrl,
	css: cssUrl,
	svg: svgUrl,
	xml: xmlUrl,
	yaml: yamlUrl,
	toml: tomlUrl,
	rust: rustUrl,
	log: logUrl,
	document: documentUrl,
	url: urlUrl,
	http: httpUrl,
}

/** @type {Record<string, string>} */
const FILE_NAMES = {
	'readme.md': 'markdown',
	'license': 'document',
	'licence': 'document',
	'package.json': 'json',
	'tsconfig.json': 'json',
	'cargo.toml': 'rust',
}

/** Extension → icon id for the curated set above. */
/** @type {Record<string, string>} */
const FILE_EXTENSIONS = {
	pdf: 'pdf',
	png: 'image',
	jpg: 'image',
	jpeg: 'image',
	gif: 'image',
	webp: 'image',
	bmp: 'image',
	ico: 'image',
	heic: 'image',
	heif: 'image',
	avif: 'image',
	tif: 'image',
	tiff: 'image',
	svg: 'svg',
	zip: 'zip',
	rar: 'zip',
	'7z': 'zip',
	tar: 'zip',
	gz: 'zip',
	tgz: 'zip',
	bz2: 'zip',
	xz: 'zip',
	mp4: 'video',
	mov: 'video',
	mkv: 'video',
	webm: 'video',
	avi: 'video',
	m4v: 'video',
	'3gp': 'video',
	mp3: 'audio',
	wav: 'audio',
	flac: 'audio',
	aac: 'audio',
	ogg: 'audio',
	m4a: 'audio',
	wma: 'audio',
	doc: 'word',
	docx: 'word',
	odt: 'word',
	rtf: 'word',
	ppt: 'powerpoint',
	pptx: 'powerpoint',
	odp: 'powerpoint',
	xls: 'table',
	xlsx: 'table',
	ods: 'table',
	csv: 'table',
	tsv: 'table',
	md: 'markdown',
	markdown: 'markdown',
	mdx: 'markdown',
	html: 'html',
	htm: 'html',
	json: 'json',
	jsonc: 'json',
	js: 'javascript',
	mjs: 'javascript',
	cjs: 'javascript',
	jsx: 'react',
	ts: 'typescript',
	mts: 'typescript',
	cts: 'typescript',
	tsx: 'react_ts',
	py: 'python',
	pyw: 'python',
	css: 'css',
	scss: 'css',
	sass: 'css',
	less: 'css',
	xml: 'xml',
	xsl: 'xml',
	yml: 'yaml',
	yaml: 'yaml',
	toml: 'toml',
	rs: 'rust',
	log: 'log',
	txt: 'document',
	text: 'document',
	url: 'url',
	webloc: 'url',
	desktop: 'url',
	http: 'http',
}

/**
 * Resolve a material-icon-theme icon id for a filename.
 * @param {string} name
 * @param {{ isFile?: boolean, open?: boolean }} [opts]
 * @returns {string} icon id (e.g. `pdf`, `folder`)
 */
const resolveMaterialIconName = (name, opts = {}) => {
	const { isFile = true, open = false } = opts

	if (!isFile || name === '..') return open ? 'folder-open' : 'folder'

	const lower = String(name || '').toLowerCase()
	if (lower.startsWith('http://') || lower.startsWith('https://')) return 'http'

	if (FILE_NAMES[lower] && ICON_URLS[FILE_NAMES[lower]]) {
		return FILE_NAMES[lower]
	}

	const ext = lower.includes('.') ? lower.slice(lower.lastIndexOf('.') + 1) : ''
	if (!ext) return 'file'

	const fromExt = FILE_EXTENSIONS[ext]
	if (fromExt) return fromExt

	return 'file'
}

/**
 * @param {string} name
 * @param {{ isFile?: boolean, open?: boolean }} [opts]
 * @returns {string} absolute URL to an SVG asset
 */
export const resolveMaterialIconUrl = (name, opts = {}) => {
	const iconName = resolveMaterialIconName(name, opts)
	return ICON_URLS[iconName] || ICON_URLS.file
}
