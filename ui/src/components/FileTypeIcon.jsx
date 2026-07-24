import { Show } from 'solid-js'
import Box from '@suid/material/Box'

import { fileKind } from '../common/fileKind'

const SIZE = 48

let uid = 0
const nextId = (prefix) => `${prefix}-${++uid}`

/** Soft glass tile wrapping a colorful glyph — works in light & dark. */
const Tile = (props) => (
	<span
		class="file-type-icon"
		classList={{ 'file-type-icon--sm': props.size === 'sm' }}
		style={props.sizePx ? { width: `${props.sizePx}px`, height: `${props.sizePx}px` } : undefined}
		aria-hidden="true"
	>
		{props.children}
	</span>
)

const Glyph = (props) => (
	<svg viewBox="0 0 48 48" width="100%" height="100%" focusable="false" class="file-type-icon__glyph">
		{props.children}
	</svg>
)

const FolderGlyph = () => {
	const g = nextId('fold')
	return (
		<Glyph>
			<defs>
				<linearGradient id={g} x1="8" y1="12" x2="40" y2="40" gradientUnits="userSpaceOnUse">
					<stop stop-color="#F6C14B" />
					<stop offset="1" stop-color="#E8892A" />
				</linearGradient>
			</defs>
			<path
				d="M10 18.5c0-1.9 1.5-3.5 3.4-3.5h6.2c.8 0 1.5.3 2 .9l1.4 1.6c.5.6 1.2.9 2 .9h9.6c1.9 0 3.4 1.6 3.4 3.5V34c0 1.9-1.5 3.5-3.4 3.5H13.4C11.5 37.5 10 36 10 34V18.5Z"
				fill={`url(#${g})`}
			/>
			<path
				d="M10 22h28v12c0 1.9-1.5 3.5-3.4 3.5H13.4C11.5 37.5 10 36 10 34V22Z"
				fill="#F0A52E"
				opacity="0.55"
			/>
		</Glyph>
	)
}

const ImageGlyph = () => {
	const sky = nextId('sky')
	const peak = nextId('peak')
	return (
		<Glyph>
			<defs>
				<linearGradient id={sky} x1="10" y1="10" x2="38" y2="38" gradientUnits="userSpaceOnUse">
					<stop stop-color="#6EC8FF" />
					<stop offset="1" stop-color="#2B6CFF" />
				</linearGradient>
				<linearGradient id={peak} x1="12" y1="28" x2="36" y2="40" gradientUnits="userSpaceOnUse">
					<stop stop-color="#5B8CFF" />
					<stop offset="1" stop-color="#1E4FD6" />
				</linearGradient>
			</defs>
			<rect x="9" y="11" width="30" height="26" rx="7" fill={`url(#${sky})`} opacity="0.35" />
			<circle cx="33" cy="18" r="3.2" fill="#FFB04A" />
			<path d="M12 34.5 21 24l5.5 6.2L31 25.5 38 34.5H12Z" fill={`url(#${peak})`} />
			<path d="M12 34.5 18.5 28l4 4.5 4.2-5.2L38 34.5H12Z" fill="#7EB6FF" opacity="0.7" />
		</Glyph>
	)
}

const VideoGlyph = () => {
	const g = nextId('vid')
	return (
		<Glyph>
			<defs>
				<linearGradient id={g} x1="10" y1="12" x2="38" y2="36" gradientUnits="userSpaceOnUse">
					<stop stop-color="#FF6B9D" />
					<stop offset="1" stop-color="#7B5CFF" />
				</linearGradient>
			</defs>
			<rect x="9" y="13" width="30" height="22" rx="7" fill={`url(#${g})`} opacity="0.9" />
			<path d="M21 19.5v9l9-4.5-9-4.5Z" fill="#fff" opacity="0.95" />
		</Glyph>
	)
}

