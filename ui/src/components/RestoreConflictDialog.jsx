import Button from '@suid/material/Button'
import Dialog from '@suid/material/Dialog'
import DialogActions from '@suid/material/DialogActions'
import DialogContent from '@suid/material/DialogContent'
import DialogTitle from '@suid/material/DialogTitle'
import DialogContentText from '@suid/material/DialogContentText'

/**
 * @typedef {Object} RestoreConflictDialogProps
 * @property {boolean} isOpened
 * @property {string} path
 * @property {(choice: 'replace' | 'rename') => void} onChoose
 * @property {() => void} onCancel
 */

/**
 * @param {RestoreConflictDialogProps} props
 */
const RestoreConflictDialog = (props) => {
	return (
		<Dialog open={props.isOpened} onClose={props.onCancel}>
			<DialogTitle>Path already exists</DialogTitle>
			<DialogContent>
				<DialogContentText>
					A live file or folder already exists at “{props.path}”. Replace it,
					restore under a new name, or cancel?
				</DialogContentText>
			</DialogContent>
			<DialogActions>
				<Button onClick={() => props.onChoose('replace')} color="warning">
					Replace
				</Button>
				<Button onClick={() => props.onChoose('rename')} color="secondary">
					Rename
				</Button>
				<Button onClick={props.onCancel} color="info">
					Cancel
				</Button>
			</DialogActions>
		</Dialog>
	)
}

export default RestoreConflictDialog
