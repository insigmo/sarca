import Button from '@suid/material/Button'
import TextField from '@suid/material/TextField'
import Dialog from '@suid/material/Dialog'
import DialogActions from '@suid/material/DialogActions'
import DialogContent from '@suid/material/DialogContent'
import DialogTitle from '@suid/material/DialogTitle'
import { createEffect, createSignal } from 'solid-js'

import { alertStore } from './AlertStack'

/**
 * @typedef {Object} CreateFolderDialogProps
 * @property {boolean} isOpened
 * @property {(folderName: string) => Promise<void>} onCreate
 * @property {() => void} onClose
 */

/**
 *
 * @param {CreateFolderDialogProps} props
 * @returns
 */
const CreateFolderDialog = (props) => {
	const [errFolderName, setErrFolderName] = createSignal(null)
	const [folderName, setFolderName] = createSignal('')
	const [creating, setCreating] = createSignal(false)
	const { addAlert } = alertStore

	let folderNameElement

	createEffect(() => {
		if (props.isOpened) {
			setTimeout(() => folderNameElement.querySelector('input').focus(), 200)
		}
	})

	/**
	 *
	 * @param {SubmitEvent} event
	 */
	const validateFolderName = (event) => {
		event.preventDefault()

		/**
		 * @type {string}
		 */
		const value = event.currentTarget.value

		setErrFolderName(
			value.includes('/') ? 'Folder name cannot have a "/" symbol' : null
		)

		setFolderName(value)
	}

	const onClose = () => {
		setErrFolderName(null)
		setFolderName('')
		props.onClose()
	}

	/**
	 * @param {SubmitEvent} [event]
	 */
	const onCreate = async (event) => {
		event?.preventDefault?.()
		const foldeName = folderName()
		if (creating()) return
		setCreating(true)
		try {
			await props.onCreate(foldeName)
			onClose()
		} catch (err) {
			console.error(err)
			addAlert(
				`Could not create folder "${foldeName}": ${err?.message || 'Unknown error'}`,
				'error',
			)
		} finally {
			setCreating(false)
		}
	}

	return (
		<>
			<Dialog open={props.isOpened} onClose={onClose}>
				<form onSubmit={onCreate}>
					<DialogTitle>Create folder</DialogTitle>
					<DialogContent>
						<TextField
							ref={folderNameElement}
							value={folderName()}
							required
							margin="dense"
							id="folder-name"
							label="New folder name"
							onChange={validateFolderName}
							helperText={errFolderName}
							error={errFolderName() !== null}
							fullWidth
							variant="standard"
						/>
					</DialogContent>
					<DialogActions>
						<Button
							type="submit"
							color="success"
							disabled={!folderName().length || errFolderName() || creating()}
						>
							Create
						</Button>
						<Button onClick={onClose} color="error" disabled={creating()}>
							Cancel
						</Button>
					</DialogActions>
				</form>
			</Dialog>
		</>
	)
}

export default CreateFolderDialog
