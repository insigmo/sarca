import { render, fireEvent, screen, waitFor } from '@solidjs/testing-library'
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { createSignal } from 'solid-js'

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
	// No cached grid thumb in these tests, so the viewer falls straight
	// through to the spinner-then-real-image path these tests assert on.
	getCachedThumb: vi.fn().mockResolvedValue(null),
	putCachedThumb: vi.fn().mockResolvedValue(undefined),
}))
vi.mock('./AlertStack', () => ({ alertStore: { addAlert: vi.fn() } }))

import FileViewer from './FileViewer'
import { DOUBLE_TAP_MS, DOUBLE_TAP_SCALE, ZOOM_BUTTON_FACTOR } from '../common/imageZoom'

const photo = (name) => ({ name, path: `/${name}`, is_file: true, size: 4096 })
const files = [photo('one.png'), photo('two.png'), photo('three.png')]

const mountViewer = async (props = {}) => {
	const onNavigate = vi.fn()
	const onClose = vi.fn()
	render(() => (
		<FileViewer
			open={true}
			file={props.file || files[1]}
			files={files}
			storageId="s1"
			onClose={onClose}
			onNavigate={onNavigate}
			resolvePreviewUrl={(path) => `preview:${path}`}
			resolveDownload={vi.fn()}
			{...props}
		/>
	))
	const image = await screen.findByAltText((props.file || files[1]).name)
	return { image, onNavigate, onClose }
}

const surface = () => document.querySelector('.file-viewer__zoom-surface')

/** jsdom has no PointerEvent; carry the fields the gesture model reads. */
const pointer = (type, { id = 1, x = 0, y = 0 } = {}) => {
	const e = new Event(type, { bubbles: true, cancelable: true })
	Object.assign(e, {
		pointerId: id,
		clientX: x,
		clientY: y,
		pointerType: 'touch',
		button: 0,
	})
	return e
}

const scaleOf = (image) => {
	const match = /scale\(([\d.]+)\)/.exec(image.getAttribute('style') || '')
	return match ? Number(match[1]) : null
}

const offsetOf = (image) => {
	const match = /translate\((-?[\d.]+)px, (-?[\d.]+)px\)/.exec(
		image.getAttribute('style') || '',
	)
	return match ? { x: Number(match[1]), y: Number(match[2]) } : null
}

/**
 * jsdom lays nothing out, so the zoom maths would silently run on a 0x0 stage
 * with a 0x0 photo. Paint an 800x600 stage holding a 400x600 portrait, which is
 * what letterboxing and pan bounds actually have to reason about.
 */
const giveLayout = (image) => {
	const rect = {
		left: 0,
		top: 0,
		right: 800,
		bottom: 600,
		width: 800,
		height: 600,
		x: 0,
		y: 0,
		toJSON: () => ({}),
	}
	image.getBoundingClientRect = () => rect
	surface().getBoundingClientRect = () => rect
	Object.defineProperty(image, 'naturalWidth', { value: 400, configurable: true })
	Object.defineProperty(image, 'naturalHeight', { value: 600, configurable: true })
	// The painted photo is 400x600 centred: x from 200 to 600.
	return { stage: rect, painted: { left: 200, right: 600 } }
}

