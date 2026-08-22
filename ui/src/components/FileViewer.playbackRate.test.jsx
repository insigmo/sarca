import { render, fireEvent, screen } from '@solidjs/testing-library'
import { describe, it, expect, vi } from 'vitest'

vi.mock('../api', () => ({
	default: {
		files: {
			getPreviewUrl: vi.fn().mockResolvedValue('/preview.jpg'),
			getInlineMediaUrl: vi.fn().mockResolvedValue('/inline'),
			download: vi.fn(),
			recordRecent: vi.fn(),
			thumb: vi.fn().mockRejectedValue(new Error('no thumb in test')),
		},
	},
}))

vi.mock('../common/nativeBridge', () => ({ nativeInvoke: vi.fn() }))
vi.mock('../common/nativeClient', () => ({
	nativeClientStore: { isNative: () => false },
}))
vi.mock('../common/previewCache', () => ({
	getCachedPreview: vi.fn().mockResolvedValue(null),
	putCachedPreview: vi.fn().mockResolvedValue(undefined),
	getCachedThumb: vi.fn().mockResolvedValue(null),
	putCachedThumb: vi.fn().mockResolvedValue(undefined),
}))
vi.mock('./AlertStack', () => ({ alertStore: { addAlert: vi.fn() } }))

import FileViewer from './FileViewer'

const video = (name) => ({
	name,
	path: `/${name}`,
	is_file: true,
	size: 1_000_000,
})

describe('FileViewer video playback speed', () => {
	it('shows a playback-rate control in the video chrome', async () => {
		const files = [video('clip.mp4')]
		render(() => (
			<FileViewer
				open={true}
				file={files[0]}
				files={files}
				storageId="s1"
				onClose={vi.fn()}
				onNavigate={vi.fn()}
				resolvePreviewUrl={(p) => `preview:${p}`}
				resolveDownload={vi.fn()}
			/>
		))
		await screen.findByAltText(/streaming|clip/i).catch(() => {})
		const rate = await vi.waitFor(() => {
			const el = document.querySelector('.file-viewer__ctrl-btn--rate')
			if (!el) throw new Error('rate button not mounted yet')
			return el
		})
		expect(rate.textContent.trim()).toBe('1×')
	})

	it('cycles the rate on click and applies it to the media element', async () => {
		const files = [video('clip.mp4')]
		render(() => (
			<FileViewer
				open={true}
				file={files[0]}
				files={files}
				storageId="s1"
				onClose={vi.fn()}
				onNavigate={vi.fn()}
				resolvePreviewUrl={(p) => `preview:${p}`}
				resolveDownload={vi.fn()}
			/>
		))
		const mediaEl = await vi.waitFor(() => {
			const el =
				document.querySelector('.file-viewer__video') ||
				document.querySelector('video')
			if (!el) throw new Error('video element not mounted yet')
			return el
		})
		const rate = await vi.waitFor(() => {
			const el = document.querySelector('.file-viewer__ctrl-btn--rate')
			if (!el) throw new Error('rate button not mounted yet')
			return el
		})

		expect(mediaEl.playbackRate).toBe(1)
		fireEvent.click(rate)
		expect(mediaEl.playbackRate).toBe(1.25)
		expect(rate.textContent.trim()).toBe('1.25×')
		fireEvent.click(rate)
		expect(mediaEl.playbackRate).toBe(1.5)
		fireEvent.click(rate)
		expect(mediaEl.playbackRate).toBe(2)
		fireEvent.click(rate)
		expect(mediaEl.playbackRate).toBe(0.5)
		fireEvent.click(rate)
		expect(mediaEl.playbackRate).toBe(0.75)
		// …and back to 1x after the last step of the cycle.
		fireEvent.click(rate)
		expect(mediaEl.playbackRate).toBe(1)
	})
})
