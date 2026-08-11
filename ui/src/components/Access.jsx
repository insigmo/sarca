import IconButton from '@suid/material/IconButton'
import { For, Show, createSignal, onMount } from 'solid-js'
import { useParams } from '@solidjs/router'

import createLocalStore from '../../libs'
import AccessTypeChip from './AccessTypeChip'
import API from '../api'
import ActionConfirmDialog from './ActionConfirmDialog'
import { alertStore } from './AlertStack'
import FluentIcon from './FluentIcon'
import GrantAccess from './GrantAccess'
import { t } from '../common/i18n'

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

	/**
	 * Admin rows are the superuser's to manage: the server refuses anything else
	 * (403), so the buttons are disabled rather than left to fail on click. The
	 * caller's own row stays disabled as before.
	 */
	const canManage = (user) =>
		store.user?.email !== user.email &&
		(!!store.user?.is_superuser || String(user.access_type).toUpperCase() !== 'A')

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
		try {
			const userID = props.users.find(
				(u) => u.email === selectedUserEmail(),
			).id

			await API.access.restrictAccess(storageId(), userID)
			setIsRestrictConfirmOpened(false)
			addAlert(
				t('storageDialogs.restrictedAccess', { email: selectedUserEmail() }),
				'success',
			)

			await props.refetchUsers()
		} catch (err) {
			console.error(err)
			// Keep confirm dialog open; apiRequest already shows the error alert.
		}
	}

	return (
		<>
			<div class="access-list">
				<Show
					when={props.users.length}
					fallback={
						<p class="access-list__empty">{t('storageDialogs.noUsersWithAccess')}</p>
					}
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
										disabled={!canManage(user)}
										aria-label={t('storageDialogs.editAccessAria', {
											email: user.email,
										})}
										onClick={() => onEditButtonClicked(user)}
									>
										<FluentIcon name="edit" size={18} />
									</IconButton>
									<IconButton
										size="small"
										disabled={!canManage(user)}
										aria-label={t('storageDialogs.removeAccessAria', {
											email: user.email,
										})}
										onClick={() => onDeleteButtonClicked(user.email)}
									>
										<FluentIcon name="delete" size={18} />
									</IconButton>
								</div>
							</div>
						)}
					</For>
				</Show>
			</div>

			<ActionConfirmDialog
				action={t('storageDialogs.restrictAction')}
				actionDescription={t('storageDialogs.restrictAccessDescription', {
					email: selectedUserEmail(),
				})}
				entity={t('storageDialogs.accessEntity')}
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
