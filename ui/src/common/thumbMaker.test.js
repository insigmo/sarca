import { describe, expect, it, vi, afterEach } from 'vitest'

import { canMakeThumb, makeThumbBlob, THUMB_MAX_EDGE } from './thumbMaker'

const originalCreateImageBitmap = globalThis.createImageBitmap
const originalOffscreen = globalThis.OffscreenCanvas

afterEach(() => {
	globalThis.createImageBitmap = originalCreateImageBitmap
	globalThis.OffscreenCanvas = originalOffscreen
	vi.restoreAllMocks()
})

/** Minimal OffscreenCanvas stand-in that records the size it was drawn at. */
const stubOffscreen = (drawn) => {
	globalThis.OffscreenCanvas = class {
		constructor(width, height) {
			drawn.width = width
			drawn.height = height
		}
		getContext() {
			return { drawImage: () => {} }
		}
		convertToBlob(opts) {
			return Promise.resolve(new Blob(['jpeg'], { type: opts.type }))
		}
	}
}

describe('canMakeThumb', () => {
	it('accepts images by mime type and by extension', () => {
		expect(canMakeThumb(new File([''], 'a.bin', { type: 'image/png' }))).toBe(true)
		expect(canMakeThumb(new File([''], 'a.JPEG'))).toBe(true)
	})

	it('rejects non-images', () => {
		expect(canMakeThumb(new File([''], 'clip.mp4', { type: 'video/mp4' }))).toBe(false)
		expect(canMakeThumb(null)).toBe(false)
	})
})

describe('makeThumbBlob', () => {
	it('fits the long edge to THUMB_MAX_EDGE and keeps the aspect ratio', async () => {
		const drawn = {}
		stubOffscreen(drawn)
		globalThis.createImageBitmap = vi
			.fn()
			.mockResolvedValue({ width: 4000, height: 3000, close: () => {} })

		const blob = await makeThumbBlob(new File([''], 'photo.jpg', { type: 'image/jpeg' }))

		expect(blob.type).toBe('image/jpeg')
		expect(drawn.width).toBe(THUMB_MAX_EDGE)
		expect(drawn.height).toBe((THUMB_MAX_EDGE * 3) / 4)
	})

	it('does not upscale an already small image', async () => {
		const drawn = {}
		stubOffscreen(drawn)
		globalThis.createImageBitmap = vi
			.fn()
			.mockResolvedValue({ width: 40, height: 20, close: () => {} })

		await makeThumbBlob(new File([''], 'tiny.png', { type: 'image/png' }))

		expect(drawn).toEqual({ width: 40, height: 20 })
	})

	it('returns null when the browser cannot decode the file', async () => {
		stubOffscreen({})
		globalThis.createImageBitmap = vi.fn().mockRejectedValue(new Error('unsupported'))

		expect(await makeThumbBlob(new File([''], 'weird.webp', { type: 'image/webp' }))).toBe(
			null,
		)
	})

	it('returns null for non-images without touching the decoder', async () => {
		const decode = vi.fn()
		globalThis.createImageBitmap = decode

		expect(await makeThumbBlob(new File([''], 'doc.pdf', { type: 'application/pdf' }))).toBe(
			null,
		)
		expect(decode).not.toHaveBeenCalled()
	})
})
