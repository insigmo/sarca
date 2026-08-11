import { t } from '../common/i18n'

export const makeAccessTypeUserFriendly = (at) => {
	switch (at) {
		case 'A':
			return t('misc.accessType.admin')
		case 'W':
			return t('misc.accessType.edit')
		case 'R':
			return t('misc.accessType.view')
		default:
			return at
	}
}

/**
 * @typedef {Object} AccessTypeChipProps
 * @property {import('../api').AccessType} at
 */

/**
 * @param {AccessTypeChipProps} props
 */
const AccessTypeChip = (props) => {
	return (
		<span class={`access-chip access-chip--${String(props.at).toLowerCase()}`}>
			{makeAccessTypeUserFriendly(props.at)}
		</span>
	)
}

export default AccessTypeChip
