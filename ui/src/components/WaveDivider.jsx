/**
 * Organic SVG wave separator (reference-style soft S-curve).
 *
 * @param {{ flip?: boolean, class?: string, fill?: string, style?: Record<string, string> | string }} props
 */
const WaveDivider = (props) => (
	<svg
		class={`wave-divider ${props.class || ''}`}
		classList={{ 'wave-divider--flip': props.flip }}
		viewBox="0 0 1440 64"
		preserveAspectRatio="none"
		aria-hidden="true"
		style={props.style}
	>
		<path
			d="M0,32 C240,64 360,0 720,24 C1080,48 1200,8 1440,36 L1440,64 L0,64 Z"
			fill={props.fill || 'currentColor'}
		/>
	</svg>
)

export default WaveDivider
