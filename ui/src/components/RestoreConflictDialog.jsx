import Button from '@suid/material/Button'
import Dialog from '@suid/material/Dialog'
import DialogActions from '@suid/material/DialogActions'
import DialogContent from '@suid/material/DialogContent'
import DialogTitle from '@suid/material/DialogTitle'
import DialogContentText from '@suid/material/DialogContentText'

import { t } from '../common/i18n'

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
	const title = () => props.title || t('folderDialogs.conflict.title')
	const message = () =>
		props.message ||
		t('folderDialogs.conflict.message', { path: props.path })
	const renameLabel = () => props.renameLabel || t('folderDialogs.conflict.rename')

	return (
		<Dialog open={props.isOpened} onClose={props.onCancel}>
			<DialogTitle>{title()}</DialogTitle>
			<DialogContent>
				<DialogContentText>{message()}</DialogContentText>
			</DialogContent>
			<DialogActions>
				<Button onClick={() => props.onChoose('replace')} color="warning">
					{t('folderDialogs.conflict.replace')}
				</Button>
				<Button onClick={() => props.onChoose('rename')} color="secondary">
					{renameLabel()}
				</Button>
				<Button onClick={props.onCancel} color="info">
					{t('folderDialogs.conflict.cancel')}
				</Button>
			</DialogActions>
		</Dialog>
	)
}

export default RestoreConflictDialog
