import Button from '@suid/material/Button'
import CircularProgress from '@suid/material/CircularProgress'
import Dialog from '@suid/material/Dialog'
import DialogActions from '@suid/material/DialogActions'
import DialogContent from '@suid/material/DialogContent'
import IconButton from '@suid/material/IconButton'
import CloseIcon from '@suid/icons-material/Close'
import { For, Show, createEffect, createMemo, createSignal } from 'solid-js'

import API from '../api'
import { fileExtensionLabel } from '../common/fileLabel'
import { fileKind } from '../common/fileKind'
import { convertSize } from '../common/size_converter'
import FileTypeIcon from './FileTypeIcon'

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
 * @param {string} path
 * @param {boolean} isFile
 */
const parentLocation = (path, isFile) => {
	const raw = String(path || '').replace(/\/+$/, '')
	if (!raw) return '/'
	const parts = raw.split('/')
	if (parts.length <= 1) return '/'
	return parts.slice(0, -1).join('/') || '/'
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
			has_thumb: d?.has_thumb ?? base.has_thumb,
			is_uploaded: d?.is_uploaded,
			chunk_size_bytes: d?.chunk_size_bytes,
			chunks_count: d?.chunks_count,
			content_type: d?.content_type,
			deleted_at: d?.deleted_at,
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
		/** @type {{ label: string, value: string }[]} */
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
			value: `${convertSize(m.size)} (${formatBytesExact(m.size)})`,
		})
		out.push({
			label: 'Location',
			value: parentLocation(m.path, m.is_file),
		})
		out.push({
			label: 'Path',
			value: m.path || '/',
		})
		if (m.content_type) {
			out.push({ label: 'MIME', value: m.content_type })
		}
		if (m.is_file) {
			out.push({
				label: 'Thumbnail',
				value: m.has_thumb ? 'Available' : 'None',
			})
			if (m.is_uploaded != null) {
				out.push({
					label: 'Upload',
					value: m.is_uploaded ? 'Complete' : 'Pending',
				})
			}
			if (m.chunks_count != null && m.chunks_count > 0) {
				out.push({
					label: 'Chunks',
					value: String(m.chunks_count),
				})
			}
			if (m.chunk_size_bytes) {
				out.push({
					label: 'Chunk size',
					value: convertSize(m.chunk_size_bytes),
				})
			}
		}
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
					<CloseIcon fontSize="small" />
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
								<dd title={row.value}>{row.value}</dd>
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
