import Stack from '@suid/material/Stack'
import Button from '@suid/material/Button'
import AddIcon from '@suid/icons-material/Add'
import { For, Show, createSignal, onMount } from 'solid-js'
import { useNavigate } from '@solidjs/router'

import API from '../../api'
import { convertSize } from '../../common/size_converter'
import FileTypeIcon from '../../components/FileTypeIcon'
import WaveDivider from '../../components/WaveDivider'

const Storages = () => {
	/**
	 * @type {[import("solid-js").Accessor<import("../../api").StorageWithInfo[]>, any]}
	 */
	const [storages, setStorages] = createSignal([])
	const navigate = useNavigate()

	onMount(async () => {
		const storagesSchema = await API.storages.listStorages()
		setStorages(storagesSchema.storages)
	})

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
						No storages yet — create one in the UI (New storage), or set
						TELEGRAM_BOT_TOKEN, TELEGRAM_CHANNEL_ID, and STORAGE_NAME in
						sarca.conf for auto-setup.
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
									<FileTypeIcon name="docs.folder" isFile={false} size={48} />
									<div style={{ 'min-width': 0, flex: 1 }}>
										<h2 class="storage-card__title">{storage.name}</h2>
										<p class="storage-card__meta">
											{storage.files_amount}{' '}
											{storage.files_amount === 1 ? 'file' : 'files'}
											{' · '}
											{convertSize(storage.size)}
											{' · '}
											Chat {storage.chat_id}
										</p>
									</div>
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
