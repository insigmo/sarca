import Button from '@suid/material/Button'
import Dialog from '@suid/material/Dialog'
import DialogActions from '@suid/material/DialogActions'
import DialogContent from '@suid/material/DialogContent'
import IconButton from '@suid/material/IconButton'
import { For, Show, createEffect, createMemo, createSignal } from 'solid-js'

import API from '../api'
import { fileExtensionLabel } from '../common/fileLabel'
import { fileKind } from '../common/fileKind'
import { convertSize } from '../common/size_converter'
import { t } from '../common/i18n'
import FileTypeIcon from './FileTypeIcon'
import FluentIcon from './FluentIcon'
import LoadingDots from './LoadingDots'

/**
 * @typedef {Object} FileInfoDialogProps
 * @property {import('../api').FSElement} file
 * @property {string} storageId
 * @property {boolean} isOpened
 * @property {() => void} onClose
 */

const KIND_LABELS = () => ({
	folder: t('viewer.kindFolder'),
	image: t('viewer.kindImage'),
	video: t('viewer.kindVideo'),
	audio: t('viewer.kindAudio'),
	pdf: t('viewer.kindPdf'),
	archive: t('viewer.kindArchive'),
	spreadsheet: t('viewer.kindSpreadsheet'),
	document: t('viewer.kindDocument'),
	presentation: t('viewer.kindPresentation'),
	link: t('viewer.kindLink'),
	markdown: t('viewer.kindMarkdown'),
	html: t('viewer.kindHtml'),
	text: t('viewer.kindText'),
	generic: t('viewer.kindGeneric'),
})

/**
 * @param {number|null|undefined} n
 */
const formatBytesExact = (n) => {
	const v = Number(n)
	if (!Number.isFinite(v)) return '—'
	return t('viewer.bytesExact', { count: Math.max(0, Math.round(v)).toLocaleString('en-US') })
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
			label: t('viewer.labelType'),
			value: KIND_LABELS()[kind()] || (m.is_file ? t('viewer.kindGeneric') : t('viewer.kindFolder')),
		})
		if (m.is_file) {
			out.push({
				label: t('viewer.labelExtension'),
				value: fileExtensionLabel(m.name, true),
			})
		}
		out.push({
			label: t('viewer.labelSize'),
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
			label: t('viewer.labelPath'),
			value: pathFromRoot(m.path),
		})
		out.push({
			label: t('viewer.labelCreated'),
			value: formatTimestamp(m.created_at) || '—',
		})
		out.push({
			label: t('viewer.labelModified'),
			value: formatTimestamp(m.modified_at) || '—',
		})
		out.push({
			label: t('viewer.labelAdded'),
			value: formatTimestamp(m.added_at) || '—',
		})
		if (m.is_favorite != null) {
			out.push({
				label: t('viewer.labelFavorite'),
				value: m.is_favorite ? t('viewer.yes') : t('viewer.no'),
			})
		}
		if (m.deleted_at) {
			out.push({
				label: t('viewer.labelTrashSince'),
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
							{KIND_LABELS()[kind()] || t('viewer.kindItem')}
							{merged()?.is_file
								? ` · ${fileExtensionLabel(merged()?.name || '', true)}`
								: ''}
						</p>
					</div>
				</div>
				<IconButton
					size="small"
					aria-label={t('common.close')}
					onClick={props.onClose}
					class="file-info-dialog__close"
				>
					<FluentIcon name="dismiss" size={18} />
				</IconButton>
			</div>

			<DialogContent class="file-info-dialog__content">
				<Show when={loading()}>
					<div class="file-info-dialog__loading">
						<span>
							{t('viewer.loadingDetails')}
							<LoadingDots />
						</span>
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
					{t('common.close')}
				</Button>
			</DialogActions>
		</Dialog>
	)
}

export default FileInfoDialog
