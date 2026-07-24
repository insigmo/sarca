import Button from '@suid/material/Button'
import Dialog from '@suid/material/Dialog'
import DialogActions from '@suid/material/DialogActions'
import DialogContent from '@suid/material/DialogContent'
import DialogTitle from '@suid/material/DialogTitle'
import DialogContentText from '@suid/material/DialogContentText'

/**
 * Shared Replace / Rename / Cancel dialog for path conflicts
 * (trash restore, copy, move).
 *
 * @typedef {Object} RestoreConflictDialogProps
 * @property {boolean} isOpened
 * @property {string} path
 * @property {(choice: 'replace' | 'rename') => void} onChoose
 * @property {() => void} onCancel
 * @property {string} [title]
 * @property {string} [message]
 * @property {string} [renameLabel]
 */

/**
 * @param {RestoreConflictDialogProps} props
 */
const RestoreConflictDialog = (props) => {
	const title = () => props.title || 'Path already exists'
	const message = () =>
		props.message ||
		`A live file or folder already exists at “${props.path}”. Replace it, continue under a new name, or cancel?`
	const renameLabel = () => props.renameLabel || 'Rename'

	return (
		<Dialog open={props.isOpened} onClose={props.onCancel}>
			<DialogTitle>{title()}</DialogTitle>
			<DialogContent>
				<DialogContentText>{message()}</DialogContentText>
			</DialogContent>
			<DialogActions>
				<Button onClick={() => props.onChoose('replace')} color="warning">
					Replace
				</Button>
				<Button onClick={() => props.onChoose('rename')} color="secondary">
					{renameLabel()}
				</Button>
				<Button onClick={props.onCancel} color="info">
					Cancel
				</Button>
			</DialogActions>
		</Dialog>
	)
}

export default RestoreConflictDialog
