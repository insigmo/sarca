import { Index, Show, createSignal, onCleanup, onMount } from 'solid-js'
import IconButton from '@suid/material/IconButton'

import { uploadQueueStore } from '../common/uploadQueue'
import FileTypeIcon from './FileTypeIcon'
import FluentIcon from './FluentIcon'

const SIZE_KEY = 'sarca.uploadMgr.size'
const DEFAULT_W = 360
const DEFAULT_H = 280
const MIN_W = 280
const MIN_H = 160

const maxW = () => Math.min(480, Math.floor(window.innerWidth * 0.9))
const maxH = () => Math.min(420, Math.floor(window.innerHeight * 0.7))

const clampSize = (w, h) => ({
	w: Math.min(maxW(), Math.max(MIN_W, Math.round(w))),
	h: Math.min(maxH(), Math.max(MIN_H, Math.round(h))),
})

const loadSize = () => {
	try {
		const raw = sessionStorage.getItem(SIZE_KEY)
		if (!raw) return clampSize(DEFAULT_W, DEFAULT_H)
		const parsed = JSON.parse(raw)
		if (
			typeof parsed?.w !== 'number' ||
			typeof parsed?.h !== 'number' ||
			!Number.isFinite(parsed.w) ||
			!Number.isFinite(parsed.h)
		) {
			return clampSize(DEFAULT_W, DEFAULT_H)
		}
		return clampSize(parsed.w, parsed.h)
	} catch {
		return clampSize(DEFAULT_W, DEFAULT_H)
	}
}

const saveSize = (size) => {
	try {
		sessionStorage.setItem(SIZE_KEY, JSON.stringify(size))
	} catch {
		/* ignore quota / private mode */
	}
}

/**
 * Circular progress ring.
 * - queued: empty static track
 * - uploading + indeterminate (server/XHR): spinning partial arc
 * - uploading + determinate (telegram): arc filled by %
 * @param {{ value?: number, active?: boolean, indeterminate?: boolean }} props
 */
const UploadRing = (props) => {
	const r = 9
	const c = 2 * Math.PI * r
	const dashOffset = () => {
		const v = Math.min(100, Math.max(0, Number(props.value) || 0))
		return c - (c * v) / 100
	}
	/** ~33% arc so rotation reads clearly while spinning. */
	const spinDash = `${0.33 * c} ${c}`

	return (
		<svg
			class="upload-mgr__ring"
			classList={{
				'upload-mgr__ring--indeterminate': Boolean(props.indeterminate),
			}}
			width="22"
			height="22"
			viewBox="0 0 24 24"
			aria-hidden="true"
		>
			<circle
				class="upload-mgr__ring-track"
				cx="12"
				cy="12"
				r={r}
				fill="none"
				stroke-width="2"
			/>
			<Show when={props.indeterminate}>
				<circle
					class="upload-mgr__ring-arc upload-mgr__ring-arc--indeterminate"
					cx="12"
					cy="12"
					r={r}
					fill="none"
					stroke-width="2"
					stroke-linecap="round"
					stroke-dasharray={spinDash}
					transform="rotate(-90 12 12)"
				/>
			</Show>
			<Show when={props.active && !props.indeterminate}>
				<circle
					class="upload-mgr__ring-arc"
					cx="12"
					cy="12"
					r={r}
					fill="none"
					stroke-width="2"
					stroke-linecap="round"
					stroke-dasharray={String(c)}
					stroke-dashoffset={dashOffset()}
					transform="rotate(-90 12 12)"
				/>
			</Show>
		</svg>
	)
}

/**
 * Google Drive–style floating upload manager.
 * Anchored bottom-right; SE corner drag handle resizes into the page.
 */
