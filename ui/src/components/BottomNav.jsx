import HomeOutlinedIcon from '@suid/icons-material/HomeOutlined'
import StorageOutlinedIcon from '@suid/icons-material/StorageOutlined'
import SettingsOutlinedIcon from '@suid/icons-material/SettingsOutlined'
import { A, useLocation } from '@solidjs/router'

import { settingsStore } from '../common/settings'
import WaveDivider from './WaveDivider'

const BottomNav = () => {
	const location = useLocation()
	const { openSettings } = settingsStore

	const path = () => location.pathname
	const onHome = () => path() === '/' || path() === '/storages'
	const onStorages = () => path().startsWith('/storages/')

	return (
		<nav class="bottom-nav" aria-label="Mobile navigation">
			<div class="bottom-nav__wave" aria-hidden="true">
				<WaveDivider flip />
			</div>

			<A
				href="/storages"
				class="bottom-nav__item"
				classList={{ 'bottom-nav__item--active': onHome() }}
			>
				<HomeOutlinedIcon />
				Home
			</A>

			<A
				href="/storages"
				class="bottom-nav__item"
				classList={{
					'bottom-nav__item--active': onStorages(),
				}}
			>
				<StorageOutlinedIcon />
				Storages
			</A>

			<button
				type="button"
				class="bottom-nav__item"
				onClick={() => openSettings()}
				aria-label="Settings"
			>
				<SettingsOutlinedIcon />
				Settings
			</button>
		</nav>
	)
}

export default BottomNav
