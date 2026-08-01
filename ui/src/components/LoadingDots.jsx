import { createSignal, onCleanup } from 'solid-js'

/**
 * Cheap stand-in for an indeterminate spinner: cycles "." -> ".." -> "..."
 * as plain text. A spinner's continuous CSS animation forces the browser to
 * recomposite every frame for as long as it's visible — costly under
 * WebKitGTK, especially over long/open-ended waits. Updating a text node
 * every 450ms has no such cost.
 */
const LoadingDots = () => {
	const [count, setCount] = createSignal(1)
	const id = window.setInterval(() => setCount((n) => (n % 3) + 1), 450)
	onCleanup(() => window.clearInterval(id))
	return <span>{'.'.repeat(count())}</span>
}

export default LoadingDots
