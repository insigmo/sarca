import { alertStore } from '../components/AlertStack'
import createLocalStore from '../../libs'
import { safeRedirectPath } from '../common/auth'

// Same-origin by default so production / Docker UI talks to the Sarca that
// served the page. Override with VITE_API_BASE for `pnpm dev` against a remote API.
const API_BASE = import.meta.env.VITE_API_BASE || '/api'

export { API_BASE }

let refreshPromise = null
let sessionExpiredHandled = false

/**
 * Decode a JWT's payload without verifying the signature — good enough to
 * read `exp` client-side and avoid sending requests we know will 401.
 * @param {string} token
 * @returns {{ exp?: number } | null}
 */
const decodeJwtPayload = (token) => {
	try {
		const part = token.split('.')[1]
		const base64 = part.replace(/-/g, '+').replace(/_/g, '/')
		return JSON.parse(atob(base64))
	} catch {
		return null
	}
}

/**
 * @param {string | null | undefined} token
 * @param {number} [skewSeconds] Treat the token as expired this many seconds early.
 */
const isTokenExpired = (token, skewSeconds = 15) => {
	if (!token) return true
	const payload = decodeJwtPayload(token)
	if (!payload?.exp) return false
	return Date.now() >= payload.exp * 1000 - skewSeconds * 1000
}

/**
 * The refresh token is gone or rejected — the session is genuinely over.
 * Clear it once and send the user to /login instead of letting every
 * in-flight request surface its own raw "not authenticated" alert.
 */
const forceReLogin = () => {
	if (sessionExpiredHandled) return
	sessionExpiredHandled = true
	const [, setStore, remove] = createLocalStore()
	remove('access_token')
	remove('refresh_token')
	setStore('redirect', safeRedirectPath(window.location.pathname))
	window.location.assign('/login')
}

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
 * Current access token, refreshed first if it looks expired (or already
 * gone). Used for `<img>`/`<video>`/`<iframe>` URLs, which can't rely on
 * apiRequest's reactive 401-retry since the browser loads them directly.
 * @returns {Promise<string|null>} raw access token (no "Bearer " prefix)
 */
