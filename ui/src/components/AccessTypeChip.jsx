export const makeAccessTypeUserFriendly = (at) => {
	switch (at) {
		case 'A':
			return 'Admin'
		case 'W':
			return 'Edit'
		case 'R':
			return 'View'
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
