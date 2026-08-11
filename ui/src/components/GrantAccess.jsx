import { For, createEffect, createSignal } from 'solid-js'
import Button from '@suid/material/Button'
import TextField from '@suid/material/TextField'
import Dialog from '@suid/material/Dialog'
import DialogActions from '@suid/material/DialogActions'
import DialogContent from '@suid/material/DialogContent'
import DialogTitle from '@suid/material/DialogTitle'
import { useParams } from '@solidjs/router'

import createLocalStore from '../../libs'
import { makeAccessTypeUserFriendly } from './AccessTypeChip'
import API from '../api'
import { alertStore } from './AlertStack'
import { t } from '../common/i18n'

const ACCESS_OPTIONS = () => [
	{ value: 'R', label: t('storageDialogs.accessView'), hint: t('storageDialogs.accessViewHint') },
	{ value: 'W', label: t('storageDialogs.accessEdit'), hint: t('storageDialogs.accessEditHint') },
	{ value: 'A', label: t('storageDialogs.accessAdmin'), hint: t('storageDialogs.accessAdminHint') },
]

/**
 * @typedef {Object} GrantAccessProps
 * @property {boolean} isVisible
 * @property {() => void} onClose
 * @property {() => void} afterGrant
 * @property {string | undefined} email
 * @property {string} [storageId]
 * @property {'R' | 'W' | 'A'} [initialAccessType]
 * @property {string[]} [existingEmails] Already-granted emails to leave out
 *   of the suggestion list — nothing useful about re-suggesting them.
 */

/**
 * @param {GrantAccessProps} props
 */
const GrantAccess = (props) => {
	const { addAlert } = alertStore
	const params = useParams()
	const [store] = createLocalStore()
	const getAction = () =>
		props.email?.length
			? t('storageDialogs.actionChange')
			: t('storageDialogs.actionGrant')
	const storageId = () => props.storageId || params.id
	const [accessType, setAccessType] = createSignal(/** @type {'R' | 'W' | 'A'} */ ('R'))
	// Handing out Admin is the superuser's call — the server answers 403 to
	// anyone else, so the option is not offered rather than offered and refused.
	const accessOptions = () =>
		store.user?.is_superuser
			? ACCESS_OPTIONS()
			: ACCESS_OPTIONS().filter((o) => o.value !== 'A')
	/** @type {[import("solid-js").Accessor<Array<{id: string, email: string}>>, any]} */
	const [directory, setDirectory] = createSignal([])
	const suggestions = () => {
		const taken = new Set(
			(props.existingEmails || []).map((e) => e.toLowerCase()),
		)
		const self = (store.user?.email || '').toLowerCase()
		return directory().filter(
			(u) => !taken.has(u.email.toLowerCase()) && u.email.toLowerCase() !== self,
		)
	}

	createEffect(() => {
		if (!props.isVisible) return
		setAccessType(props.initialAccessType || 'R')
	})

	// Load the directory fresh every time the dialog opens. On failure leave
	// it empty — the TextField still works as free text and the server-side
	// check remains the backstop.
	createEffect(() => {
		if (!props.isVisible) {
			setDirectory([])
			return
		}
		API.users
			.listUserDirectory()
			.then((data) => setDirectory(data?.users || []))
			.catch(() => setDirectory([]))
	})

	/**
	 * @param {SubmitEvent} event
	 */
	const onGrant = async (event) => {
		event.preventDefault()

		const data = new FormData(event.currentTarget)
		const email = props.email || data.get('email')
		const access_type = accessType()

		// Skip the registered-user check when changing an existing grantee's
		// access type — that path never lets the email be edited.
		if (!props.email && directory().length) {
			const known = directory().some(
				(u) => u.email.toLowerCase() === String(email).toLowerCase(),
			)
			if (!known) {
				addAlert(t('storageDialogs.registeredUsersOnly'), 'error')
				return
			}
		}

		await API.access.grantAccess(storageId(), email, access_type)

		props.onClose()
		addAlert(
			t('storageDialogs.grantedAccess', {
				accessType: makeAccessTypeUserFriendly(access_type),
				email,
			}),
			'success',
		)

		props.afterGrant()
	}

	return (
		<>
			<Dialog open={props.isVisible} onClose={props.onClose}>
				<form onSubmit={onGrant}>
					<DialogTitle>
						{t('storageDialogs.accessDialogTitle', { action: getAction() })}
					</DialogTitle>
					<DialogContent>
						<TextField
							required
							defaultValue={props.email}
							disabled={Boolean(props.email)}
							margin="normal"
							id="email"
							label={t('storageDialogs.userEmailLabel')}
							type="email"
							name="email"
							fullWidth
							variant="standard"
							inputProps={{ list: 'grant-access-emails', autocomplete: 'off' }}
						/>
						{/* @suid/material has no Autocomplete component; a native datalist
							gets the same suggest-as-you-type behavior without a new
							dependency, and works on mobile WebViews. */}
						<datalist id="grant-access-emails">
							<For each={suggestions()}>
								{(u) => <option value={u.email} />}
							</For>
						</datalist>

						<div class="access-type-picker">
							<span class="access-type-picker__label">
								{t('storageDialogs.accessLabel')}
							</span>
							<div
								class="access-type-picker__options"
								role="radiogroup"
								aria-label={t('storageDialogs.accessLabel')}
							>
								{accessOptions().map((opt) => (
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
							{t('common.cancel')}
						</Button>
					</DialogActions>
				</form>
			</Dialog>
		</>
	)
}

export default GrantAccess
