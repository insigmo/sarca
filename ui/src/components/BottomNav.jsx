import HomeOutlinedIcon from '@suid/icons-material/HomeOutlined'
import SettingsOutlinedIcon from '@suid/icons-material/SettingsOutlined'
import { A, useLocation } from '@solidjs/router'

import { settingsStore } from '../common/settings'
import WaveDivider from './WaveDivider'

const BottomNav = () => {
	const location = useLocation()
	const { openSettings, isOpen } = settingsStore

	const path = () => location.pathname
	const onHome = () => {
		if (isOpen()) return false
		const p = path()
		return (
			p === '/' ||
			p === '/storages' ||
			p.startsWith('/storages/') ||
			p === '/setup' ||
			p.startsWith('/setup')
		)
	}
	const onSettings = () => isOpen()

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

			<button
				type="button"
				class="bottom-nav__item"
				classList={{ 'bottom-nav__item--active': onSettings() }}
				onClick={() => openSettings()}
				aria-label="Settings"
				aria-pressed={onSettings()}
			>
				<SettingsOutlinedIcon />
				Settings
			</button>
		</nav>
	)
}

export default BottomNav
