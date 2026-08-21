import logoUrl from '../assets/logo.svg'

/**
 * @param {{ size?: number, class?: string }} props
 */
const AppIcon = (props) => {
	const size = () => props.size ?? 36

	return (
		<img
			src={logoUrl}
			alt="Sarca"
			width={size()}
			height={size()}
			class={props.class}
			style={{
				display: 'block',
				'object-fit': 'contain',
			}}
		/>
	)
}

export default AppIcon
