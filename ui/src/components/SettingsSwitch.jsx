export default function SettingsSwitch(props) {
	const checked = () => Boolean(props.checked)
	const disabled = () => Boolean(props.disabled)
	const toggle = (e) => {
		// Prevent <label> ancestors from synthesizing a second click on this
		// button (would flip the switch twice → auto-upload appears to
		// enable then immediately disable).
		e?.preventDefault?.()
		e?.stopPropagation?.()
		if (disabled()) return
		props.onChange?.(!checked())
	}
	const onKeyDown = (e) => {
		if (e.key === ' ' || e.key === 'Enter') {
			e.preventDefault()
			e.stopPropagation()
			toggle(e)
		}
	}
	return (
		<button
			type="button"
			id={props.id}
			role="switch"
			class="settings-switch"
			aria-checked={checked() ? 'true' : 'false'}
			aria-label={props.ariaLabel}
			disabled={disabled()}
			onClick={toggle}
			onKeyDown={onKeyDown}
		>
			<span class="settings-switch__thumb" aria-hidden="true" />
		</button>
	)
}
