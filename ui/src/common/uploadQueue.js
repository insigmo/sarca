import { createEffect, createRoot, createSignal, onCleanup } from 'solid-js'
import { t } from './i18n'
import API from '../api'
import { alertStore } from '../components/AlertStack'

/**
 * Two-stage pipeline concurrency:
 * - At most one file spooling (client→Sarca, before `phase: spooled`)
 * - At most one file past spool (Sarca→Telegram NDJSON still open)
 * Storage Manager serializes Telegram; overlapping spool+telegram is the win.
 */
const MAX_SPOOL_CONCURRENT = 1
/** Max HTTP uploads in flight (1 spooling + 1 telegramming). */
const MAX_CONCURRENT = 2

/** Soft cap for the normal file picker (not folder / drag-drop). */
export const MAX_FILE_PICKER = 10

/**
 * @typedef {'queued' | 'uploading' | 'done' | 'canceled' | 'error'} UploadItemStatus
 */

/**
 * @typedef {Object} UploadQueueItem
 * @property {string} id
 * @property {string} name
 * @property {UploadItemStatus} status
 * @property {number} progress
 * @property {boolean} indeterminate - True during client→Sarca / waiting-for-Telegram; false once Telegram progress starts.
 * @property {boolean} spooled - True after server `phase: spooled` (or first telegram event); frees the spool slot.
 * @property {number} [bytesUploaded]
 * @property {number} [bytesTotal]
 * @property {number} [retryAfter] - Seconds Telegram asked us to wait (flood); set while waiting.
 * @property {string} storageId
 * @property {string} parentPath
 * @property {File} file
 * @property {string} [error]
 */

let uploadSeq = 0

/** @type {Map<string, AbortController>} */
const abortControllers = new Map()

/** Humanize upload errors (disk full, etc.). Rate limits are retried server-side. */
const formatUploadError = (err) => {
	const msg = String(err?.message || err || t('upload.failed'))
	if (/disk full|no space|INSUFFICIENT_STORAGE/i.test(msg)) {
		return t('upload.diskFull')
	}
	return msg
}

/** True AbortError from XHR/AbortSignal — not messages that merely contain “abort”. */
const isAbortError = (err) => err?.name === 'AbortError'

/**
 * @param {UploadQueueItem[]} list
 * @returns {boolean}
 */
const canStartSpool = (list) => {
	const uploading = list.filter((i) => i.status === 'uploading')
	if (uploading.length >= MAX_CONCURRENT) return false
	const spooling = uploading.filter((i) => !i.spooled).length
	return spooling < MAX_SPOOL_CONCURRENT
}

/**
 * Global upload queue (Drive-style mini window + concurrency).
 */
