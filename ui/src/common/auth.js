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
