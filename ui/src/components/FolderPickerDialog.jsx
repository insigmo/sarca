import Button from '@suid/material/Button'
import Dialog from '@suid/material/Dialog'
import DialogActions from '@suid/material/DialogActions'
import DialogContent from '@suid/material/DialogContent'
import DialogTitle from '@suid/material/DialogTitle'
import List from '@suid/material/List'
import ListItemButton from '@suid/material/ListItemButton'
import ListItemIcon from '@suid/material/ListItemIcon'
import ListItemText from '@suid/material/ListItemText'
import Typography from '@suid/material/Typography'
import CircularProgress from '@suid/material/CircularProgress'
import FolderOutlinedIcon from '@suid/icons-material/FolderOutlined'
import ArrowUpwardIcon from '@suid/icons-material/ArrowUpward'
import { For, Show, createEffect, createSignal } from 'solid-js'

import API from '../api'

/**
 * @typedef {Object} FolderPickerDialogProps
 * @property {boolean} isOpened
 * @property {string} storageId
 * @property {'copy' | 'move'} mode
 * @property {string} [sourcePath] Normalized source path (folders end with /)
 * @property {string} [itemName] Display name of the item being copied/moved
 * @property {(destinationFolder: string) => void | Promise<void>} onConfirm
 * @property {() => void} onCancel
 */

/**
 * Browse storage folders and pick a destination parent.
 * @param {FolderPickerDialogProps} props
 */
const FolderPickerDialog = (props) => {
	const [browsePath, setBrowsePath] = createSignal('')
	/** @type {[import('solid-js').Accessor<import('../api').FSElement[]>, any]} */
	const [folders, setFolders] = createSignal([])
	const [loading, setLoading] = createSignal(false)
	const [submitting, setSubmitting] = createSignal(false)
	const [error, setError] = createSignal('')

	const normalizeFolderPath = (path) => {
		const p = String(path || '')
		if (!p) return ''
		return p.endsWith('/') ? p.slice(0, -1) : p
	}

	const isBlockedFolder = (folderPath) => {
		const src = props.sourcePath || ''
		if (!src.endsWith('/')) return false
		const dest = folderPath.endsWith('/') ? folderPath : `${folderPath}/`
		// Cannot move/copy a folder into itself or a descendant
		return dest === src || dest.startsWith(src)
	}

	const loadFolders = async (path) => {
		setLoading(true)
		setError('')
		try {
			const layer = await API.files.getFSLayer(props.storageId, path)
			const onlyFolders = layer
				.filter((el) => !el.is_file)
				.filter((el) => !isBlockedFolder(el.path))
				.sort((a, b) =>
					a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }),
				)
			setFolders(onlyFolders)
		} catch (err) {
			console.error(err)
			setFolders([])
			setError(err.message || 'Failed to load folders')
		} finally {
			setLoading(false)
		}
	}

	createEffect(() => {
		if (!props.isOpened) return
		setBrowsePath('')
		setSubmitting(false)
		setError('')
		loadFolders('')
	})

	const displayPath = () => {
		const p = browsePath()
		return p ? `/${p}` : '/'
	}

	const title = () =>
		props.mode === 'copy' ? 'Copy to…' : 'Move to…'

	const confirmLabel = () =>
		props.mode === 'copy' ? 'Copy here' : 'Move here'

	const goUp = () => {
		const p = browsePath()
		if (!p) return
		const parent = p.split('/').slice(0, -1).join('/')
		setBrowsePath(parent)
		loadFolders(parent)
	}

	const enterFolder = (el) => {
		const next = normalizeFolderPath(el.path)
		setBrowsePath(next)
		loadFolders(next)
	}

	const onConfirm = async () => {
		if (submitting()) return
		setSubmitting(true)
		try {
			await props.onConfirm(browsePath())
		} finally {
			setSubmitting(false)
		}
	}

	return (
		<Dialog
			open={props.isOpened}
			onClose={() => {
				if (!submitting()) props.onCancel()
			}}
			fullWidth
			maxWidth="xs"
		>
			<DialogTitle>{title()}</DialogTitle>
			<DialogContent>
				<Show when={props.itemName}>
					<Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
						{props.mode === 'copy' ? 'Copy' : 'Move'} “{props.itemName}”
					</Typography>
				</Show>
				<Typography
					variant="caption"
					display="block"
					sx={{ mb: 1, fontFamily: 'monospace' }}
				>
					Destination: {displayPath()}
				</Typography>

				<Show when={loading()}>
					<div
						style={{
							display: 'grid',
							'place-items': 'center',
							padding: '24px',
						}}
					>
						<CircularProgress size={28} color="secondary" />
					</div>
				</Show>

				<Show when={!loading() && error()}>
					<Typography color="error" variant="body2">
						{error()}
					</Typography>
				</Show>

				<Show when={!loading() && !error()}>
					<List dense sx={{ maxHeight: 280, overflow: 'auto', py: 0 }}>
						<Show when={browsePath()}>
							<ListItemButton onClick={goUp} dense>
								<ListItemIcon sx={{ minWidth: 36 }}>
									<ArrowUpwardIcon fontSize="small" />
								</ListItemIcon>
								<ListItemText primary=".." secondary="Parent folder" />
							</ListItemButton>
						</Show>
						<Show
							when={folders().length}
							fallback={
								<Typography
									variant="body2"
									color="text.secondary"
									sx={{ py: 2, textAlign: 'center' }}
								>
									No subfolders here — you can still{' '}
									{props.mode === 'copy' ? 'copy' : 'move'} into this
									folder.
								</Typography>
							}
						>
							<For each={folders()}>
								{(el) => (
									<ListItemButton onClick={() => enterFolder(el)}>
										<ListItemIcon sx={{ minWidth: 36 }}>
											<FolderOutlinedIcon fontSize="small" />
										</ListItemIcon>
										<ListItemText primary={el.name} />
									</ListItemButton>
								)}
							</For>
						</Show>
					</List>
				</Show>
			</DialogContent>
			<DialogActions>
				<Button
					onClick={onConfirm}
					color="secondary"
					disabled={loading() || submitting() || Boolean(error())}
				>
					{confirmLabel()}
				</Button>
				<Button
					onClick={props.onCancel}
					color="info"
					disabled={submitting()}
				>
					Cancel
				</Button>
			</DialogActions>
		</Dialog>
	)
}

export default FolderPickerDialog