export const getFreshAccessToken = async () => {
	const [store] = createLocalStore()
	if (!store.access_token) return null
	if (!isTokenExpired(store.access_token)) return store.access_token

	const refreshed = await tryRefreshToken()
	if (!refreshed) {
		forceReLogin()
		return null
	}
	return refreshed.replace(/^Bearer /, '')
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
	silent = false,
	signal = undefined,
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
			signal,
		})

		if (response.status === 401 && auth_token) {
			// Only spend a refresh attempt on the first 401 — if the retried
			// request (already carrying a fresh token) 401s again, the
			// session is genuinely dead and must not surface as a raw error.
			const newToken = retried ? null : await tryRefreshToken()
			if (newToken) {
				return apiRequest(
					path,
					method,
					newToken,
					body,
					return_response,
					true,
					silent,
					signal,
				)
			}
			forceReLogin()
		}

		if (!response.ok) {
			const text = await response.text()
			const err = new Error(text)
			err.status = response.status
			throw err
		}

		if (return_response) {
			return response
		}

		try {
			return await response.json()
		} catch {}
	} catch (err) {
		if (err?.name === 'AbortError' || signal?.aborted) {
			throw err
		}
		if (!silent && !sessionExpiredHandled) {
			addAlert(err.message, 'error')
		}

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
 * @property {'server' | 'spooled' | 'telegram' | 'waiting' | 'heartbeat'} phase
 * @property {number} percent
 * @property {number} [uploaded]
 * @property {number} [total]
 * @property {number} [chunk]
 * @property {number} [chunks]
 * @property {number} [retry_after] - Seconds Telegram asked us to wait (flood control).
 */

/**
 * Parse one NDJSON line from an upload response.
 * `phase: spooled` = bytes on Sarca + DB row; free the client spool slot.
 * `phase: waiting` is flood-control; keep progress alive and pass `retry_after`
 * so the queue can show a waiting status instead of looking stuck.
 * `phase: heartbeat` keeps the HTTP stream alive during Telegram quiet periods.
 * @param {string} line
 * @param {(progress: UploadProgressEvent) => void} [emit]
 * @returns {{ error?: string, done?: boolean }}
 */
const handleUploadNdjsonLine = (line, emit) => {
	const trimmed = line.trim()
	if (!trimmed) return {}
	try {
		const ev = JSON.parse(trimmed)
		if (ev.phase === 'heartbeat') {
			// Keepalive only — do not change UI progress / spool state.
			return {}
		}
		if (ev.phase === 'spooled') {
			const total = Number(ev.total) || 0
			emit?.({
				phase: 'spooled',
				percent: 0,
				uploaded: 0,
				total,
			})
		} else if (ev.phase === 'telegram' || ev.phase === 'waiting') {
			const total = Number(ev.total) || 0
			const uploaded = Number(ev.uploaded) || 0
			const percent = total > 0 ? (uploaded / total) * 100 : 0
			const retryAfter = Number(ev.retry_after)
			emit?.({
				phase: ev.phase === 'waiting' ? 'waiting' : 'telegram',
				percent,
				uploaded,
				total,
				chunk: ev.chunk,
				chunks: ev.chunks,
				...(Number.isFinite(retryAfter) && retryAfter > 0
					? { retry_after: retryAfter }
					: {}),
			})
		} else if (ev.phase === 'error') {
			return { error: ev.message || 'Upload failed' }
		} else if (ev.phase === 'done') {
			emit?.({ phase: 'telegram', percent: 100 })
			return { done: true }
		} else if (ev.uploaded != null && ev.total != null) {
			// Fallback: any progress-shaped line clears phase-1 spinner.
			const total = Number(ev.total) || 0
			const uploaded = Number(ev.uploaded) || 0
			const percent = total > 0 ? (uploaded / total) * 100 : 0
			emit?.({
				phase: 'telegram',
				percent,
				uploaded,
				total,
				chunk: ev.chunk,
				chunks: ev.chunks,
			})
		}
	} catch {
		// ignore partial / non-json fragments
	}
	return {}
}

/**
 * Multipart upload with live Telegram progress.
 *
 * Uses fetch + ReadableStream (not XHR) so NDJSON progress lines are consumed
 * as they arrive during Sarca→Telegram. XHR often buffers responseText until
 * the request completes, which left the UI stuck on the phase-1 spinner.
 *
 * Phase 1 (client→Sarca): no upload % is reported — callers keep an
 * indeterminate spinner. `phase: spooled` frees the spool slot for pipelining.
 * Phase 2 starts on the first `phase:telegram` line.
 */
export const apiMultipartRequest = (path, auth_token, form, onProgress, options = {}) => {
	const { addAlert } = alertStore
	const fullpath = `${API_BASE}${path}`
	const silent = Boolean(options.silent)
	const signal = options.signal

	const emit = (ev) => {
		if (onProgress) onProgress(ev)
	}

	const fail = (message) => {
		if (!silent && !sessionExpiredHandled) addAlert(message, 'error')
		return Promise.reject(new Error(message))
	}

	/**
	 * @param {string | null | undefined} token
	 * @param {boolean} retried
	 */
	const run = async (token, retried) => {
		if (signal?.aborted) {
			throw new DOMException('Aborted', 'AbortError')
		}

		const headers = new Headers()
		if (token) {
			headers.append('Authorization', token)
		}

		let response
		try {
			response = await fetch(fullpath, {
				method: 'POST',
				headers,
				body: form,
				signal,
			})
		} catch (err) {
			if (err?.name === 'AbortError' || signal?.aborted) {
				throw err instanceof DOMException
					? err
					: new DOMException('Aborted', 'AbortError')
			}
			return fail(err?.message || 'Network Error')
		}

		if (response.status === 401 && token && !retried) {
			const newToken = await tryRefreshToken()
			if (newToken) {
				return run(newToken, true)
			}
			forceReLogin()
		}

		if (!response.ok) {
			const text = await response.text().catch(() => '')
			return fail(text || 'Upload failed')
		}

		let streamError = null
		let streamDone = false
		let sawPhase = false
		let sawTerminalPhase = false
		let rawFallback = ''

		const applyLine = (line) => {
			const result = handleUploadNdjsonLine(line, emit)
			if (line.includes('"phase"') && !line.includes('"heartbeat"')) {
				sawPhase = true
			}
			if (result.error) {
				streamError = result.error
				sawTerminalPhase = true
			}
			if (result.done) {
				streamDone = true
				sawTerminalPhase = true
			}
		}

		const reader = response.body?.getReader?.()
		if (reader) {
			const decoder = new TextDecoder()
			let buffer = ''
			try {
				while (true) {
					const { done, value } = await reader.read()
					if (done) break
					buffer += decoder.decode(value, { stream: true })
					const parts = buffer.split('\n')
					buffer = parts.pop() ?? ''
					for (const line of parts) {
						applyLine(line)
					}
				}
				buffer += decoder.decode()
				if (buffer.trim()) {
					applyLine(buffer)
				}
			} catch (err) {
				if (err?.name === 'AbortError' || signal?.aborted) {
					throw err instanceof DOMException
						? err
						: new DOMException('Aborted', 'AbortError')
				}
				// Stream closed mid-flight (proxy idle timeout, navigation, etc.).
				if (!sawTerminalPhase) {
					return fail(
						err?.message ||
							'Upload connection lost before completion — please retry',
					)
				}
				return fail(err?.message || 'Upload failed')
			}
		} else {
			// Rare: no body stream — parse the whole payload at once.
			rawFallback = await response.text().catch(() => '')
			for (const line of rawFallback.split('\n')) {
				applyLine(line)
			}
		}

		if (streamError) {
			return fail(streamError)
		}
		if (sawPhase && !streamDone) {
			return fail(
				'Upload connection closed before Telegram finished — please retry',
			)
		}

		// NDJSON uploads resolve with no JSON body; legacy JSON still parses.
		if (rawFallback) {
			try {
				return JSON.parse(rawFallback)
			} catch {
				return rawFallback
			}
		}
		return undefined
	}

	return run(auth_token, false)
}

/**
 * Unauthenticated public API calls (share links). Always sends cookies so
 * HttpOnly unlock cookies work after POST /unlock.
 *
 * @param {string} path
 * @param {Method} method
 * @param {any} [body]
 * @param {boolean} [return_response]
 * @param {boolean} [silent]
 */
export const publicApiRequest = async (
	path,
	method,
	body,
	return_response = false,
	silent = false,
	signal = undefined,
) => {
	const { addAlert } = alertStore
	const fullpath = `${API_BASE}${path}`
	const headers = new Headers()
	headers.append('Content-Type', 'application/json')

	try {
		const response = await fetch(fullpath, {
			method,
			body: body === undefined ? undefined : JSON.stringify(body),
			headers,
			credentials: 'include',
			signal,
		})

		if (!response.ok) {
			const text = await response.text()
			let parsed = null
			try {
				parsed = text ? JSON.parse(text) : null
			} catch {
				/* plain text error body */
			}
			const err = new Error(
				(parsed && (parsed.message || parsed.error)) || text || response.statusText,
			)
			err.status = response.status
			err.body = parsed
			throw err
		}

		if (return_response) {
			return response
		}

		if (response.status === 204) {
			return undefined
		}

		try {
			return await response.json()
		} catch {
			return undefined
		}
	} catch (err) {
		if (err?.name === 'AbortError' || signal?.aborted) {
			throw err
		}
		if (!silent) {
			addAlert(err.message || 'Request failed', 'error')
		}
		throw err
	}
}

export default apiRequest
