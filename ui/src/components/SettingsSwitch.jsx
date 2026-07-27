export default function SettingsSwitch(props) {
	const checked = () => Boolean(props.checked)
	const disabled = () => Boolean(props.disabled)
	const toggle = () => {
		if (disabled()) return
		props.onChange?.(!checked())
	}
	const onKeyDown = (e) => {
		if (e.key === ' ' || e.key === 'Enter') {
			e.preventDefault()
			toggle()
		}
	}
	return (
		<button
			type="button"
			id={props.id}
			role="switch"
			class="settings-switch"
			aria-checked={checked() ? 'true' : 'false'}
			disabled={disabled()}
			onClick={toggle}
			onKeyDown={onKeyDown}
		>
			<span class="settings-switch__thumb" aria-hidden="true" />
		</button>
	)
}