describe('FileViewer photo zoom', () => {
	beforeEach(() => {
		vi.stubGlobal(
			'fetch',
			vi.fn().mockRejectedValue(new Error('offline in tests')),
		)
	})

	it('offers the magnifier controls for a photo', async () => {
		await mountViewer()
		expect(screen.getByLabelText('Zoom in')).toBeInTheDocument()
		expect(screen.getByLabelText('Zoom out')).toBeInTheDocument()
		expect(screen.getByLabelText('Reset zoom')).toHaveTextContent('100%')
	})

	it('has nothing to zoom out of at 100%', async () => {
		await mountViewer()
		// aria-disabled, not disabled: pressing Enter at the end of the range
		// must not throw the keyboard user's focus back to the document top.
		expect(screen.getByLabelText('Zoom out')).toHaveAttribute('aria-disabled', 'true')
		expect(screen.getByLabelText('Reset zoom')).toHaveAttribute('aria-disabled', 'true')
		expect(screen.getByLabelText('Zoom in')).toHaveAttribute('aria-disabled', 'false')
		expect(screen.getByLabelText('Zoom out')).not.toBeDisabled()
	})

	it('keeps focus on the button that ran out of range', async () => {
		const { image } = await mountViewer()
		const out = screen.getByLabelText('Zoom out')
		fireEvent.click(screen.getByLabelText('Zoom in'))
		await waitFor(() => expect(scaleOf(image)).toBeGreaterThan(1))

		out.focus()
		fireEvent.click(out)
		await waitFor(() => expect(scaleOf(image)).toBe(1))
		expect(document.activeElement).toBe(out)
	})

	it('scales the photo from the buttons and comes back on reset', async () => {
		const { image } = await mountViewer()

		fireEvent.click(screen.getByLabelText('Zoom in'))
		await waitFor(() => expect(scaleOf(image)).toBeCloseTo(ZOOM_BUTTON_FACTOR))
		expect(screen.getByLabelText('Reset zoom')).toHaveTextContent('150%')
		expect(screen.getByLabelText('Zoom out')).toHaveAttribute('aria-disabled', 'false')

		fireEvent.click(screen.getByLabelText('Zoom out'))
		await waitFor(() => expect(scaleOf(image)).toBeCloseTo(1))

		fireEvent.click(screen.getByLabelText('Zoom in'))
		fireEvent.click(screen.getByLabelText('Zoom in'))
		await waitFor(() => expect(scaleOf(image)).toBeGreaterThan(1))
		fireEvent.click(screen.getByLabelText('Reset zoom'))
		await waitFor(() => expect(scaleOf(image)).toBe(1))
	})

	it('zooms from the keyboard', async () => {
		const { image } = await mountViewer()

		fireEvent.keyDown(window, { key: '+' })
		await waitFor(() => expect(scaleOf(image)).toBeGreaterThan(1))
		fireEvent.keyDown(window, { key: '0' })
		await waitFor(() => expect(scaleOf(image)).toBe(1))
	})

	it('marks the viewer while zoomed so the phone can show the controls', async () => {
		await mountViewer()
		const viewer = document.querySelector('.file-viewer')
		expect(viewer.classList.contains('file-viewer--zoomed')).toBe(false)
		fireEvent.click(screen.getByLabelText('Zoom in'))
		await waitFor(() =>
			expect(viewer.classList.contains('file-viewer--zoomed')).toBe(true),
		)
	})

	it('double taps to zoom in and out again', async () => {
		const { image } = await mountViewer()
		const el = surface()
		const doubleTap = () => {
			for (let i = 0; i < 2; i++) {
				el.dispatchEvent(pointer('pointerdown', { x: 50, y: 40 }))
				el.dispatchEvent(pointer('pointerup', { x: 50, y: 40 }))
			}
		}

		doubleTap()
		await waitFor(() => expect(scaleOf(image)).toBe(DOUBLE_TAP_SCALE))
		doubleTap()
		await waitFor(() => expect(scaleOf(image)).toBe(1))
	})

	it('swipes to the next and previous photo', async () => {
		const { onNavigate } = await mountViewer()
		const el = surface()

		el.dispatchEvent(pointer('pointerdown', { x: 300, y: 200 }))
		el.dispatchEvent(pointer('pointermove', { x: 100, y: 205 }))
		el.dispatchEvent(pointer('pointerup', { x: 100, y: 205 }))
		expect(onNavigate).toHaveBeenCalledWith(files[2])

		el.dispatchEvent(pointer('pointerdown', { id: 2, x: 100, y: 200 }))
		el.dispatchEvent(pointer('pointermove', { id: 2, x: 300, y: 205 }))
		el.dispatchEvent(pointer('pointerup', { id: 2, x: 300, y: 205 }))
		expect(onNavigate).toHaveBeenLastCalledWith(files[0])
	})

	it('closes on a swipe down', async () => {
		const { onClose } = await mountViewer()
		const el = surface()

		el.dispatchEvent(pointer('pointerdown', { x: 200, y: 50 }))
		el.dispatchEvent(pointer('pointermove', { x: 205, y: 300 }))
		el.dispatchEvent(pointer('pointerup', { x: 205, y: 300 }))
		expect(onClose).toHaveBeenCalled()
	})

	it('a swipe on a zoomed photo pans instead of navigating', async () => {
		const { image, onNavigate } = await mountViewer()
		giveLayout(image)
		// 2.25x on a 400x600 photo in an 800x600 stage: 900 wide, so there are
		// 50px of overflow to pan into on each side.
		fireEvent.click(screen.getByLabelText('Zoom in'))
		fireEvent.click(screen.getByLabelText('Zoom in'))
		await waitFor(() => expect(scaleOf(image)).toBeCloseTo(2.25))

		const el = surface()
		el.dispatchEvent(pointer('pointerdown', { x: 300, y: 200 }))
		el.dispatchEvent(pointer('pointermove', { x: 100, y: 205 }))
		el.dispatchEvent(pointer('pointerup', { x: 100, y: 205 }))
		expect(onNavigate).not.toHaveBeenCalled()
		await waitFor(() => expect(offsetOf(image).x).toBe(-50))
	})

	it('tapping the empty bars beside a portrait photo closes the viewer', async () => {
		const { image, onClose } = await mountViewer()
		giveLayout(image)
		const el = surface()

		// On the photo: nothing happens.
		el.dispatchEvent(pointer('pointerdown', { x: 400, y: 300 }))
		el.dispatchEvent(pointer('pointerup', { x: 400, y: 300 }))
		await new Promise((r) => setTimeout(r, DOUBLE_TAP_MS + 40))
		expect(onClose).not.toHaveBeenCalled()

		// In the bar to the left of it: closes, like the backdrop.
		el.dispatchEvent(pointer('pointerdown', { x: 60, y: 300 }))
		el.dispatchEvent(pointer('pointerup', { x: 60, y: 300 }))
		await waitFor(() => expect(onClose).toHaveBeenCalled(), { timeout: 2000 })
	})

	it('double tapping the empty bars zooms instead of closing', async () => {
		const { image, onClose } = await mountViewer()
		giveLayout(image)
		const el = surface()

		for (let i = 0; i < 2; i++) {
			el.dispatchEvent(pointer('pointerdown', { x: 60, y: 300 }))
			el.dispatchEvent(pointer('pointerup', { x: 60, y: 300 }))
		}
		await waitFor(() => expect(scaleOf(image)).toBe(DOUBLE_TAP_SCALE))
		await new Promise((r) => setTimeout(r, DOUBLE_TAP_MS + 40))
		// The first half of the double tap must not have closed it underneath.
		expect(onClose).not.toHaveBeenCalled()
	})

	it('Escape unzooms first and only then closes', async () => {
		const { image, onClose } = await mountViewer()
		fireEvent.click(screen.getByLabelText('Zoom in'))
		await waitFor(() => expect(scaleOf(image)).toBeGreaterThan(1))

		fireEvent.keyDown(window, { key: 'Escape' })
		await waitFor(() => expect(scaleOf(image)).toBe(1))
		expect(onClose).not.toHaveBeenCalled()

		fireEvent.keyDown(window, { key: 'Escape' })
		expect(onClose).toHaveBeenCalled()
	})

	it('arrow keys walk the folder unzoomed and pan once zoomed', async () => {
		const { image, onNavigate } = await mountViewer()
		fireEvent.keyDown(window, { key: 'ArrowRight' })
		expect(onNavigate).toHaveBeenCalledWith(files[2])

		fireEvent.click(screen.getByLabelText('Zoom in'))
		await waitFor(() => expect(scaleOf(image)).toBeGreaterThan(1))
		onNavigate.mockClear()
		fireEvent.keyDown(window, { key: 'ArrowRight' })
		expect(onNavigate).not.toHaveBeenCalled()
	})

	it('starts every photo unzoomed', async () => {
		const [file, setFile] = createSignal(files[1])
		render(() => (
			<FileViewer
				open={true}
				file={file()}
				files={files}
				storageId="s1"
				onClose={vi.fn()}
				onNavigate={setFile}
				resolvePreviewUrl={(path) => `preview:${path}`}
				resolveDownload={vi.fn()}
			/>
		))
		await screen.findByAltText('two.png')
		fireEvent.click(screen.getByLabelText('Zoom in'))
		await waitFor(() =>
			expect(scaleOf(screen.getByAltText('two.png'))).toBeGreaterThan(1),
		)

		// Walking to the next photo must not inherit the last one's zoom. The
		// arrow keys pan while zoomed, so this is the button's job.
		fireEvent.click(screen.getByLabelText('Next file'))
		const next = await screen.findByAltText('three.png')
		await waitFor(() => expect(scaleOf(next)).toBe(1))
		expect(screen.getByLabelText('Reset zoom')).toHaveTextContent('100%')
	})

	it('leaves non-photos alone', async () => {
		render(() => (
			<FileViewer
				open={true}
				file={{ name: 'notes.txt', path: '/notes.txt', is_file: true, size: 12 }}
				files={[]}
				storageId="s1"
				onClose={vi.fn()}
				resolveDownload={vi.fn().mockResolvedValue(new Blob(['hello']))}
			/>
		))
		await waitFor(() =>
			expect(document.querySelector('.file-viewer')).toBeInTheDocument(),
		)
		expect(screen.queryByLabelText('Zoom in')).not.toBeInTheDocument()
		expect(document.querySelector('.file-viewer__zoom-surface')).toBeNull()
	})
})
