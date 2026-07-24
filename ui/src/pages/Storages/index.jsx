import Stack from '@suid/material/Stack'
import Button from '@suid/material/Button'
import IconButton from '@suid/material/IconButton'
import AddIcon from '@suid/icons-material/Add'
import SettingsOutlinedIcon from '@suid/icons-material/SettingsOutlined'
import WarningAmberIcon from '@suid/icons-material/WarningAmber'
import { For, Show, createSignal, onMount } from 'solid-js'
import { useNavigate } from '@solidjs/router'

import API from '../../api'
import { convertSize } from '../../common/size_converter'
import { storageSettingsStore } from '../../common/storageSettings'
import FileTypeIcon from '../../components/FileTypeIcon'
import WaveDivider from '../../components/WaveDivider'

const Storages = () => {
	/**
	 * @type {[import("solid-js").Accessor<import("../../api").StorageWithInfo[]>, any]}
	 */
	const [storages, setStorages] = createSignal([])
	const navigate = useNavigate()
	const { open: openStorageSettings } = storageSettingsStore

	onMount(async () => {
		const storagesSchema = await API.storages.listStorages()
		setStorages(storagesSchema.storages)
		if (!storagesSchema.storages.length) {
			navigate('/setup', { replace: true })
		}
	})

	const openSettings = (e, storage) => {
		e.stopPropagation()
		e.preventDefault()
		openStorageSettings(storage)
	}

	return (
		<Stack>
			<div class="page-header" style={{ 'justify-content': 'flex-end' }}>
				<Button
					onClick={() => navigate('/storages/register')}
					variant="contained"
					color="secondary"
					startIcon={<AddIcon />}
				>
					New storage
				</Button>
			</div>

			<WaveDivider style={{ margin: '0 0 8px', height: '36px' }} />

			<Show
				when={storages().length}
				fallback={
					<div class="storages-empty">
						No storages yet — the setup wizard will guide you, or set
						TELEGRAM_BOT_TOKEN, TELEGRAM_CHANNEL_ID, and STORAGE_NAME in
						sarca.conf.
					</div>
				}
			>
				<div class="storages-grid">
					<For each={storages()}>
						{(storage, index) => (
							<article
								class="storage-card"
								style={{ 'animation-delay': `${index() * 60}ms` }}
								onClick={() => navigate(`/storages/${storage.id}/files`)}
								onKeyDown={(e) => {
									if (e.key === 'Enter' || e.key === ' ') {
										e.preventDefault()
										navigate(`/storages/${storage.id}/files`)
									}
								}}
								tabIndex={0}
								role="button"
								aria-label={`Open storage ${storage.name}`}
							>
								<div class="storage-card__top">
									<FileTypeIcon name="docs.folder" isFile={false} size={56} />
									<div style={{ 'min-width': 0, flex: 1 }}>
										<h2 class="storage-card__title">
											{storage.name}
											<Show when={storage.has_dead_channel}>
												<span
													class="storage-card__warning"
													role="img"
													aria-label={`${storage.name} has a deleted channel — open settings to fix`}
													title="A channel was deleted — open settings to fix"
												>
													<WarningAmberIcon fontSize="small" />
												</span>
											</Show>
										</h2>
										<p class="storage-card__meta">
											{storage.files_amount}{' '}
											{storage.files_amount === 1 ? 'file' : 'files'}
											{' · '}
											{convertSize(storage.size)}
										</p>
									</div>
									<IconButton
										class="storage-card__settings"
										size="small"
										aria-label={`Settings for ${storage.name}`}
										title="Bot, channels, rename…"
										onClick={(e) => openSettings(e, storage)}
										onMouseDown={(e) => e.stopPropagation()}
										onKeyDown={(e) => e.stopPropagation()}
									>
										<SettingsOutlinedIcon fontSize="small" />
									</IconButton>
								</div>
							</article>
						)}
					</For>
				</div>
			</Show>
		</Stack>
	)
}

export default Storages
