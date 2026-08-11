import Box from '@suid/material/Box'
import IconButton from '@suid/material/IconButton'
import InputAdornment from '@suid/material/InputAdornment'
import TextField from '@suid/material/TextField'
import { Show } from 'solid-js'

import FluentIcon from './FluentIcon'
import { filesChromeStore } from '../common/filesChrome'
import { t } from '../common/i18n'

/**
 * Search pill for the files toolbar.
 *
 * It used to live in the fixed app bar at the top of the window. The bar cost
 * 56-64px of vertical space on every screen to carry a wordmark and this one
 * control, so the bar is gone and the control moved next to the breadcrumb it
 * actually searches within. The `header-search` class stays: it is what the
 * styling and the GUI suite both key off.
 */
const FilesSearch = () => {
	const chrome = filesChromeStore

	return (
		<Show when={chrome.active()}>
			<Box class="search-pill header-search files-page__search">
				<TextField
					fullWidth
					size="small"
					placeholder={t('misc.search.placeholder')}
					value={chrome.searchQuery()}
					onChange={(e) => chrome.setSearchQuery(e.target.value)}
					onKeyDown={(e) => {
						if (e.key === 'Enter') chrome.runSearch()
					}}
					InputProps={{
						startAdornment: (
							<InputAdornment position="start">
								<FluentIcon name="search" size={18} />
							</InputAdornment>
						),
						endAdornment: (
							<InputAdornment position="end">
								<Show when={chrome.searchQuery() || chrome.isSearching()}>
									<IconButton size="small" onClick={chrome.clearSearch}>
										<FluentIcon name="dismiss" size={18} />
									</IconButton>
								</Show>
							</InputAdornment>
						),
					}}
				/>
			</Box>
		</Show>
	)
}

export default FilesSearch
