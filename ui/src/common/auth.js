/** Routes that render without a session. Never a post-login destination. */
const AUTH_PATHS = ['/login']

/**
 * @param {string} path
 * @returns {boolean}
 */
const isAuthPath = (path) => {
	const clean = path.split('?')[0].split('#')[0].replace(/\/+$/, '') || '/'
	return AUTH_PATHS.includes(clean)
}

/**
 * Where to land after a successful sign-in.
 *
 * `redirect` is written by the 401 handler and the route guard from whatever
 * path was current, and a 401 can land while the user is already sitting on
 * /login (the native client re-injects stale tokens on every navigate). That
 * stored `/login` made the post-login `navigate()` a no-op: the token was
 * saved, the page never moved, and only a restart looked signed in.
 *
 * @param {unknown} raw Stored redirect value.
 * @returns {string} A safe in-app path, never an auth route or a foreign origin.
 */
export const safeRedirectPath = (raw) => {
	if (typeof raw !== 'string') return '/'
	const path = raw.trim()
	// Same-origin absolute paths only — "//evil.com" and "https://…" are not ours.
	if (!path.startsWith('/') || path.startsWith('//')) return '/'
	if (isAuthPath(path)) return '/'
	return path
}

/**
 * Clear the locally persisted session and return the login route.
 * Callers are responsible for navigating after any UI cleanup.
 *
 * @param {(key: string, value?: unknown) => void} setStore
 * @returns {string}
 */
export const clearSession = (setStore) => {
	setStore('access_token')
	setStore('refresh_token')
	setStore('user')
	setStore('redirect', '/')

	return '/login'
}
