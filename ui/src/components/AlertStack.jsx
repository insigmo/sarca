import Alert from '@suid/material/Alert'
import Stack from '@suid/material/Stack'
import { For, createRoot, createSignal } from 'solid-js'

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
			sx={{
				position: 'fixed',
				zIndex: 99999,
				right: '1rem',
				top: '5rem',
				'@media (max-width: 840px)': {
					top: 'calc(56px + 12px + max(var(--sarca-safe-top), var(--sarca-android-top)))',
				},
				maxWidth: 360,
				width: '30vw',
				minWidth: 240,
			}}
			spacing={1}
		>
			<For each={alertList()}>
				{(alert) => (
					<Alert
						severity={alert.severity}
						onClose={() => dismissAlert(alert.id)}
					>
						{alert.msg}
					</Alert>
				)}
			</For>
		</Stack>
	)
}

export default AlertStack