export const uploadQueueStore = createRoot(() => {
	/**
	 * @type {[import('solid-js').Accessor<UploadQueueItem[]>, import('solid-js').Setter<UploadQueueItem[]>]}
	 */
	const [items, setItems] = createSignal([])
	const [visible, setVisible] = createSignal(false)
	const [collapsed, setCollapsed] = createSignal(false)
	/** True after Cancel until the panel is dismissed or a new batch starts. */
	const [canceledBatch, setCanceledBatch] = createSignal(false)
	/** Epoch ms when the current active batch began transferring bytes. */
	const [transferStartedAt, setTransferStartedAt] = createSignal(0)

	/** @type {Set<() => void>} */
	const idleListeners = new Set()
	/** @type {Set<(item: UploadQueueItem) => void>} */
	const itemDoneListeners = new Set()

	const hasActiveWork = () =>
		items().some((i) => i.status === 'queued' || i.status === 'uploading')

	/** Native “leave site?” dialog while uploads are queued or in flight. */
	const onBeforeUnload = (e) => {
		if (!hasActiveWork()) return
		e.preventDefault()
		e.returnValue = ''
	}

	createEffect(() => {
		if (!hasActiveWork()) return
		window.addEventListener('beforeunload', onBeforeUnload)
		onCleanup(() => {
			window.removeEventListener('beforeunload', onBeforeUnload)
		})
	})

	const notifyIdle = () => {
		if (hasActiveWork()) return
		for (const fn of idleListeners) {
			try {
				fn()
			} catch (e) {
				console.error(e)
			}
		}
	}

	/**
	 * Fired after a single upload’s API promise resolves and the item is marked
	 * `done` (server bookkeeping finished). Not gated on the rest of the queue.
	 * @param {UploadQueueItem} item
	 */
	const notifyItemDone = (item) => {
		for (const fn of itemDoneListeners) {
			try {
				fn(item)
			} catch (e) {
				console.error(e)
			}
		}
	}

	/**
	 * @param {() => void} fn
	 * @returns {() => void} unsubscribe
	 */
	const onIdle = (fn) => {
		idleListeners.add(fn)
		return () => idleListeners.delete(fn)
	}

	/**
	 * @param {(item: UploadQueueItem) => void} fn
	 * @returns {() => void} unsubscribe
	 */
	const onItemDone = (fn) => {
		itemDoneListeners.add(fn)
		return () => itemDoneListeners.delete(fn)
	}

	/**
	 * @param {string} id
	 * @param {Partial<UploadQueueItem>} patch
	 */
	const patchItem = (id, patch) => {
		setItems((list) =>
			list.map((it) => (it.id === id ? { ...it, ...patch } : it)),
		)
	}

	/**
	 * Mark spool complete and try to start the next client→Sarca upload.
	 * @param {string} id
	 * @param {Partial<UploadQueueItem>} [extra]
	 */
	const markSpooled = (id, extra = {}) => {
		const cur = items().find((i) => i.id === id)
		if (!cur || cur.status !== 'uploading') return
		if (cur.spooled && !Object.keys(extra).length) return
		patchItem(id, { spooled: true, ...extra })
		// Defer pump so Solid's signal write is visible and we never nest
		// runUpload→setItems inside another progress callback stack.
		queueMicrotask(() => pump())
	}

	/**
	 * @param {string} id
	 */
	const runUpload = async (id) => {
		let file = /** @type {File | null} */ (null)
		let storageId = ''
		let parentPath = ''

		setItems((list) => {
			if (!canStartSpool(list)) return list
			const item = list.find((i) => i.id === id)
			if (!item || item.status !== 'queued') return list
			file = item.file
			storageId = item.storageId
			parentPath = item.parentPath
			return list.map((i) =>
				i.id === id
					? {
							...i,
							status: /** @type {UploadItemStatus} */ ('uploading'),
							progress: 0,
							indeterminate: true,
							spooled: false,
							bytesUploaded: 0,
							bytesTotal: Number(item.file?.size) || 0,
						}
					: i,
			)
		})

		if (!file) return

		if (!transferStartedAt()) {
			setTransferStartedAt(Date.now())
		}

		const ac = new AbortController()
		abortControllers.set(id, ac)

		try {
			await API.files.uploadFile(
				storageId,
				parentPath,
				file,
				(ev) => {
					const cur = items().find((i) => i.id === id)
					if (!cur || cur.status !== 'uploading') return
					const total =
						ev.total != null
							? Number(ev.total)
							: Number(file.size) || cur.bytesTotal || 0
					const uploaded =
						ev.uploaded != null
							? Number(ev.uploaded)
							: total > 0
								? ((Number(ev.percent) || 0) / 100) * total
								: 0

					// Spool+DB ready: free spool slot so the next file can start
					// client→Sarca while this connection stays open for Telegram.
					if (ev.phase === 'spooled') {
						markSpooled(id, {
							indeterminate: true,
							retryAfter: undefined,
						})
						return
					}

					// Phase 1 (client→Sarca): keep spinning ring; track bytes for ETA only.
					// (fetch uploads no longer emit server % — spinner stays until telegram.)
					if (ev.phase === 'server') {
						patchItem(id, {
							indeterminate: true,
							bytesUploaded: uploaded,
							bytesTotal: total,
							retryAfter: undefined,
						})
						return
					}

					// Flood wait: keep determinate progress, surface retry_after for ETA.
					if (ev.phase === 'waiting') {
						const pct = Math.round(Number(ev.percent) || 0)
						const retryAfter = Number(ev.retry_after) || 0
						markSpooled(id, {
							indeterminate: false,
							progress: Math.min(100, Math.max(0, pct)),
							bytesUploaded: uploaded,
							bytesTotal: total || cur.bytesTotal || Number(file.size) || 0,
							retryAfter: retryAfter > 0 ? retryAfter : undefined,
						})
						return
					}

					// Phase 2 (Sarca→Telegram / done): determinate circular progress.
					const pct = Math.round(Number(ev.percent) || 0)
					markSpooled(id, {
						indeterminate: false,
						progress: Math.min(100, Math.max(0, pct)),
						bytesUploaded: uploaded,
						bytesTotal: total || cur.bytesTotal || Number(file.size) || 0,
						retryAfter: undefined,
					})
				},
				{ silent: true, signal: ac.signal },
			)

			const after = items().find((i) => i.id === id)
			if (!after || after.status === 'canceled') return
			const donePatch = {
				status: /** @type {UploadItemStatus} */ ('done'),
				indeterminate: false,
				progress: 100,
				bytesUploaded: after.bytesTotal || Number(file.size) || 0,
			}
			patchItem(id, donePatch)
			// After await API.files.uploadFile — server has emitted phase:done.
			notifyItemDone({ ...after, ...donePatch })
		} catch (err) {
			// cancelAll() marks items canceled before abort() resolves; keep that label.
			const cur = items().find((i) => i.id === id)
			if (!cur || cur.status === 'canceled') return

			// Unexpected XHR/signal abort (proxy drop, tab discard, etc.) is a failure —
			// never mislabel network/Telegram errors that merely contain the word "abort".
			const msg =
				isAbortError(err) || ac.signal.aborted
					? t('upload.interrupted')
					: formatUploadError(err)
			patchItem(id, {
				status: 'error',
				error: msg,
				progress: 0,
				indeterminate: false,
			})
			alertStore.addAlert(`${cur.name || file.name || 'file'}: ${msg}`, 'error')
		} finally {
			abortControllers.delete(id)
			// Always continue the pipeline — including after errors / cancels.
			queueMicrotask(() => {
				pump()
				notifyIdle()
			})
		}
	}

	const pump = () => {
		const list = items()
		if (!canStartSpool(list)) return
		const spooling = list.filter((i) => i.status === 'uploading' && !i.spooled).length
		const slots = MAX_SPOOL_CONCURRENT - spooling
		if (slots <= 0) return
		// Prefer smaller files first; stable sort keeps FIFO among equal sizes.
		const queued = list
			.filter((i) => i.status === 'queued')
			.slice()
			.sort((a, b) => {
				const sa = Number(a.bytesTotal) || Number(a.file?.size) || 0
				const sb = Number(b.bytesTotal) || Number(b.file?.size) || 0
				return sa - sb
			})
			.slice(0, slots)
		for (const item of queued) {
			void runUpload(item.id)
		}
	}

	/**
	 * @param {{
	 *   storageId: string,
	 *   parentPath: string,
	 *   file: File,
	 *   name?: string,
	 * }[]} entries
	 */
	const enqueue = (entries) => {
		if (!entries?.length) return

		const terminalOnly = !hasActiveWork()
		if (terminalOnly) {
			for (const id of abortControllers.keys()) {
				abortControllers.get(id)?.abort()
			}
			abortControllers.clear()
			setItems([])
			setCanceledBatch(false)
			setTransferStartedAt(0)
			setCollapsed(false)
		}

		const next = entries.map((entry) => {
			const basename =
				String(entry.name || entry.file?.name || 'unnamed')
					.split(/[/\\]/)
					.pop()
					.trim() || 'unnamed'
			return /** @type {UploadQueueItem} */ ({
				id: `up-${++uploadSeq}`,
				name: basename,
				status: 'queued',
				progress: 0,
				indeterminate: false,
				spooled: false,
				bytesUploaded: 0,
				bytesTotal: Number(entry.file?.size) || 0,
				storageId: entry.storageId,
				parentPath: entry.parentPath ?? '',
				file: entry.file,
			})
		})

		setItems((list) => [...list, ...next])
		setVisible(true)
		setCanceledBatch(false)
		pump()
	}

	const cancelAll = () => {
		// Mark canceled first so in-flight catch handlers see intentional cancel
		// (abort() rejects the upload promise and may run before this function returns).
		setItems((list) =>
			list.map((it) =>
				it.status === 'queued' || it.status === 'uploading'
					? { ...it, status: 'canceled', progress: 0, indeterminate: false }
					: it,
			),
		)
		setCanceledBatch(true)
		for (const [, ac] of abortControllers) {
			try {
				ac.abort()
			} catch {
				/* ignore */
			}
		}
		abortControllers.clear()
		setVisible(true)
		setCollapsed(false)
		notifyIdle()
	}

	/**
	 * Close (X): hide panel; keep background uploads unless nothing active,
	 * in which case clear the list (Drive: X after cancel dismisses).
	 */
	const dismiss = () => {
		if (hasActiveWork()) {
			setVisible(false)
			return
		}
		setItems([])
		setCanceledBatch(false)
		setTransferStartedAt(0)
		setVisible(false)
		setCollapsed(false)
	}

	const toggleCollapsed = () => setCollapsed((c) => !c)

	/**
	 * ETA / status strip text while uploads are active.
	 * @returns {string}
	 */
	const etaLabel = () => {
		const active = items().filter(
			(i) => i.status === 'queued' || i.status === 'uploading',
		)
		if (!active.length) return ''

		const waiting = active.find(
			(i) => i.status === 'uploading' && Number(i.retryAfter) > 0,
		)
		if (waiting) {
			const secs = Math.round(Number(waiting.retryAfter))
			return secs >= 60
				? `Waiting for Telegram… ~${Math.round(secs / 60)} min`
				: `Waiting for Telegram… ${secs}s`
		}

		let total = 0
		let uploaded = 0
		for (const it of active) {
			const size = Number(it.bytesTotal) || Number(it.file?.size) || 0
			total += size
			if (it.status === 'uploading') {
				uploaded += Number(it.bytesUploaded) || 0
			}
		}

		const started = transferStartedAt()
		const elapsedMs = started ? Date.now() - started : 0
		if (!started || uploaded < 64 * 1024 || elapsedMs < 1500) {
			return t('upload.uploading')
		}

		const rate = uploaded / (elapsedMs / 1000)
		if (!(rate > 0) || !(total > uploaded)) return t('upload.uploading')

		const secLeft = (total - uploaded) / rate
		if (!Number.isFinite(secLeft) || secLeft < 0) return t('upload.uploading')
		if (secLeft < 60) return t('upload.lessThanMinute')
		const mins = Math.round(secLeft / 60)
		if (mins < 60) return t('upload.minutesLeft', { mins })
		const hours = Math.floor(mins / 60)
		const rem = mins % 60
		return rem ? `${hours} hr ${rem} min left…` : `${hours} hr left…`
	}

	const headerTitle = () => {
		const list = items()
		const pending = list.filter(
			(i) => i.status === 'queued' || i.status === 'uploading',
		)
		if (pending.length) {
			return t(pending.length === 1 ? 'upload.uploadingOne' : 'upload.uploadingMany', {
				count: pending.length,
			})
		}
		const canceled = list.filter((i) => i.status === 'canceled').length
		if (canceledBatch() || (canceled > 0 && !list.some((i) => i.status === 'done'))) {
			const n = canceled || list.length
			return t(n === 1 ? 'upload.canceledOne' : 'upload.canceledMany', { count: n })
		}
		if (canceled > 0) {
			return t(canceled === 1 ? 'upload.canceledOne' : 'upload.canceledMany', { count: canceled })
		}
		const done = list.filter((i) => i.status === 'done').length
		const errors = list.filter((i) => i.status === 'error').length
		if (errors && !done) {
			return t(errors === 1 ? 'upload.failedOne' : 'upload.failedMany', { count: errors })
		}
		if (done) {
			return t(done === 1 ? 'upload.completeOne' : 'upload.completeMany', { count: done })
		}
		return t('upload.title')
	}

	const showCancelStrip = () => hasActiveWork()

	return {
		items,
		visible,
		collapsed,
		enqueue,
		cancelAll,
		dismiss,
		toggleCollapsed,
		onIdle,
		onItemDone,
		hasActiveWork,
		etaLabel,
		headerTitle,
		showCancelStrip,
	}
})
