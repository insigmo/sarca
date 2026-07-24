import { alertStore } from '../components/AlertStack'
import createLocalStore from '../../libs'

// Same-origin by default so production / Docker UI talks to the Sarca that
// served the page. Override with VITE_API_BASE for `pnpm dev` against a remote API.
const API_BASE = import.meta.env.VITE_API_BASE || '/api'

export { API_BASE }

let refreshPromise = null

/**
 * Attempt a single token refresh using the stored refresh_token.
 * Concurrent callers share one in-flight refresh.
 * @returns {Promise<string|null>} new Bearer token or null on failure
 */
const tryRefreshToken = async () => {
	if (refreshPromise) {
		return refreshPromise
	}

	refreshPromise = (async () => {
		const [store, setStore, remove] = createLocalStore()
		const refreshToken = store.refresh_token
		if (!refreshToken) {
			return null
		}

		try {
			const response = await fetch(`${API_BASE}/auth/refresh`, {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ refresh_token: refreshToken }),
			})

			if (!response.ok) {
				remove('access_token')
				remove('refresh_token')
				return null
			}

			const data = await response.json()
			setStore('access_token', data.access_token)
			setStore('refresh_token', data.refresh_token)
			return `Bearer ${data.access_token}`
		} catch {
			return null
		} finally {
			refreshPromise = null
		}
	})()

	return refreshPromise
}

/**
 * @typedef {'get' | 'post' | 'patch' | 'delete'} Method
 */

/**
 *
 * @param {string} path
 * @param {Method} method
 * @param {string | null | undefined} auth_token
 * @param {any} body
 * @param {boolean} return_response
 * @param {boolean} [retried]
 * @returns
 */
const apiRequest = async (
	path,
	method,
	auth_token,
	body,
	return_response = false,
	retried = false,
) => {
	const { addAlert } = alertStore

	const fullpath = `${API_BASE}${path}`

	const headers = new Headers()
	headers.append('Content-Type', 'application/json')
	if (auth_token) {
		headers.append('Authorization', auth_token)
	}

	try {
		const response = await fetch(fullpath, {
			method,
			body: body === undefined ? undefined : JSON.stringify(body),
			headers,
		})

		if (response.status === 401 && auth_token && !retried) {
			const newToken = await tryRefreshToken()
			if (newToken) {
				return apiRequest(
					path,
					method,
					newToken,
					body,
					return_response,
					true,
				)
			}
		}

		if (!response.ok) {
			throw new Error(await response.text())
		}

		if (return_response) {
			return response
		}

		try {
			return await response.json()
		} catch {}
	} catch (err) {
		addAlert(err.message, 'error')

		throw err
	}
}

/**
 *
 * @param {string} path
 * @param {string | null | undefined} auth_token
 * @param {FormData} form
 * @param {(progress: number) => void} [onProgress]
 * @returns
 */
export const apiMultipartRequest = (path, auth_token, form, onProgress) => {
	const { addAlert } = alertStore
	const fullpath = `${API_BASE}${path}`

	return new Promise((resolve, reject) => {
		const xhr = new XMLHttpRequest()

		xhr.open('POST', fullpath)

		if (auth_token) {
			xhr.setRequestHeader('Authorization', auth_token)
		}

		if (onProgress) {
			xhr.upload.onprogress = (event) => {
				if (event.lengthComputable) {
					const percentComplete = (event.loaded / event.total) * 100
					onProgress(percentComplete)
				}
			}
		}

		xhr.onload = async () => {
			if (xhr.status === 401 && auth_token) {
				const newToken = await tryRefreshToken()
				if (newToken) {
					apiMultipartRequest(path, newToken, form, onProgress)
						.then(resolve)
						.catch(reject)
					return
				}
			}

			if (xhr.status >= 200 && xhr.status < 300) {
				try {
					const json = JSON.parse(xhr.responseText)
					resolve(json)
				} catch {
					resolve(xhr.responseText)
				}
			} else {
				const errorMsg = xhr.responseText || 'Upload failed'
				addAlert(errorMsg, 'error')
				reject(new Error(errorMsg))
			}
		}

		xhr.onerror = () => {
			addAlert('Network Error', 'error')
			reject(new Error('Network Error'))
		}

		xhr.send(form)
	})
}

export default apiRequest
