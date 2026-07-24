import { createEffect, createSignal } from 'solid-js'
import Button from '@suid/material/Button'
import TextField from '@suid/material/TextField'
import Dialog from '@suid/material/Dialog'
import DialogActions from '@suid/material/DialogActions'
import DialogContent from '@suid/material/DialogContent'
import DialogTitle from '@suid/material/DialogTitle'
import { useParams } from '@solidjs/router'

import { makeAccessTypeUserFriendly } from './AccessTypeChip'
import API from '../api'
import { alertStore } from './AlertStack'

const ACCESS_OPTIONS = [
	{ value: 'R', label: 'View', hint: 'Read only' },
	{ value: 'W', label: 'Edit', hint: 'Upload & change' },
	{ value: 'A', label: 'Admin', hint: 'Full control' },
]

/**
 * @typedef {Object} GrantAccessProps
 * @property {boolean} isVisible
 * @property {() => void} onClose
 * @property {() => void} afterGrant
 * @property {string | undefined} email
 * @property {string} [storageId]
 * @property {'R' | 'W' | 'A'} [initialAccessType]
 */

/**
 * @param {GrantAccessProps} props
 */
const GrantAccess = (props) => {
	const { addAlert } = alertStore
	const params = useParams()
	const getAction = () => (props.email?.length ? 'Change' : 'Grant')
	const storageId = () => props.storageId || params.id
	const [accessType, setAccessType] = createSignal(/** @type {'R' | 'W' | 'A'} */ ('R'))

	createEffect(() => {
		if (!props.isVisible) return
		setAccessType(props.initialAccessType || 'R')
	})

	/**
	 * @param {SubmitEvent} event
	 */
	const onGrant = async (event) => {
		event.preventDefault()

		const data = new FormData(event.currentTarget)
		const email = props.email || data.get('email')
		const access_type = accessType()

		await API.access.grantAccess(storageId(), email, access_type)

		props.onClose()
		addAlert(
			`Granted "${makeAccessTypeUserFriendly(access_type)}" access to the user with email "${email}"`,
			'success',
		)

		props.afterGrant()
	}

	return (
		<>
			<Dialog open={props.isVisible} onClose={props.onClose}>
				<form onSubmit={onGrant}>
					<DialogTitle>{getAction()} access</DialogTitle>
					<DialogContent>
						<TextField
							required
							defaultValue={props.email}
							disabled={Boolean(props.email)}
							margin="normal"
							id="email"
							label="User's email"
							type="email"
							name="email"
							fullWidth
							variant="standard"
						/>

						<div class="access-type-picker">
							<span class="access-type-picker__label">Access</span>
							<div
								class="access-type-picker__options"
								role="radiogroup"
								aria-label="Access"
							>
								{ACCESS_OPTIONS.map((opt) => (
									<button
										type="button"
										role="radio"
										aria-checked={accessType() === opt.value}
										class="access-type-option"
										classList={{
											'access-type-option--active': accessType() === opt.value,
											[`access-type-option--${opt.value.toLowerCase()}`]: true,
										}}
										onClick={() =>
											setAccessType(/** @type {'R' | 'W' | 'A'} */ (opt.value))
										}
									>
										<span class="access-type-option__label">{opt.label}</span>
										<span class="access-type-option__hint">{opt.hint}</span>
									</button>
								))}
							</div>
						</div>
					</DialogContent>
					<DialogActions>
						<Button type="submit" color="success">
							{getAction()}
						</Button>

						<Button onClick={props.onClose} color="error">
							Cancel
						</Button>
					</DialogActions>
				</form>
			</Dialog>
		</>
	)
}

export default GrantAccess
