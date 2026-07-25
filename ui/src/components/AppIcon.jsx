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
				'border-radius': '12px',
				display: 'block',
				'box-shadow': '0 6px 16px rgba(0, 120, 212, 0.28)',
			}}
		/>
	)
}

export default AppIcon
