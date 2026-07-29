export const PTR_THRESHOLD_PX = 64
export const PTR_MAX_PX = 96

export function canBeginPull({ scrollTop, refreshing }) {
	return scrollTop <= 0 && !refreshing
}

export function pullDelta({ startY, currentY }) {
	return Math.max(0, currentY - startY)
}

export function isHorizontalGesture({ startX, startY, currentX, currentY }) {
	const dx = currentX - startX
	const dy = currentY - startY
	return Math.abs(dx) > Math.abs(dy) && Math.abs(dx) > 10
}

export function shouldTriggerRefresh(pullPx) {
	return pullPx >= PTR_THRESHOLD_PX
}

/**
 * @param {HTMLElement} el scroll container
 * @param {{
 *   onRefresh: () => Promise<void> | void,
 *   isEnabled?: () => boolean,
 *   onPullChange?: (px: number) => void,
 *   onRefreshingChange?: (v: boolean) => void,
 * }} opts
 * @returns {() => void} detach
 */
export function attachPullToRefresh(el, opts) {
	let startX = 0
	let startY = 0
	let pulling = false
	let armed = false
	let refreshing = false
	let pullPx = 0

	const setPull = (px) => {
		pullPx = Math.min(PTR_MAX_PX, Math.max(0, px))
		opts.onPullChange?.(pullPx)
	}

	const onStart = (e) => {
		if (opts.isEnabled && !opts.isEnabled()) return
		if (!canBeginPull({ scrollTop: el.scrollTop, refreshing })) return
		const t = e.touches[0]
		startX = t.clientX
		startY = t.clientY
		armed = true
		pulling = false
	}

	const onMove = (e) => {
		if (!armed || refreshing) return
		const t = e.touches[0]
		if (
			isHorizontalGesture({
				startX,
				startY,
				currentX: t.clientX,
				currentY: t.clientY,
			})
		) {
			armed = false
			setPull(0)
			return
		}
		const d = pullDelta({ startY, currentY: t.clientY })
		if (d > 0 && el.scrollTop <= 0) {
			pulling = true
			if (e.cancelable) e.preventDefault()
			setPull(d)
		}
	}

	const onEnd = async () => {
		if (!armed && !pulling) return
		armed = false
		const trigger = pulling && shouldTriggerRefresh(pullPx)
		pulling = false
		if (!trigger) {
			setPull(0)
			return
		}
		refreshing = true
		opts.onRefreshingChange?.(true)
		setPull(PTR_THRESHOLD_PX)
		try {
			await opts.onRefresh()
		} finally {
			refreshing = false
			opts.onRefreshingChange?.(false)
			setPull(0)
		}
	}

	el.addEventListener('touchstart', onStart, { passive: true })
	el.addEventListener('touchmove', onMove, { passive: false })
	el.addEventListener('touchend', onEnd)
	el.addEventListener('touchcancel', onEnd)
	return () => {
		el.removeEventListener('touchstart', onStart)
		el.removeEventListener('touchmove', onMove)
		el.removeEventListener('touchend', onEnd)
		el.removeEventListener('touchcancel', onEnd)
	}
}
