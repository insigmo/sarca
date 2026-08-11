import { For, Show, createEffect, createSignal, onCleanup, onMount } from 'solid-js'
import Button from '@suid/material/Button'
import Typography from '@suid/material/Typography'
import AccessTimeIcon from '@suid/icons-material/AccessTime'
import CheckIcon from '@suid/icons-material/Check'
import LoadingDots from './LoadingDots'

import { formatBytes, isMobileNativePlatform } from '../common/nativeBridge'
import { syncSettingsStore } from '../common/syncSettingsStore'
import { filesChromeStore } from '../common/filesChrome'
import { t } from '../common/i18n'
import SettingsSwitch from './SettingsSwitch'

/**
 * Sync tab: Camera media auto-upload. Storage is locked to the currently
 * open Files storage. All state and IPC live in {@link syncSettingsStore} so
 * closing and reopening Settings never restarts a cold load.
 * @param {{ storageId?: string, storageName?: string }} props
 */
const SettingsSyncPanel = (props) => {
	const chrome = filesChromeStore
	const sync = syncSettingsStore
	const [platform, setPlatform] = createSignal(sync.platform())

	createEffect(() => setPlatform(sync.platform()))
	createEffect(() => {
		sync.setStorageId(props.storageId || chrome.storageId() || '')
	})

	const isMobile = () => isMobileNativePlatform(platform())

	onMount(() => {
		sync.start()
		onCleanup(() => sync.stop())
	})

	const statusIcon = (status) => {
		if (status === 'active') {
			// A transfer list can hold many "active" rows at once; a spinner per
			// row means that many continuously-repainting animations at all
			// times, which is costly under WebKitGTK. Animated dot text is not.
			return (
				<span style={{ 'font-size': '16px', 'line-height': 1, color: 'var(--sarca-teal)' }}>
					<LoadingDots />
				</span>
			)
		}
		if (status === 'waiting') {
			return <AccessTimeIcon sx={{ fontSize: 16, color: 'text.secondary' }} />
		}
		return <CheckIcon sx={{ fontSize: 16, color: 'success.main' }} />
	}

	return (
		<div class="settings-sync-panel">
			<div class="settings-toggle">
				<span>{t('settings.enableAutoUpload')}</span>
				{/*
				  * Always interactive, never disabled while the native work runs:
				  * the switch renders the user's intent immediately and the store
				  * reverts it only if the native call actually fails.
				  */}
				<SettingsSwitch
					id="settings-camera-switch"
					checked={sync.cameraOn()}
					onChange={(checked) => sync.setAutoUpload(checked)}
				/>
			</div>

			<Show when={sync.autoBinding()}>
				<p class="settings-sync-panel__meta">
					{sync.autoBinding().local_path} →{' '}
					{sync.autoBinding().remote_root || t('settings.cameraFolderName')}
				</p>
				<div class="settings-sync-panel__row">
					<Button
						variant="outlined"
						size="small"
						disabled={sync.busy()}
						onClick={async () => {
							const path = await sync.pickFolder(sync.localPath())
							if (!path) return
							await sync.changeLocalFolder(path)
						}}
					>
						{t('settings.changeLocalFolder')}
					</Button>
				</div>
			</Show>

			<Show when={sync.autoBinding() && isMobile()}>
				<div class="settings-toggle">
					<span>{t('settings.uploadOnWifiOnly')}</span>
					<SettingsSwitch
						id="settings-wifi-switch"
						checked={sync.prefs().wifi_only !== false}
						onChange={(checked) =>
							sync.savePrefs({ ...sync.prefs(), wifi_only: checked })
						}
					/>
				</div>
			</Show>

			<div class="settings-sync-panel__row">
				<Button
					variant="contained"
					color="secondary"
					onClick={() => sync.runSyncNow()}
				>
					{t('settings.uploadNow')}
				</Button>
			</div>

			{/* Inline, right under the buttons: the queue is the answer to "is it
			    working?", so it must not be hidden behind another tap. */}
			<div class="settings-sync-panel__section">
				<Typography variant="subtitle2" sx={{ mb: 1 }}>
					{t('settings.uploading')}
					<span class="settings-sync-panel__queue-count">
						{sync.transferSnap().uploading}
					</span>
				</Typography>
				<Show
					when={sync.uploadItems().length}
					fallback={<p class="settings-account__hint">{t('settings.noTransfersYet')}</p>}
				>
					<ul class="settings-sync-panel__transfer-list">
						<For each={sync.uploadItems()}>
							{(item) => (
								<li class="settings-sync-panel__transfer-item">
									<div class="settings-sync-panel__transfer-meta">
										<Show when={item.path}>
											<div class="settings-account__hint">{item.path}/</div>
										</Show>
										<div class="settings-sync-panel__transfer-name">
											{item.name}
										</div>
										<Show when={item.size != null}>
											<div class="settings-account__hint">
												{formatBytes(Number(item.size) || 0)}
											</div>
										</Show>
									</div>
									<div
										class="settings-sync-panel__transfer-status"
										aria-label={item.status}
									>
										{statusIcon(item.status)}
									</div>
								</li>
							)}
						</For>
					</ul>
				</Show>
			</div>

			<Show when={sync.scanHint()}>
				<p class="settings-account__hint">{sync.scanHint()}</p>
			</Show>

			<Show when={sync.statuses().some((s) => s.last_error)}>
				<p class="settings-bot-hint" role="alert">
					{sync
						.statuses()
						.filter((s) => s.last_error)
						.map((s) => s.last_error)
						.join(' · ')}
				</p>
			</Show>
			<Show when={sync.msg()}>
				<p class="settings-bot-hint" role="status">
					{sync.msg()}
				</p>
			</Show>
		</div>
	)
}

export default SettingsSyncPanel
