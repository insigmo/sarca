import add24Regular from '@fluentui/svg-icons/icons/add_24_regular.svg?raw'
import arrowDownload24Regular from '@fluentui/svg-icons/icons/arrow_download_24_regular.svg?raw'
import arrowMove24Regular from '@fluentui/svg-icons/icons/arrow_move_24_regular.svg?raw'
import arrowSort24Regular from '@fluentui/svg-icons/icons/arrow_sort_24_regular.svg?raw'
import arrowSync24Regular from '@fluentui/svg-icons/icons/arrow_sync_24_regular.svg?raw'
import arrowUndo24Regular from '@fluentui/svg-icons/icons/arrow_undo_24_regular.svg?raw'
import arrowUp24Regular from '@fluentui/svg-icons/icons/arrow_up_24_regular.svg?raw'
import chevronDown24Regular from '@fluentui/svg-icons/icons/chevron_down_24_regular.svg?raw'
import chevronLeft24Regular from '@fluentui/svg-icons/icons/chevron_left_24_regular.svg?raw'
import chevronRight24Regular from '@fluentui/svg-icons/icons/chevron_right_24_regular.svg?raw'
import cloud24Regular from '@fluentui/svg-icons/icons/cloud_24_regular.svg?raw'
import cloud24Filled from '@fluentui/svg-icons/icons/cloud_24_filled.svg?raw'
import copy24Regular from '@fluentui/svg-icons/icons/copy_24_regular.svg?raw'
import delete24Regular from '@fluentui/svg-icons/icons/delete_24_regular.svg?raw'
import delete24Filled from '@fluentui/svg-icons/icons/delete_24_filled.svg?raw'
import deleteDismiss24Regular from '@fluentui/svg-icons/icons/delete_dismiss_24_regular.svg?raw'
import dismiss24Regular from '@fluentui/svg-icons/icons/dismiss_24_regular.svg?raw'
import documentArrowUp24Regular from '@fluentui/svg-icons/icons/document_arrow_up_24_regular.svg?raw'
import edit24Regular from '@fluentui/svg-icons/icons/edit_24_regular.svg?raw'
import eye24Regular from '@fluentui/svg-icons/icons/eye_24_regular.svg?raw'
import folder24Regular from '@fluentui/svg-icons/icons/folder_24_regular.svg?raw'
import folder24Filled from '@fluentui/svg-icons/icons/folder_24_filled.svg?raw'
import folderAdd24Regular from '@fluentui/svg-icons/icons/folder_add_24_regular.svg?raw'
import folderArrowUp24Regular from '@fluentui/svg-icons/icons/folder_arrow_up_24_regular.svg?raw'
import grid24Regular from '@fluentui/svg-icons/icons/grid_24_regular.svg?raw'
import grid24Filled from '@fluentui/svg-icons/icons/grid_24_filled.svg?raw'
import history24Regular from '@fluentui/svg-icons/icons/history_24_regular.svg?raw'
import history24Filled from '@fluentui/svg-icons/icons/history_24_filled.svg?raw'
import info24Regular from '@fluentui/svg-icons/icons/info_24_regular.svg?raw'
import link24Regular from '@fluentui/svg-icons/icons/link_24_regular.svg?raw'
import link24Filled from '@fluentui/svg-icons/icons/link_24_filled.svg?raw'
import list24Regular from '@fluentui/svg-icons/icons/list_24_regular.svg?raw'
import list24Filled from '@fluentui/svg-icons/icons/list_24_filled.svg?raw'
import lockClosed24Regular from '@fluentui/svg-icons/icons/lock_closed_24_regular.svg?raw'
import lockClosed24Filled from '@fluentui/svg-icons/icons/lock_closed_24_filled.svg?raw'
import navigation24Regular from '@fluentui/svg-icons/icons/navigation_24_regular.svg?raw'
import options24Regular from '@fluentui/svg-icons/icons/options_24_regular.svg?raw'
import person24Regular from '@fluentui/svg-icons/icons/person_24_regular.svg?raw'
import person24Filled from '@fluentui/svg-icons/icons/person_24_filled.svg?raw'
import plugDisconnected24Regular from '@fluentui/svg-icons/icons/plug_disconnected_24_regular.svg?raw'
import rename24Regular from '@fluentui/svg-icons/icons/rename_24_regular.svg?raw'
import search24Regular from '@fluentui/svg-icons/icons/search_24_regular.svg?raw'
import settings24Regular from '@fluentui/svg-icons/icons/settings_24_regular.svg?raw'
import signOut24Regular from '@fluentui/svg-icons/icons/sign_out_24_regular.svg?raw'
import star24Regular from '@fluentui/svg-icons/icons/star_24_regular.svg?raw'
import star24Filled from '@fluentui/svg-icons/icons/star_24_filled.svg?raw'
import storage24Regular from '@fluentui/svg-icons/icons/storage_24_regular.svg?raw'
import storage24Filled from '@fluentui/svg-icons/icons/storage_24_filled.svg?raw'
import warning24Regular from '@fluentui/svg-icons/icons/warning_24_regular.svg?raw'

