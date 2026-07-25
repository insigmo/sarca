import Stack from '@suid/material/Stack'
import Button from '@suid/material/Button'
import IconButton from '@suid/material/IconButton'
import { For, Show, createSignal, onCleanup, onMount } from 'solid-js'
import { useNavigate } from '@solidjs/router'

import API from '../../api'
import { convertSize } from '../../common/size_converter'
import { storageSettingsStore } from '../../common/storageSettings'
import FileTypeIcon from '../../components/FileTypeIcon'
import FilesSidebar from '../../components/FilesSidebar'
import FluentIcon from '../../components/FluentIcon'
import WaveDivider from '../../components/WaveDivider'

const Storages = () => {
	/**
	 * @type {[import("solid-js").Accessor<import("../../api").StorageWithInfo[]>, any]}
	 */
	const [storages, setStorages] = createSignal([])
	const [mobileNavOpen, setMobileNavOpen] = createSignal(false)
	const navigate = useNavigate()
	const { open: openStorageSettings } = storageSettingsStore

	onMount(async () => {
		const storagesSchema = await API.storages.listStorages()
		setStorages(storagesSchema.storages)
		if (!storagesSchema.storages.length) {
			navigate('/setup', { replace: true })
		}

		const mobileMediaQuery = window.matchMedia('(max-width: 840px)')
		const closeMobileNavOnDesktop = (event) => {
			if (!event.matches) setMobileNavOpen(false)
		}
		mobileMediaQuery.addEventListener('change', closeMobileNavOnDesktop)
		onCleanup(() => {
			mobileMediaQuery.removeEventListener('change', closeMobileNavOnDesktop)
		})
	})

	const openSettings = (e, storage) => {
		e.stopPropagation()
		e.preventDefault()
		openStorageSettings(storage)
	}

	return (
		<div class="files-shell">
			<FilesSidebar
				variant="storages"
				mobileOpen={mobileNavOpen()}
				onMobileClose={() => setMobileNavOpen(false)}
			/>
			<div class="files-shell__main">
				<Stack>
					<div class="page-header" style={{ 'justify-content': 'flex-end' }}>
						<IconButton
							class="files-page__nav-toggle"
							aria-label="Open menu"
							onClick={() => setMobileNavOpen(true)}
							sx={{ mr: 'auto' }}
						>
							<FluentIcon name="navigation" size={22} />
						</IconButton>
						<Button
							onClick={() => navigate('/storages/register')}
							variant="contained"
							color="secondary"
							startIcon={<FluentIcon name="add" size={18} />}
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
											<FileTypeIcon name="storage" isFile={false} storage size={56} />
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
															<FluentIcon name="warning" size={18} />
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
												<FluentIcon name="settings" size={18} />
											</IconButton>
										</div>
									</article>
								)}
							</For>
						</div>
					</Show>
				</Stack>
			</div>
		</div>
	)
}

export default Storages
