import Button from '@suid/material/Button'
import CircularProgress from '@suid/material/CircularProgress'
import Dialog from '@suid/material/Dialog'
import DialogActions from '@suid/material/DialogActions'
import DialogContent from '@suid/material/DialogContent'
import IconButton from '@suid/material/IconButton'
import { For, Show, createEffect, createMemo, createSignal } from 'solid-js'

import API from '../api'
import { fileExtensionLabel } from '../common/fileLabel'
import { fileKind } from '../common/fileKind'
import { convertSize } from '../common/size_converter'
import FileTypeIcon from './FileTypeIcon'
import FluentIcon from './FluentIcon'

/**
 * @typedef {Object} FileInfoDialogProps
 * @property {import('../api').FSElement} file
 * @property {string} storageId
 * @property {boolean} isOpened
 * @property {() => void} onClose
 */

const KIND_LABELS = {
	folder: 'Folder',
	image: 'Image',
	video: 'Video',
	audio: 'Audio',
	pdf: 'PDF',
	archive: 'Archive',
	spreadsheet: 'Spreadsheet',
	document: 'Document',
	presentation: 'Presentation',
	link: 'Link',
	markdown: 'Markdown',
	html: 'HTML',
	text: 'Text',
	generic: 'File',
}

/**
 * @param {number|null|undefined} n
 */
const formatBytesExact = (n) => {
	const v = Number(n)
	if (!Number.isFinite(v)) return '—'
	return `${Math.max(0, Math.round(v)).toLocaleString()} bytes`
}

/**
 * @param {string|number|null|undefined} raw
 */
const formatTimestamp = (raw) => {
	if (raw == null || raw === '') return ''
	const d = new Date(typeof raw === 'number' && raw < 1e12 ? raw * 1000 : raw)
	if (Number.isNaN(d.getTime())) return ''
	return d.toLocaleString()
}

/**
 * @param {string|null|undefined} path
 */
const pathFromRoot = (path) => {
	const p = String(path || '')
		.replace(/^\/+/, '')
		.replace(/\/+$/, '')
	return p ? `/${p}` : '/'
}

/**
 * @param {FileInfoDialogProps} props
 */
const FileInfoDialog = (props) => {
	/** @type {[import('solid-js').Accessor<import('../api').FileInfo | null>, any]} */
	const [detail, setDetail] = createSignal(null)
	const [loading, setLoading] = createSignal(false)

	createEffect(() => {
		if (!props.isOpened || !props.file || !props.storageId) {
			setDetail(null)
			return
		}
		const path = props.file.path
		let cancelled = false
		setLoading(true)
		setDetail(null)
		;(async () => {
			try {
				const info = await API.files.getFileInfo(props.storageId, path)
				if (!cancelled) setDetail(info)
			} catch {
				if (!cancelled) setDetail(null)
			} finally {
				if (!cancelled) setLoading(false)
			}
		})()
		return () => {
			cancelled = true
		}
	})

	const el = () => props.file
	const merged = createMemo(() => {
		const base = el()
		const d = detail()
		if (!base) return null
		return {
			name: d?.name || base.name,
			path: d?.path || base.path,
			size: d?.size ?? base.size,
			is_file: d?.is_file ?? base.is_file,
			deleted_at: d?.deleted_at,
			added_at: d?.added_at,
			created_at: d?.created_at,
			modified_at: d?.modified_at,
			is_favorite: base.is_favorite,
		}
	})

	const kind = () => {
		const m = merged()
		if (!m) return 'generic'
		return fileKind(m.name, m.is_file)
	}

	const rows = createMemo(() => {
		const m = merged()
		if (!m) return []
		/** @type {{ label: string, value: import('solid-js').JSX.Element, title?: string }[]} */
		const out = []
		out.push({
			label: 'Type',
			value: KIND_LABELS[kind()] || (m.is_file ? 'File' : 'Folder'),
		})
		if (m.is_file) {
			out.push({
				label: 'Extension',
				value: fileExtensionLabel(m.name, true),
			})
		}
		out.push({
			label: 'Size',
			value: (
				<>
					{convertSize(m.size)}
					<span class="file-info-dialog__bytes">
						({formatBytesExact(m.size)})
					</span>
				</>
			),
			title: `${convertSize(m.size)} (${formatBytesExact(m.size)})`,
		})
		out.push({
			label: 'Path',
			value: pathFromRoot(m.path),
		})
		out.push({
			label: 'Created',
			value: formatTimestamp(m.created_at) || '—',
		})
		out.push({
			label: 'Modified',
			value: formatTimestamp(m.modified_at) || '—',
		})
		out.push({
			label: 'Added',
			value: formatTimestamp(m.added_at) || '—',
		})
		if (m.is_favorite != null) {
			out.push({
				label: 'Favorite',
				value: m.is_favorite ? 'Yes' : 'No',
			})
		}
		if (m.deleted_at) {
			out.push({
				label: 'In trash since',
				value: new Date(m.deleted_at).toLocaleString(),
			})
		}
		return out
	})

	return (
		<Dialog
			open={props.isOpened}
			onClose={props.onClose}
			maxWidth="xs"
			fullWidth
			classes={{ paper: 'file-info-dialog' }}
		>
			<div class="file-info-dialog__header">
				<div class="file-info-dialog__identity">
					<FileTypeIcon
						name={merged()?.name || ''}
						isFile={merged()?.is_file !== false}
						size={48}
					/>
					<div class="file-info-dialog__titles">
						<h2 class="file-info-dialog__title">{merged()?.name || '—'}</h2>
						<p class="file-info-dialog__subtitle">
							{KIND_LABELS[kind()] || 'Item'}
							{merged()?.is_file
								? ` · ${fileExtensionLabel(merged()?.name || '', true)}`
								: ''}
						</p>
					</div>
				</div>
				<IconButton
					size="small"
					aria-label="Close"
					onClick={props.onClose}
					class="file-info-dialog__close"
				>
					<FluentIcon name="dismiss" size={18} />
				</IconButton>
			</div>

			<DialogContent class="file-info-dialog__content">
				<Show when={loading()}>
					<div class="file-info-dialog__loading">
						<CircularProgress size={22} color="secondary" />
						<span>Loading details…</span>
					</div>
				</Show>
				<dl class="file-info-dialog__list">
					<For each={rows()}>
						{(row) => (
							<div class="file-info-dialog__row">
								<dt>{row.label}</dt>
								<dd
									title={
										row.title ??
										(typeof row.value === 'string'
											? row.value
											: undefined)
									}
								>
									{row.value}
								</dd>
							</div>
						)}
					</For>
				</dl>
			</DialogContent>

			<DialogActions class="file-info-dialog__actions">
				<Button onClick={props.onClose} color="secondary" variant="contained">
					Close
				</Button>
			</DialogActions>
		</Dialog>
	)
}

export default FileInfoDialog
