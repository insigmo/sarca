import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const cache = new Map()

vi.mock('./previewCache', () => ({
	getCachedPreview: vi.fn(async (scope, path) => cache.get(`${scope}:${path}`) || null),
	putCachedPreview: vi.fn(async (scope, path, blob) => {
		cache.set(`${scope}:${path}`, blob)
	}),
	getCachedThumb: vi.fn(async () => null),
	putCachedThumb: vi.fn(async () => {}),
}))

vi.mock('./nativeBridge', () => ({
	nativeInvoke: vi.fn(async () => null),
}))

vi.mock('./thumbQueue', () => ({
	enqueueThumbFetch: vi.fn((run) => run(new AbortController().signal)),
}))

const { loadPreview, loadThumb, resetPreviewLoader } = await import('./previewLoader')

/** A resolvable stand-in for a slow network fetch. */
const deferred = () => {
	let resolve
	let reject
	const promise = new Promise((res, rej) => {
		resolve = res
		reject = rej
	})
	return { promise, resolve, reject }
}

describe('loadPreview', () => {
	beforeEach(() => {
		cache.clear()
		resetPreviewLoader()
	})

	afterEach(() => {
		vi.unstubAllGlobals()
	})

	it('serves a second caller from the in-flight request instead of fetching twice', async () => {
		const gate = deferred()
		const fetchMock = vi.fn(async () => {
			await gate.promise
			return { ok: true, blob: async () => new Blob(['jpeg']) }
		})
		vi.stubGlobal('fetch', fetchMock)

		const opts = {
			scope: 's1',
			path: 'photos/a.jpg',
			resolveUrl: async () => 'https://api/preview?access_token=1',
			native: false,
		}
		const open = loadPreview(opts)
		const prefetch = loadPreview(opts)

		gate.resolve()
		const [a, b] = await Promise.all([open, prefetch])

		expect(fetchMock).toHaveBeenCalledTimes(1)
		expect(a).toBe(b)
	})

	it('reuses the cache on a later call rather than refetching', async () => {
		const fetchMock = vi.fn(async () => ({ ok: true, blob: async () => new Blob(['jpeg']) }))
		vi.stubGlobal('fetch', fetchMock)

		const opts = {
			scope: 's1',
			path: 'photos/a.jpg',
			resolveUrl: async () => 'https://api/preview',
			native: false,
		}
		await loadPreview(opts)
		await loadPreview(opts)

		expect(fetchMock).toHaveBeenCalledTimes(1)
	})

	it('still caches on the native client when the IPC bridge answers nothing', async () => {
		// Regression: the native client read and wrote *only* the on-disk cache
		// behind `nativeInvoke`, and every failure there is swallowed — so a
		// bridge that returns null left the viewer refetching the same photo
		// from the server on every open.
		const fetchMock = vi.fn(async () => ({ ok: true, blob: async () => new Blob(['jpeg']) }))
		vi.stubGlobal('fetch', fetchMock)

		const opts = {
			scope: 's1',
			path: 'photos/a.jpg',
			resolveUrl: async () => 'https://api/preview',
			native: true,
		}
		await loadPreview(opts)
		// Drop the in-process blob map so the second open has to come off a
		// persistent layer rather than out of memory.
		resetPreviewLoader()
		await loadPreview(opts)

		expect(fetchMock).toHaveBeenCalledTimes(1)
	})

	it('reopens a photo from memory without touching the persistent caches', async () => {
		const { getCachedPreview } = await import('./previewCache')
		const fetchMock = vi.fn(async () => ({ ok: true, blob: async () => new Blob(['jpeg']) }))
		vi.stubGlobal('fetch', fetchMock)

		const opts = {
			scope: 's1',
			path: 'photos/a.jpg',
			resolveUrl: async () => 'https://api/preview',
			native: true,
		}
		await loadPreview(opts)
		getCachedPreview.mockClear()
		const again = await loadPreview(opts)

		expect(again).toBeInstanceOf(Blob)
		expect(getCachedPreview).not.toHaveBeenCalled()
		expect(fetchMock).toHaveBeenCalledTimes(1)
	})

	it('hands back the preview without waiting on the native cache write', async () => {
		const { nativeInvoke } = await import('./nativeBridge')
		const stuck = deferred()
		nativeInvoke.mockImplementation(async (cmd) => {
			if (cmd === 'cache_put_preview') await stuck.promise
			return null
		})
		const fetchMock = vi.fn(async () => ({ ok: true, blob: async () => new Blob(['jpeg']) }))
		vi.stubGlobal('fetch', fetchMock)

		const blob = await loadPreview({
			scope: 's1',
			path: 'photos/a.jpg',
			resolveUrl: async () => 'https://api/preview',
			native: true,
		})

		expect(blob).toBeInstanceOf(Blob)
		stuck.resolve()
		nativeInvoke.mockImplementation(async () => null)
	})

	it('retries after a failure instead of negatively caching the path', async () => {
		let calls = 0
		const fetchMock = vi.fn(async () => {
			calls += 1
			if (calls === 1) throw new Error('offline')
			return { ok: true, blob: async () => new Blob(['jpeg']) }
		})
		vi.stubGlobal('fetch', fetchMock)

		const opts = {
			scope: 's1',
			path: 'photos/a.jpg',
			resolveUrl: async () => 'https://api/preview',
			native: false,
		}
		await expect(loadPreview(opts)).rejects.toThrow('offline')
		await expect(loadPreview(opts)).resolves.toBeInstanceOf(Blob)
		expect(fetchMock).toHaveBeenCalledTimes(2)
	})

	it('does not fetch a preview twice when two tiles request the same thumb', async () => {
		const gate = deferred()
		const fetchBlob = vi.fn(async () => {
			await gate.promise
			return new Blob(['thumb'])
		})
		const opts = { scope: 's1', path: 'photos/a.jpg', fetchBlob }

		const first = loadThumb(opts)
		const second = loadThumb(opts)
		gate.resolve()
		await Promise.all([first, second])

		expect(fetchBlob).toHaveBeenCalledTimes(1)
	})

	it('lets one tile abort without cancelling the shared thumb download', async () => {
		const gate = deferred()
		const fetchBlob = vi.fn(async () => {
			await gate.promise
			return new Blob(['thumb'])
		})
		const ac = new AbortController()
		const opts = { scope: 's1', path: 'photos/a.jpg', fetchBlob }

		const aborted = loadThumb({ ...opts, signal: ac.signal })
		const kept = loadThumb(opts)
		ac.abort()

		await expect(aborted).rejects.toThrow()
		gate.resolve()
		await expect(kept).resolves.toBeInstanceOf(Blob)
		expect(fetchBlob).toHaveBeenCalledTimes(1)
	})
})
