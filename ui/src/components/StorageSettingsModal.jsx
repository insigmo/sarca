import { Show, createEffect, createSignal, onCleanup } from 'solid-js'
import IconButton from '@suid/material/IconButton'
import Button from '@suid/material/Button'
import TextField from '@suid/material/TextField'
import CloseIcon from '@suid/icons-material/Close'
import DeleteIcon from '@suid/icons-material/Delete'

import API from '../api'
import { alertStore } from './AlertStack'
import ActionConfirmDialog from './ActionConfirmDialog'
import WaveDivider from './WaveDivider'

/**
 * @typedef {Object} StorageSettingsModalProps
 * @property {import('../api').StorageWithInfo | null} storage
 * @property {() => void} onClose
 * @property {(storage: import('../api').StorageWithInfo) => void} onRenamed
 * @property {(storageId: string) => void} onDeleted
 */

/**
 * @param {StorageSettingsModalProps} props
 */
const StorageSettingsModal = (props) => {
	const { addAlert } = alertStore
	const [name, setName] = createSignal('')
	const [saving, setSaving] = createSignal(false)
	const [confirmDelete, setConfirmDelete] = createSignal(false)

	createEffect(() => {
		const storage = props.storage
		if (!storage) {
			setConfirmDelete(false)
			return
		}

		setName(storage.name)
		setConfirmDelete(false)
		document.body.style.overflow = 'hidden'

		const onKeyDown = (e) => {
			if (e.key === 'Escape') {
				if (confirmDelete()) setConfirmDelete(false)
				else props.onClose()
			}
		}
		window.addEventListener('keydown', onKeyDown)

		onCleanup(() => {
			document.body.style.overflow = ''
			window.removeEventListener('keydown', onKeyDown)
		})
	})

	const saveName = async (e) => {
		e.preventDefault()
		const storage = props.storage
		if (!storage) return

		const next = name().trim()
		if (!next) {
			addAlert('Storage name is required', 'error')
			return
		}
		if (next === storage.name) {
			props.onClose()
			return
		}

		setSaving(true)
		try {
			const updated = await API.storages.renameStorage(storage.id, next)
			addAlert(`Renamed storage to "${updated.name}"`, 'success')
			props.onRenamed({ ...storage, name: updated.name })
			props.onClose()
		} catch (err) {
			console.error(err)
		} finally {
			setSaving(false)
		}
	}

	const deleteStorage = async () => {
		const storage = props.storage
		if (!storage) return

		setSaving(true)
		try {
			await API.storages.deleteStorage(storage.id)
			addAlert(`Deleted storage "${storage.name}" and all its files`, 'success')
			setConfirmDelete(false)
			props.onDeleted(storage.id)
			props.onClose()
		} catch (err) {
			console.error(err)
		} finally {
			setSaving(false)
		}
	}

	return (
		<>
			<Show when={props.storage}>
				<div
					class="settings-overlay"
					onClick={(e) => {
						if (e.target === e.currentTarget) props.onClose()
					}}
					role="presentation"
				>
					<div
						class="settings-modal settings-modal--storage"
						role="dialog"
						aria-modal="true"
						aria-labelledby="storage-settings-title"
						onClick={(e) => e.stopPropagation()}
					>
						<div class="settings-modal__header">
							<div>
								<h2 id="storage-settings-title">Storage settings</h2>
								<p class="settings-modal__sub">
									Rename or permanently delete this storage
								</p>
							</div>
							<IconButton
								aria-label="Close storage settings"
								onClick={props.onClose}
								class="sarca-header-icon"
								size="small"
							>
								<CloseIcon />
							</IconButton>
						</div>

						<WaveDivider class="settings-modal__wave" />

						<div class="settings-modal__body">
							<div class="storage-settings-meta">
								<span class="storage-settings-meta__label">Telegram chat</span>
								<span class="storage-settings-meta__value">
									{props.storage.chat_id}
								</span>
							</div>

							<form class="storage-settings-form" onSubmit={saveName}>
								<TextField
									label="Name"
									name="name"
									value={name()}
									onChange={(_, v) => setName(v)}
									fullWidth
									required
									autoFocus
									disabled={saving()}
								/>
								<div class="storage-settings-form__actions">
									<Button
										type="submit"
										variant="contained"
										color="secondary"
										disabled={saving() || !name().trim()}
									>
										Save
									</Button>
								</div>
							</form>

							<div class="storage-settings-danger">
								<h3>Danger zone</h3>
								<p>
									Delete this storage together with all files. This cannot be
									undone.
								</p>
								<Button
									variant="outlined"
									color="error"
									startIcon={<DeleteIcon />}
									disabled={saving()}
									onClick={() => setConfirmDelete(true)}
								>
									Delete storage and files
								</Button>
							</div>
						</div>
					</div>
				</div>
			</Show>

			<ActionConfirmDialog
				isOpened={confirmDelete()}
				entity="storage"
				action="Delete"
				actionDescription={`permanently delete storage "${props.storage?.name || ''}" and all its files`}
				onConfirm={deleteStorage}
				onCancel={() => setConfirmDelete(false)}
			/>
		</>
	)
}

export default StorageSettingsModal