const AudioGlyph = () => {
	const g = nextId('aud')
	return (
		<Glyph>
			<defs>
				<linearGradient id={g} x1="12" y1="10" x2="36" y2="38" gradientUnits="userSpaceOnUse">
					<stop stop-color="#FF7AD9" />
					<stop offset="1" stop-color="#7A5CFF" />
				</linearGradient>
			</defs>
			<rect x="15" y="28" width="3.2" height="8" rx="1.5" fill={`url(#${g})`} />
			<rect x="20.4" y="22" width="3.2" height="14" rx="1.5" fill={`url(#${g})`} />
			<rect x="25.8" y="16" width="3.2" height="20" rx="1.5" fill={`url(#${g})`} />
			<rect x="31.2" y="24" width="3.2" height="12" rx="1.5" fill={`url(#${g})`} />
		</Glyph>
	)
}

const ArchiveGlyph = () => {
	const a = nextId('a1')
	const b = nextId('a2')
	const c = nextId('a3')
	return (
		<Glyph>
			<defs>
				<linearGradient id={a} x1="10" y1="14" x2="38" y2="22" gradientUnits="userSpaceOnUse">
					<stop stop-color="#7BE7FF" />
					<stop offset="1" stop-color="#3B7BFF" />
				</linearGradient>
				<linearGradient id={b} x1="10" y1="22" x2="38" y2="30" gradientUnits="userSpaceOnUse">
					<stop stop-color="#5AD6FF" />
					<stop offset="1" stop-color="#2F63E8" />
				</linearGradient>
				<linearGradient id={c} x1="10" y1="30" x2="38" y2="38" gradientUnits="userSpaceOnUse">
					<stop stop-color="#3BC4F5" />
					<stop offset="1" stop-color="#2550C8" />
				</linearGradient>
			</defs>
			<path d="M14 18h20l-3 6H17l-3-6Z" fill={`url(#${a})`} opacity="0.95" />
			<path d="M15 25h18l-2.5 6H17.5L15 25Z" fill={`url(#${b})`} opacity="0.95" />
			<path d="M16.5 32h15l-2 5H18.5l-2-5Z" fill={`url(#${c})`} />
		</Glyph>
	)
}

const PdfGlyph = () => {
	const g = nextId('pdf')
	return (
		<Glyph>
			<defs>
				<linearGradient id={g} x1="12" y1="10" x2="36" y2="38" gradientUnits="userSpaceOnUse">
					<stop stop-color="#FF8A7A" />
					<stop offset="1" stop-color="#E1243B" />
				</linearGradient>
			</defs>
			<path
				d="M16 10h12l8 8v18c0 1.7-1.3 3-3 3H16c-1.7 0-3-1.3-3-3V13c0-1.7 1.3-3 3-3Z"
				fill={`url(#${g})`}
			/>
			<path d="M28 10v6c0 1.1.9 2 2 2h6" fill="#FFD0C8" opacity="0.85" />
			<path
				d="M19 28h10M19 23h10M19 33h7"
				stroke="#fff"
				stroke-width="2"
				stroke-linecap="round"
				opacity="0.9"
			/>
		</Glyph>
	)
}

const DocumentGlyph = () => {
	const g = nextId('doc')
	return (
		<Glyph>
			<defs>
				<linearGradient id={g} x1="12" y1="12" x2="36" y2="36" gradientUnits="userSpaceOnUse">
					<stop stop-color="#7AA8FF" />
					<stop offset="1" stop-color="#6B4DFF" />
				</linearGradient>
			</defs>
			<rect x="13" y="11" width="22" height="26" rx="5" fill="none" stroke={`url(#${g})`} stroke-width="2.2" stroke-dasharray="3 2.5" />
			<path
				d="M21 16h6v3.2h-1.7V30h-2.6V19.2H21V16Z"
				fill={`url(#${g})`}
			/>
		</Glyph>
	)
}

const TextGlyph = () => {
	const g = nextId('txt')
	return (
		<Glyph>
			<defs>
				<linearGradient id={g} x1="12" y1="12" x2="36" y2="36" gradientUnits="userSpaceOnUse">
					<stop stop-color="#5ED6A8" />
					<stop offset="1" stop-color="#1F8A6A" />
				</linearGradient>
			</defs>
			<rect x="12" y="11" width="24" height="26" rx="6" fill={`url(#${g})`} opacity="0.9" />
			<path
				d="M18 20h12M18 25h12M18 30h8"
				stroke="#fff"
				stroke-width="2.2"
				stroke-linecap="round"
			/>
		</Glyph>
	)
}

