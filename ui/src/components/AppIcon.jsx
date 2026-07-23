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
				'border-radius': '10px',
				display: 'block',
			}}
		/>
	)
}

export default AppIcon
