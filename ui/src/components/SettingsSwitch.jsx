/** Minimal stub — Task 6 replaces with full styled switch. */
const SettingsSwitch = (props) => (
	<button
		type="button"
		role="switch"
		aria-checked={props.checked ? 'true' : 'false'}
		onClick={() => props.onChange?.(!props.checked)}
	/>
)

export default SettingsSwitch
