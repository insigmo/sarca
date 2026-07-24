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
 * @typedef {'get' | 'post' | 'put' | 'patch' | 'delete'} Method
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
 * Format bytes for upload status (binary units).
 * @param {number} n
 */
export const formatUploadBytes = (n) => {
	const v = Number(n) || 0
	if (v < 1024) return `${v} B`
	if (v < 1024 * 1024) return `${(v / 1024).toFixed(1)} KiB`
	if (v < 1024 * 1024 * 1024) return `${(v / (1024 * 1024)).toFixed(1)} MiB`
	return `${(v / (1024 * 1024 * 1024)).toFixed(2)} GiB`
}

/**
 * @typedef {Object} UploadProgressEvent
 * @property {'server' | 'telegram'} phase
 * @property {number} percent
 * @property {number} [uploaded]
 * @property {number} [total]
 * @property {number} [chunk]
 * @property {number} [chunks]
 */

/**
 *
 * @param {string} path
 * @param {string | null | undefined} auth_token
 * @param {FormData} form
 * @param {(progress: UploadProgressEvent) => void} [onProgress]
 * @param {{ silent?: boolean }} [options]
 * @returns
 */
export const apiMultipartRequest = (path, auth_token, form, onProgress, options = {}) => {
	const { addAlert } = alertStore
	const fullpath = `${API_BASE}${path}`
	const silent = Boolean(options.silent)

	return new Promise((resolve, reject) => {
		const xhr = new XMLHttpRequest()
		let parsedLen = 0
		let streamError = null
		let streamDone = false

		const emit = (ev) => {
			if (onProgress) onProgress(ev)
		}

		const consumeNdjson = (text) => {
			if (!text || text.length <= parsedLen) return
			const chunk = text.slice(parsedLen)
			const parts = chunk.split('\n')
			const incomplete = parts.pop() ?? ''
			parsedLen = text.length - incomplete.length

			for (const line of parts) {
				const trimmed = line.trim()
				if (!trimmed) continue
				try {
					const ev = JSON.parse(trimmed)
					if (ev.phase === 'telegram') {
						const total = Number(ev.total) || 0
						const uploaded = Number(ev.uploaded) || 0
						const percent = total > 0 ? (uploaded / total) * 100 : 0
						emit({
							phase: 'telegram',
							percent,
							uploaded,
							total,
							chunk: ev.chunk,
							chunks: ev.chunks,
						})
					} else if (ev.phase === 'error') {
						streamError = ev.message || 'Upload failed'
					} else if (ev.phase === 'done') {
						streamDone = true
						emit({ phase: 'telegram', percent: 100 })
					}
				} catch {
					// ignore partial / non-json fragments
				}
			}
		}

		xhr.open('POST', fullpath)

		if (auth_token) {
			xhr.setRequestHeader('Authorization', auth_token)
		}

		xhr.upload.onprogress = (event) => {
			if (event.lengthComputable) {
				emit({
					phase: 'server',
					percent: (event.loaded / event.total) * 100,
					uploaded: event.loaded,
					total: event.total,
				})
			}
		}

		xhr.onprogress = () => {
			consumeNdjson(xhr.responseText || '')
		}

		xhr.onload = async () => {
			if (xhr.status === 401 && auth_token) {
				const newToken = await tryRefreshToken()
				if (newToken) {
					apiMultipartRequest(path, newToken, form, onProgress, options)
						.then(resolve)
						.catch(reject)
					return
				}
			}

			consumeNdjson(xhr.responseText || '')

			if (xhr.status >= 200 && xhr.status < 300) {
				if (streamError) {
					if (!silent) addAlert(streamError, 'error')
					reject(new Error(streamError))
					return
				}
				if (
					xhr.responseText &&
					xhr.responseText.includes('"phase"') &&
					!streamDone
				) {
					const msg = 'Upload did not complete'
					if (!silent) addAlert(msg, 'error')
					reject(new Error(msg))
					return
				}
				try {
					const json = JSON.parse(xhr.responseText)
					resolve(json)
				} catch {
					resolve(xhr.responseText)
				}
			} else {
				const errorMsg = xhr.responseText || 'Upload failed'
				if (!silent) addAlert(errorMsg, 'error')
				reject(new Error(errorMsg))
			}
		}

		xhr.onerror = () => {
			if (!silent) addAlert('Network Error', 'error')
			reject(new Error('Network Error'))
		}

		xhr.send(form)
	})
}

export default apiRequest
