import SettingsSwitch from './SettingsSwitch'

/** @param {{ checked: boolean, onChange: (checked: boolean) => void }} props */
export default function AppLockToggle(props) {
	return (
		<label class="settings-toggle">
			<span>App lock</span>
			<SettingsSwitch
				id="settings-app-lock-switch"
				checked={props.checked}
				onChange={props.onChange}
			/>
		</label>
	)
}
