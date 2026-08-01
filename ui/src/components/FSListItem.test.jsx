import { render, fireEvent, screen } from '@solidjs/testing-library'
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

vi.mock('@solidjs/router', () => ({
	useNavigate: () => () => {},
	useParams: () => ({ id: 'storage1' }),
}))

vi.mock('../api', () => ({
	default: {
		files: {
			thumb: vi.fn(),
			deleteFile: vi.fn().mockResolvedValue(undefined),
			download: vi.fn(),
		},
	},
}))

vi.mock('../common/thumbQueue', () => ({
	enqueueThumbFetch: vi.fn(),
}))

vi.mock('./AlertStack', () => ({
	alertStore: { addAlert: vi.fn() },
}))

import FSListItem from './FSListItem'

const fsElement = {
	name: 'photo.png',
	path: '/photo.png',
	is_file: true,
	has_thumb: false,
	size: 1000,
}

const longPress = async (el) => {
	const touchStart = new Event('touchstart', { bubbles: true, cancelable: true })
	touchStart.touches = [{ clientX: 10, clientY: 10 }]
	fireEvent(el, touchStart)
	// 520ms long-press timer + time for the Menu's Grow transition to mount.
	await vi.advanceTimersByTimeAsync(1500)
}

describe('FSListItem', () => {
	beforeEach(() => {
		vi.useFakeTimers()
	})

	afterEach(() => {
		vi.useRealTimers()
	})

	// Regression: suppressClickAfterLongPress was set true when a long-press
	// opened the context menu, but only ever reset inside handleItemClick.
	// Dismissing the menu any other way (Escape, backdrop click, an action
	// that itself doesn't route through handleItemClick) left the flag stuck
	// true, so the *next* tap on that tile was silently swallowed — matches
	// the user's report of delete/tap "not always working" in the photo
	// (grid/tiles) folder view after a long-press. Fixed by resetting the
	// flag in handleCloseMore, so any dismissal path clears it.
	it('does not swallow the next tap after a long-press menu is dismissed via Escape', async () => {
		const onOpen = vi.fn()
		render(() => (
			<FSListItem fsElement={fsElement} storageId="storage1" onDelete={vi.fn()} onOpen={onOpen} />
		))

		const tile = document.querySelector('[data-fs-path]')
		await longPress(tile)

		// Context menu opened (portaled outside the render container).
		const menuItem = screen.queryByText('Rename')
		expect(menuItem).toBeInTheDocument()

		fireEvent.keyDown(menuItem, { key: 'Escape', code: 'Escape' })
		expect(screen.queryByText('Rename')).not.toBeInTheDocument()

		fireEvent.click(tile)
		expect(onOpen).toHaveBeenCalledWith(fsElement)
	})

	it('opens the file on a normal tap (no prior long-press)', () => {
		const onOpen = vi.fn()
		render(() => (
			<FSListItem fsElement={fsElement} storageId="storage1" onDelete={vi.fn()} onOpen={onOpen} />
		))

		const tile = document.querySelector('[data-fs-path]')
		fireEvent.click(tile)
		expect(onOpen).toHaveBeenCalledWith(fsElement)
	})
})
