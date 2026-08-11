import Button from '@suid/material/Button'
import TextField from '@suid/material/TextField'
import Dialog from '@suid/material/Dialog'
import DialogActions from '@suid/material/DialogActions'
import DialogContent from '@suid/material/DialogContent'
import DialogTitle from '@suid/material/DialogTitle'
import { createEffect, createSignal } from 'solid-js'

import { t } from '../common/i18n'

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
			value.includes('/') ? t('folderDialogs.create.nameError') : null
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
			// apiRequest already surfaces an error alert; just keep the
			// dialog open with the typed name so the user can retry.
			console.error(err)
		} finally {
			setCreating(false)
		}
	}

	return (
		<>
			<Dialog open={props.isOpened} onClose={onClose}>
				<form onSubmit={onCreate}>
					<DialogTitle>{t('folderDialogs.create.title')}</DialogTitle>
					<DialogContent>
						<TextField
							ref={folderNameElement}
							value={folderName()}
							required
							margin="dense"
							id="folder-name"
							label={t('folderDialogs.create.nameLabel')}
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
							{t('folderDialogs.create.submit')}
						</Button>
						<Button onClick={onClose} color="error" disabled={creating()}>
							{t('folderDialogs.create.cancel')}
						</Button>
					</DialogActions>
				</form>
			</Dialog>
		</>
	)
}

export default CreateFolderDialog
