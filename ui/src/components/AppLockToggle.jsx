import { t } from '../common/i18n'
import SettingsSwitch from './SettingsSwitch'

/** @param {{ checked: boolean, onChange: (checked: boolean) => void }} props */
export default function AppLockToggle(props) {
	return (
		<div class="settings-toggle">
			<span>{t('settings.appLock')}</span>
			<SettingsSwitch
				id="settings-app-lock-switch"
				checked={props.checked}
				onChange={props.onChange}
			/>
		</div>
	)
}
