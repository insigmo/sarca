import Button from '@suid/material/Button'
import Dialog from '@suid/material/Dialog'
import DialogActions from '@suid/material/DialogActions'
import DialogContent from '@suid/material/DialogContent'
import DialogTitle from '@suid/material/DialogTitle'
import DialogContentText from '@suid/material/DialogContentText'

import { t } from '../common/i18n'

/**
 * @typedef {Object} ActionConfirmDialogProps
 * @property {boolean} isOpened
 * @property {string} entity
 * @property {string} action
 * @property {string} actionDescription
 * @property {() => void} onConfirm
 * @property {() => void} onCancel
 */

/**
 *
 * @param {ActionConfirmDialogProps} props
 */
const ActionConfirmDialog = (props) => {
	return (
		<Dialog open={props.isOpened} onClose={props.onCancel}>
			<DialogTitle>
				{t('confirmDialog.title', { action: props.action, entity: props.entity })}
			</DialogTitle>
			<DialogContent>
				<DialogContentText>
					{t('confirmDialog.body', { description: props.actionDescription })}
				</DialogContentText>
			</DialogContent>

			<DialogActions>
				<Button onClick={props.onConfirm} color="warning">
					{t('common.confirm')}
				</Button>
				<Button onClick={props.onCancel} color="info">
					{t('common.cancel')}
				</Button>
			</DialogActions>
		</Dialog>
	)
}

export default ActionConfirmDialog
