import Alert from '@suid/material/Alert'
import Stack from '@suid/material/Stack'
import { For, createRoot, createSignal } from 'solid-js'

import { t } from '../common/i18n'

/**
 * @typedef {"error" | "warning" | "info" | "success"} AlertSeverity
 */

/**
 * @typedef {Object} AlertType
 * @property {string} id
 * @property {string} msg
 * @property {AlertSeverity} severity
 */

let alertSeq = 0

export const alertStore = createRoot(() => {
	/**
	 * @type {[import("solid-js").Accessor<AlertType[]>, import("solid-js").Setter<AlertType[]>]}
	 */
	const [alertList, setAlertList] = createSignal([])

	/**
	 * @param {string} id
	 */
	const dismissAlert = (id) => {
		setAlertList((list) => list.filter((a) => a.id !== id))
	}

	/**
	 * @param {string} msg
	 * @param {AlertSeverity} severity
	 */
	const addAlert = (msg, severity) => {
		const id = `alert-${++alertSeq}`
		setAlertList((list) => [{ id, msg, severity }, ...list])
		setTimeout(() => dismissAlert(id), 5e3)
	}

	return { alertList, addAlert, dismissAlert }
})

const AlertStack = () => {
	const { alertList, dismissAlert } = alertStore

	return (
		<Stack
			class="alert-stack"
			sx={{
				position: 'fixed',
				zIndex: 99999,
				right: '1rem',
				// The old offset added a hardcoded 56px for an app bar that no
				// longer exists, and it applied on /login too, where there never
				// was one. Only the real status-bar inset matters now — on a
				// device with a tall cutout the text used to run under the tray.
				top: 'calc(1rem + max(var(--sarca-safe-top), var(--sarca-android-top)))',
				left: 'auto',
				maxWidth: 360,
				width: '30vw',
				minWidth: 240,
				'@media (max-width: 840px)': {
					// Phones: full width minus the gutters, so a long message
					// wraps instead of growing a tall narrow column.
					left: '1rem',
					width: 'auto',
					maxWidth: 'none',
					minWidth: 0,
				},
			}}
			spacing={1}
		>
			<For each={alertList()}>
				{(alert) => (
					<Alert
						class="alert-stack__item"
						severity={alert.severity}
						onClose={() => dismissAlert(alert.id)}
						closeText={t('misc.alerts.close')}
					>
						{alert.msg}
					</Alert>
				)}
			</For>
		</Stack>
	)
}

export default AlertStack
