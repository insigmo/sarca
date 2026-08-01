/**
 * Batches a high-frequency callback (e.g. per-mousemove hit-testing) to run
 * at most once per animation frame, using only the latest scheduled args.
 * @template {(...args: any[]) => void} Fn
 * @param {Fn} fn
 */
export const createRafBatcher = (fn) => {
	/** @type {number | null} */
	let rafId = null
	/** @type {Parameters<Fn>} */
	let pendingArgs

	/** @param {...Parameters<Fn>} args */
	const schedule = (...args) => {
		pendingArgs = args
		if (rafId != null) return
		rafId = requestAnimationFrame(() => {
			rafId = null
			fn(...pendingArgs)
		})
	}

	const cancel = () => {
		if (rafId != null) {
			cancelAnimationFrame(rafId)
			rafId = null
		}
	}

	return { schedule, cancel }
}