const UploadManager = () => {
	const q = uploadQueueStore
	const [size, setSize] = createSignal(clampSize(DEFAULT_W, DEFAULT_H))

	onMount(() => {
		setSize(loadSize())
		const onWinResize = () => setSize((s) => clampSize(s.w, s.h))
		window.addEventListener('resize', onWinResize)
		onCleanup(() => window.removeEventListener('resize', onWinResize))
	})

	const onResizePointerDown = (e) => {
		if (q.collapsed()) return
		e.preventDefault()
		e.stopPropagation()
		const el = e.currentTarget
		const pointerId = e.pointerId
		el.setPointerCapture?.(pointerId)

		const startX = e.clientX
		const startY = e.clientY
		const start = size()

		const onMove = (ev) => {
			// Panel is fixed to bottom-right: SE drag grows width/height into the page.
			const next = clampSize(
				start.w + (ev.clientX - startX),
				start.h + (ev.clientY - startY),
			)
			setSize(next)
		}

		const onUp = (ev) => {
			el.releasePointerCapture?.(pointerId)
			el.removeEventListener('pointermove', onMove)
			el.removeEventListener('pointerup', onUp)
			el.removeEventListener('pointercancel', onUp)
			if (ev) {
				const next = clampSize(
					start.w + (ev.clientX - startX),
					start.h + (ev.clientY - startY),
				)
				setSize(next)
				saveSize(next)
			} else {
				saveSize(size())
			}
		}

		el.addEventListener('pointermove', onMove)
		el.addEventListener('pointerup', onUp)
		el.addEventListener('pointercancel', onUp)
	}

	return (
		<Show when={q.visible() && q.items().length > 0}>
			<div
				class="upload-mgr"
				classList={{ 'upload-mgr--collapsed': q.collapsed() }}
				role="dialog"
				aria-label={q.headerTitle()}
				style={
					q.collapsed()
						? { width: `${size().w}px` }
						: { width: `${size().w}px`, height: `${size().h}px` }
				}
			>
				<div class="upload-mgr__header">
					<span class="upload-mgr__title">{q.headerTitle()}</span>
					<div class="upload-mgr__actions">
						<IconButton
							size="small"
							aria-label={q.collapsed() ? 'Expand' : 'Collapse'}
							onClick={() => q.toggleCollapsed()}
							sx={{ color: 'var(--sarca-ink-soft)' }}
						>
							<span
								class="upload-mgr__chevron"
								classList={{
									'upload-mgr__chevron--up': q.collapsed(),
								}}
							>
								<FluentIcon name="chevronDown" size={18} />
							</span>
						</IconButton>
						<IconButton
							size="small"
							aria-label="Close"
							onClick={() => q.dismiss()}
							sx={{ color: 'var(--sarca-ink-soft)' }}
						>
							<FluentIcon name="dismiss" size={18} />
						</IconButton>
					</div>
				</div>

				<Show when={!q.collapsed()}>
					<Show when={q.showCancelStrip()}>
						<div class="upload-mgr__status">
							<span class="upload-mgr__eta">{q.etaLabel()}</span>
							<button
								type="button"
								class="upload-mgr__cancel"
								onClick={() => q.cancelAll()}
							>
								Cancel
							</button>
						</div>
					</Show>

					<ul class="upload-mgr__list">
						{/* Index: avoid For remounts on patchItem that reset the spin animation. */}
						<Index each={q.items()}>
							{(item) => (
								<li class="upload-mgr__row">
									<span class="upload-mgr__icon">
										<FileTypeIcon
											name={item().name}
											isFile
											size={28}
										/>
									</span>
									<span class="upload-mgr__name" title={item().name}>
										{item().name}
									</span>
									<span class="upload-mgr__trail">
										<Show
											when={
												item().status === 'canceled' ||
												item().status === 'error'
											}
											fallback={
												<Show
													when={
														item().status === 'queued' ||
														item().status === 'uploading'
													}
													fallback={
														<span
															class="upload-mgr__done"
															aria-hidden="true"
														>
															✓
														</span>
													}
												>
													<UploadRing
														active={item().status === 'uploading'}
														indeterminate={
															item().status === 'uploading' &&
															Boolean(item().indeterminate)
														}
														value={
															item().status === 'uploading'
																? item().progress
																: 0
														}
													/>
												</Show>
											}
										>
											<span
												class="upload-mgr__status-text"
												title={
													item().status === 'error'
														? item().error || 'Upload failed'
														: 'Upload canceled'
												}
											>
												{item().status === 'error'
													? item().error || 'Upload failed'
													: 'Upload canceled'}
											</span>
										</Show>
									</span>
								</li>
							)}
						</Index>
					</ul>

					<button
						type="button"
						class="upload-mgr__resize"
						aria-label="Resize upload panel"
						title="Resize"
						onPointerDown={onResizePointerDown}
					/>
				</Show>
			</div>
		</Show>
	)
}

export default UploadManager