/**
 * Named Fluent icons used by Sarca chrome.
 * Import raw SVG strings for one-offs; use names for shared chrome glyphs.
 *
 * @type {Record<string, string>}
 */
export const fluentIcons = {
	add: add24Regular,
	arrowDownload: arrowDownload24Regular,
	arrowMove: arrowMove24Regular,
	arrowSort: arrowSort24Regular,
	arrowSync: arrowSync24Regular,
	arrowUndo: arrowUndo24Regular,
	arrowUp: arrowUp24Regular,
	chevronDown: chevronDown24Regular,
	chevronLeft: chevronLeft24Regular,
	chevronRight: chevronRight24Regular,
	cloud: cloud24Regular,
	cloudFilled: cloud24Filled,
	copy: copy24Regular,
	delete: delete24Regular,
	deleteFilled: delete24Filled,
	deleteDismiss: deleteDismiss24Regular,
	dismiss: dismiss24Regular,
	documentArrowUp: documentArrowUp24Regular,
	edit: edit24Regular,
	eye: eye24Regular,
	folder: folder24Regular,
	folderFilled: folder24Filled,
	folderAdd: folderAdd24Regular,
	folderArrowUp: folderArrowUp24Regular,
	grid: grid24Regular,
	gridFilled: grid24Filled,
	history: history24Regular,
	historyFilled: history24Filled,
	info: info24Regular,
	link: link24Regular,
	linkFilled: link24Filled,
	list: list24Regular,
	listFilled: list24Filled,
	lockClosed: lockClosed24Regular,
	lockClosedFilled: lockClosed24Filled,
	navigation: navigation24Regular,
	options: options24Regular,
	person: person24Regular,
	personFilled: person24Filled,
	plugDisconnected: plugDisconnected24Regular,
	rename: rename24Regular,
	search: search24Regular,
	settings: settings24Regular,
	signOut: signOut24Regular,
	star: star24Regular,
	starFilled: star24Filled,
	storage: storage24Regular,
	storageFilled: storage24Filled,
	warning: warning24Regular,
}

/**
 * Inline Fluent System Icon (SVG string).
 *
 * `name` indexes the bundled icon table above and is the only way to reach the
 * `innerHTML` sink. The component deliberately takes no raw-markup prop: an
 * `src`-style escape hatch would render caller-supplied SVG unsanitized, and
 * SVG carries script (`<script>`, `on*`, `<foreignObject>`). Anything dynamic
 * must go through `sanitizeHtml` at the call site instead.
 *
 * @param {{
 *   name?: keyof typeof fluentIcons | string,
 *   size?: number,
 *   class?: string,
 *   classList?: Record<string, boolean>,
 *   title?: string,
 *   'aria-label'?: string,
 *   'aria-hidden'?: boolean | 'true' | 'false',
 * }} props
 */
const FluentIcon = (props) => {
	const svg = () => {
		if (props.name && Object.hasOwn(fluentIcons, props.name)) {
			return fluentIcons[props.name]
		}
		return ''
	}

	const size = () => props.size ?? 20
	const decorative = () =>
		props['aria-hidden'] !== false &&
		props['aria-hidden'] !== 'false' &&
		!props['aria-label']

	return (
		<span
			class={`fluent-icon${props.class ? ` ${props.class}` : ''}`}
			classList={props.classList}
			style={{
				'font-size': `${size()}px`,
				width: '1em',
				height: '1em',
			}}
			title={props.title}
			aria-hidden={decorative() ? 'true' : undefined}
			aria-label={props['aria-label']}
			role={props['aria-label'] ? 'img' : undefined}
			innerHTML={svg()}
		/>
	)
}

export default FluentIcon
