import Stack from '@suid/material/Stack'
import Button from '@suid/material/Button'
import IconButton from '@suid/material/IconButton'
import { For, Show, createEffect, createSignal, onCleanup, onMount } from 'solid-js'
import { useNavigate } from '@solidjs/router'

import { convertSize } from '../../common/size_converter'
import { storageSettingsStore } from '../../common/storageSettings'
import { storagesStore } from '../../common/storagesStore'
import { startAutoRefresh } from '../../common/autoRefresh'
import { t } from '../../common/i18n'
import FileTypeIcon from '../../components/FileTypeIcon'
import FilesSidebar from '../../components/FilesSidebar'
import FluentIcon from '../../components/FluentIcon'
import WaveDivider from '../../components/WaveDivider'

const Storages = () => {
	const [mobileNavOpen, setMobileNavOpen] = createSignal(false)
	const navigate = useNavigate()
	const { open: openStorageSettings } = storageSettingsStore
	const { storages, loaded, refreshStorages } = storagesStore

	// Whenever the list is loaded and empty - on first mount, or right after a
	// delete elsewhere in the app - send the user straight into the setup
	// wizard instead of showing a dead-end empty page.
	createEffect(() => {
		if (loaded() && !storages().length) {
			navigate('/setup', { replace: true })
		}
	})

	onMount(async () => {
		await refreshStorages()

		const mobileMediaQuery = window.matchMedia('(max-width: 840px)')
		const closeMobileNavOnDesktop = (event) => {
			if (!event.matches) setMobileNavOpen(false)
		}
		mobileMediaQuery.addEventListener('change', closeMobileNavOnDesktop)

		// Storage usage and channel health move without any action from this
		// page, so keep the cards current while it is on screen.
		const stopAutoRefresh = startAutoRefresh({
			run: () => refreshStorages(),
			// The settings modal edits the very storage the refresh would replace.
			isPaused: () => Boolean(storageSettingsStore.storage()),
		})

		onCleanup(() => {
			mobileMediaQuery.removeEventListener('change', closeMobileNavOnDesktop)
			stopAutoRefresh()
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
							aria-label={t('storages.openMenu')}
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
							{t('storages.newStorage')}
						</Button>
					</div>

					<WaveDivider style={{ margin: '0 0 8px', height: '36px' }} />

					<Show
						when={storages().length}
						fallback={
							<div class="storages-empty">
								{loaded() ? t('storages.redirectingToSetup') : t('storages.loading')}
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
										aria-label={t('storages.openStorage', { name: storage.name })}
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
															aria-label={t('storages.deadChannelFor', {
																name: storage.name,
															})}
															title={t('storages.deadChannel')}
														>
															<FluentIcon name="warning" size={18} />
														</span>
													</Show>
												</h2>
												<p class="storage-card__meta">
													{t('storages.cardMeta', {
														files: t(
															storage.files_amount === 1
																? 'storages.fileOne'
																: 'storages.fileMany',
															{ count: storage.files_amount },
														),
														size: convertSize(storage.size),
													})}
												</p>
											</div>
											<IconButton
												class="storage-card__settings"
												size="small"
												aria-label={t('storages.settingsFor', { name: storage.name })}
												title={t('storages.settingsHint')}
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
