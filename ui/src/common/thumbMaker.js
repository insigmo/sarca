/**
 * Client-side grid thumbnails.
 *
 * The browser already holds the full-resolution photo while uploading it, so
 * the 128px tile is free here: no server-side decode, and no round trip to
 * fetch back a picture the client just sent.
 */

export const THUMB_MAX_EDGE = 128
const THUMB_QUALITY = 0.75
const IMAGE_EXT = /\.(jpe?g|png|gif|webp|bmp)$/i

/** @param {File|Blob} file */
export const canMakeThumb = (file) => {
	if (!file) return false
	if (typeof file.type === 'string' && file.type.startsWith('image/')) return true
	return IMAGE_EXT.test(file.name || '')
}

const fitWithin = (width, height, maxEdge) => {
	const longest = Math.max(width, height)
	if (!longest || longest <= maxEdge) return [width, height]
	const scale = maxEdge / longest
	return [Math.max(1, Math.round(width * scale)), Math.max(1, Math.round(height * scale))]
}

const drawToBlob = async (source, width, height) => {
	if (typeof OffscreenCanvas === 'function') {
		const canvas = new OffscreenCanvas(width, height)
		canvas.getContext('2d').drawImage(source, 0, 0, width, height)
		return await canvas.convertToBlob({ type: 'image/jpeg', quality: THUMB_QUALITY })
	}
	const canvas = document.createElement('canvas')
	canvas.width = width
	canvas.height = height
	canvas.getContext('2d').drawImage(source, 0, 0, width, height)
	return await new Promise((resolve) => {
		canvas.toBlob(resolve, 'image/jpeg', THUMB_QUALITY)
	})
}

/**
 * Downscale an image file to a JPEG thumbnail.
 *
 * Returns `null` whenever the browser cannot do it (no `createImageBitmap`, an
 * unreadable file, a codec it does not decode). The server keeps its own
 * generation path for exactly those cases, so a null here costs nothing.
 *
 * @param {File|Blob} file
 * @returns {Promise<Blob|null>}
 */
export const makeThumbBlob = async (file) => {
	if (!canMakeThumb(file) || typeof createImageBitmap !== 'function') return null
	let bitmap
	try {
		// `from-image` applies the EXIF rotation, so a phone portrait shot is not
		// stored sideways in the grid.
		bitmap = await createImageBitmap(file, { imageOrientation: 'from-image' })
		const [width, height] = fitWithin(bitmap.width, bitmap.height, THUMB_MAX_EDGE)
		return await drawToBlob(bitmap, width, height)
	} catch {
		return null
	} finally {
		bitmap?.close?.()
	}
}