const SpreadsheetGlyph = () => {
	const g = nextId('xls')
	return (
		<Glyph>
			<defs>
				<linearGradient id={g} x1="12" y1="14" x2="36" y2="36" gradientUnits="userSpaceOnUse">
					<stop stop-color="#5BE7D0" />
					<stop offset="1" stop-color="#2B6CFF" />
				</linearGradient>
			</defs>
			<path
				d="M12 34h24v2.5H12V34Zm3-2 5-10 4.5 6.5L30 14l6 20H15Z"
				fill={`url(#${g})`}
			/>
		</Glyph>
	)
}

const LinkGlyph = () => {
	const g = nextId('lnk')
	return (
		<Glyph>
			<defs>
				<linearGradient id={g} x1="10" y1="14" x2="38" y2="34" gradientUnits="userSpaceOnUse">
					<stop stop-color="#B06BFF" />
					<stop offset="1" stop-color="#3B7BFF" />
				</linearGradient>
			</defs>
			<path
				d="M20.5 27.5c-1.8 1.8-4.7 1.8-6.5 0s-1.8-4.7 0-6.5l5-5c1.8-1.8 4.7-1.8 6.5 0"
				fill="none"
				stroke={`url(#${g})`}
				stroke-width="3.2"
				stroke-linecap="round"
			/>
			<path
				d="M27.5 20.5c1.8-1.8 4.7-1.8 6.5 0s1.8 4.7 0 6.5l-5 5c-1.8 1.8-4.7 1.8-6.5 0"
				fill="none"
				stroke={`url(#${g})`}
				stroke-width="3.2"
				stroke-linecap="round"
			/>
		</Glyph>
	)
}

const GenericGlyph = () => {
	const g = nextId('gen')
	return (
		<Glyph>
			<defs>
				<radialGradient id={g} cx="40%" cy="35%" r="65%">
					<stop stop-color="#8EF0FF" />
					<stop offset="0.55" stop-color="#4A8CFF" />
					<stop offset="1" stop-color="#2A4FD0" />
				</radialGradient>
			</defs>
			<ellipse cx="24" cy="24" rx="12" ry="12" fill={`url(#${g})`} />
			<ellipse cx="24" cy="24" rx="16" ry="5.5" fill="none" stroke="#9AD8FF" stroke-width="2" opacity="0.85" />
		</Glyph>
	)
}

/**
 * @param {string} kind
 */
const glyphForKind = (kind) => {
	switch (kind) {
		case 'folder':
			return <FolderGlyph />
		case 'image':
			return <ImageGlyph />
		case 'video':
			return <VideoGlyph />
		case 'audio':
			return <AudioGlyph />
		case 'archive':
			return <ArchiveGlyph />
		case 'pdf':
			return <PdfGlyph />
		case 'document':
			return <DocumentGlyph />
		case 'presentation':
			return <DocumentGlyph />
		case 'text':
			return <TextGlyph />
		case 'spreadsheet':
			return <SpreadsheetGlyph />
		case 'link':
			return <LinkGlyph />
		default:
			return <GenericGlyph />
	}
}

/**
 * @typedef {Object} FileTypeIconProps
 * @property {string} name
 * @property {boolean} [isFile]
 * @property {string} [thumbUrl] when set, shows thumbnail instead of glyph
 * @property {number} [size]
 */

/**
 * Glass/neumorphic file-type tile with gradient SVG glyph.
 * Harmonizes on desktop + mobile; reads light/dark via `data-theme`.
 *
 * @param {FileTypeIconProps} props
 */
const FileTypeIcon = (props) => {
	const size = () => props.size || SIZE
	const kind = () => fileKind(props.name, props.isFile !== false)

	return (
		<Show
			when={props.thumbUrl}
			fallback={
				<Tile sizePx={size()}>
					{glyphForKind(kind())}
				</Tile>
			}
		>
			<Box
				component="img"
				src={props.thumbUrl}
				alt=""
				class="file-type-icon file-type-icon--thumb"
				sx={{
					width: size(),
					height: size(),
					objectFit: 'cover',
				}}
			/>
		</Show>
	)
}

export default FileTypeIcon
export { SIZE as FILE_TYPE_ICON_SIZE }
