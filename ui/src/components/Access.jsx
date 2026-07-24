import IconButton from '@suid/material/IconButton'
import DeleteIcon from '@suid/icons-material/Delete'
import EditIcon from '@suid/icons-material/Edit'
import { For, Show, createSignal, onMount } from 'solid-js'
import { useParams } from '@solidjs/router'

import createLocalStore from '../../libs'
import AccessTypeChip from './AccessTypeChip'
import API from '../api'
import ActionConfirmDialog from './ActionConfirmDialog'
import { alertStore } from './AlertStack'
import GrantAccess from './GrantAccess'

/**
 * @typedef {Object} AccessProps
 * @property {() => void} [setIsGrantAccessVisible]
 * @property {() => void} onMount
 * @property {import('../api').UserWithAccess[]} users
 * @property {() => Promise<void>} refetchUsers
 * @property {string} [storageId]
 */

/**
 * @param {AccessProps} props
 */
const Access = (props) => {
	const [selectedUserEmail, setSelectedUserEmail] = createSignal()
	const [selectedAccessType, setSelectedAccessType] = createSignal(
		/** @type {'R' | 'W' | 'A' | undefined} */ (undefined),
	)
	const [isRestrictConfirmOpened, setIsRestrictConfirmOpened] =
		createSignal(false)
	const [isChangeAccessOpened, setIsChangeAccessOpened] = createSignal(false)
	const [store, _setStore] = createLocalStore()
	const { addAlert } = alertStore
	const params = useParams()
	const storageId = () => props.storageId || params.id

	onMount(props.onMount)

	const onEditButtonClicked = (user) => {
		setSelectedUserEmail(user.email)
		setSelectedAccessType(user.access_type)
		setIsChangeAccessOpened(true)
	}

	const onChangeAccess = async () => {
		setIsChangeAccessOpened(false)
		await props.refetchUsers()
	}

	const onDeleteButtonClicked = (email) => {
		setSelectedUserEmail(email)
		setIsRestrictConfirmOpened(true)
	}

	const onRestrict = async () => {
		const userID = props.users.find((u) => u.email === selectedUserEmail()).id

		await API.access.restrictAccess(storageId(), userID)
		addAlert(
			`Restricted access for the user with email ${selectedUserEmail()}`,
			'success',
		)

		await props.refetchUsers()
	}

	return (
		<>
			<div class="access-list">
				<Show
					when={props.users.length}
					fallback={<p class="access-list__empty">No users with access yet</p>}
				>
					<For each={props.users}>
						{(user) => (
							<div class="access-row">
								<span class="access-row__email" title={user.email}>
									{user.email}
								</span>
								<AccessTypeChip at={user.access_type} />
								<div class="access-row__actions">
									<IconButton
										size="small"
										disabled={store.user?.email === user.email}
										aria-label={`Edit access for ${user.email}`}
										onClick={() => onEditButtonClicked(user)}
									>
										<EditIcon fontSize="small" />
									</IconButton>
									<IconButton
										size="small"
										disabled={store.user?.email === user.email}
										aria-label={`Remove access for ${user.email}`}
										onClick={() => onDeleteButtonClicked(user.email)}
									>
										<DeleteIcon fontSize="small" />
									</IconButton>
								</div>
							</div>
						)}
					</For>
				</Show>
			</div>

			<ActionConfirmDialog
				action="Restrict"
				actionDescription={`restrict access for the user with email "${selectedUserEmail()}"`}
				entity="access"
				isOpened={isRestrictConfirmOpened()}
				onCancel={() => setIsRestrictConfirmOpened(false)}
				onConfirm={onRestrict}
			/>

			<GrantAccess
				afterGrant={onChangeAccess}
				email={selectedUserEmail()}
				initialAccessType={selectedAccessType()}
				isVisible={isChangeAccessOpened()}
				onClose={() => setIsChangeAccessOpened(false)}
				storageId={storageId()}
			/>
		</>
	)
}

export default Access
