import createLocalStore from '../../libs'

import apiRequest, {
	apiMultipartRequest,
	publicApiRequest,
	getFreshAccessToken,
	API_BASE,
} from './request'
import { alertStore } from '../components/AlertStack'
import { makeThumbBlob } from '../common/thumbMaker'
import { putCachedThumb } from '../common/previewCache'
import { createRafBatcher } from '../common/rafBatch'

/////////////////////////////////////////////////////////////
////  USERS
/////////////////////////////////////////////////////////////

/**
 * @typedef {Object} TokenData
 * @property {string} access_token
 */

/**
 * Create a user (superuser only).
 * @param {string} email
 * @param {string} password
 * @returns {Promise<void>}
 */
const createUser = async (email, password) => {
	return await apiRequest('/users', 'post', getAuthToken(), {
		email,
		password,
	})
}

/**
 * List all users (superuser only).
 * @returns {Promise<{users: Array<{id: string, email: string, email_verified: boolean, is_superuser: boolean, disabled: boolean}>}>}
 */
const listUsers = async () => {
	return await apiRequest('/users', 'get', getAuthToken())
}

/**
 * Change the caller's own password. The server revokes every session on a
 * password change (including this one), so it hands back a fresh token pair
 * that the caller must persist the same way the login flow does.
 * @param {string} currentPassword
 * @param {string} newPassword
 * @returns {Promise<TokenData & {email_verified: boolean}>}
 */
const changeMyPassword = async (currentPassword, newPassword) => {
	return await apiRequest('/users/me/password', 'put', getAuthToken(), {
		current_password: currentPassword,
		new_password: newPassword,
	})
}

/**
 * Set another user's password (superuser only). No current-password check;
 * the target's sessions are revoked.
 * @param {string} userId
 * @param {string} newPassword
 * @returns {Promise<void>}
 */
const setUserPassword = async (userId, newPassword) => {
	return await apiRequest(
		`/users/${userId}/password`,
		'put',
		getAuthToken(),
		{ new_password: newPassword },
	)
}

/**
 * Enable or disable a user's account (superuser only). Disabling blocks
 * login and revokes existing sessions immediately.
 * @param {string} userId
 * @param {boolean} disabled
 * @returns {Promise<void>}
 */
const setUserDisabled = async (userId, disabled) => {
	return await apiRequest(`/users/${userId}/disabled`, 'put', getAuthToken(), {
		disabled,
	})
}

/**
 * Directory of registered users, for the grant-access autocomplete. Open to
 * the superuser or any user holding AccessType::A on at least one storage.
 * @returns {Promise<{users: Array<{id: string, email: string}>}>}
 */
const listUserDirectory = async () => {
	return await apiRequest('/users/directory', 'get', getAuthToken())
}

/////////////////////////////////////////////////////////////
////  AUTH
/////////////////////////////////////////////////////////////

/**
 * @typedef {Object} TokenData
 * @property {string} access_token
 * @property {string} refresh_token
 * @property {string} [email]
 * @property {boolean} [email_verified]
 */

/**
 * @typedef {Object} AuthMe
 * @property {string} email
 * @property {boolean} email_verified
 * @property {boolean} [is_superuser]
 */

/**
 *
 * @param {string} email
 * @param {string} password
 * @returns {Promise<TokenData>}
 */
const login = async (email, password) => {
	return await apiRequest('/auth/login', 'post', undefined, {
		email,
		password,
	})
}

/**
 *
 * @param {string} refresh_token
 * @returns {Promise<TokenData>}
 */
const refresh = async (refresh_token) => {
	return await apiRequest('/auth/refresh', 'post', undefined, {
		refresh_token,
	})
}

/**
 * Revoke every token issued for this account server-side. Best effort: the
 * local session is cleared either way, but without this the old refresh token
 * would stay usable until it expires.
 * @returns {Promise<void>}
 */
const logout = async () => {
	try {
		await apiRequest('/auth/logout', 'post', getAuthToken())
	} catch {
		// Offline or already-invalid token: nothing left to revoke.
	}
}

/**
 * @returns {Promise<AuthMe>}
 */
const me = async () => {
	return await apiRequest('/auth/me', 'get', getAuthToken())
}

/**
 * Soft-fail variant for app shell. No toast on missing endpoint.
 * @returns {Promise<AuthMe|null>}
 */
const meSilent = async () => {
	try {
		return await apiRequest(
			'/auth/me',
			'get',
			getAuthToken(),
			undefined,
			false,
			false,
			true,
		)
	} catch {
		return null
	}
}

/////////////////////////////////////////////////////////////
////  STORAGES
/////////////////////////////////////////////////////////////

/**
 * @typedef {Object} Storage
 * @property {string} id
 * @property {string} name
 * @property {number} primary_position
 */

/**
 * @typedef {Object} StorageWithInfoProperties
 * @property {number} size
 * @property {number} files_amount
 * @property {boolean} has_dead_channel
 * @typedef {Storage & StorageWithInfoProperties} StorageWithInfo
 */

/**
 * @typedef {Object} StoragesSchema
 * @property {StorageWithInfo[]} storages
 */

/**
 *
 * @returns {Promise<StoragesSchema>}
 */
const listStorages = async () => {
	return await apiRequest('/storages', 'get', getAuthToken())
}

/**
 * @param {string} id
 * @returns {Promise<Storage>}
 */
const getStorage = async (id) => {
	return await apiRequest(`/storages/${id}`, 'get', getAuthToken())
}

/**
 * @typedef {'active' | 'dead'} ChannelStatus
 */

/**
 * @typedef {Object} StorageChannel
 * @property {string} id
 * @property {number} position
 * @property {number} chat_id
 * @property {string} name
 * @property {ChannelStatus} status
 */

/**
 * @typedef {Object} ReplicationStats
 * @property {number} pending
 * @property {number} uploaded
 * @property {number} failed
 */

/**
 * @typedef {Object} StorageBot
 * @property {string} id
 * @property {string} name
 * @property {string} token_masked
 */

/**
 * @typedef {Object} StorageDetailProperties
 * @property {boolean} has_dead_channel
 * @property {StorageChannel[]} channels
 * @property {ReplicationStats} replication
 * @property {StorageBot | null} [bot]
 * @typedef {Storage & StorageDetailProperties} StorageDetail
 */

/**
 * Full storage detail: channels + replication stats, used by the settings modal.
 * @param {string} id
 * @returns {Promise<StorageDetail>}
 */
const getStorageDetail = getStorage

/**
 * @param {string} storageId
 * @param {number} chatId
 * @param {string} [name]
 * @returns {Promise<StorageChannel>}
 */
const addChannel = async (storageId, chatId, name) => {
	return await apiRequest(
		`/storages/${storageId}/channels`,
		'post',
		getAuthToken(),
		{ chat_id: chatId, ...(name ? { name } : {}) },
	)
}

/**
 * Discover admin chats for this storage's bot and add missing ones (add-only, max 3).
 * @param {string} storageId
 * @returns {Promise<{
 *   added: StorageChannel[],
 *   skipped_full: boolean,
 *   skipped_in_use: number[],
 *   channels: StorageChannel[],
 *   hint?: string | null
 * }>}
 */
const refreshChannels = async (storageId) => {
	return await apiRequest(
		`/storages/${storageId}/channels/refresh`,
		'post',
		getAuthToken(),
	)
}

/**
 * Attach or replace the Telegram bot for this storage. When `removeChannels`
 * is set the server also drops the old bot's channels (confirmed by the user).
 * @param {string} storageId
 * @param {string} token
 * @param {boolean} [removeChannels]
 * @returns {Promise<StorageBot>}
 */
const setStorageBot = async (storageId, token, removeChannels) => {
	return await apiRequest(`/storages/${storageId}/bot`, 'put', getAuthToken(), {
		token,
		...(removeChannels ? { remove_channels: true } : {}),
	})
}

/**
 * @param {string} storageId
 * @param {string} channelId
 * @param {{ chat_id?: number, name?: string }} patch
 * @returns {Promise<StorageChannel>}
 */
const updateChannel = async (storageId, channelId, patch) => {
	return await apiRequest(
		`/storages/${storageId}/channels/${channelId}`,
		'put',
		getAuthToken(),
		patch,
	)
}

/**
 * @param {string} storageId
 * @param {string} channelId
 */
const removeChannel = async (storageId, channelId) => {
	await apiRequest(
		`/storages/${storageId}/channels/${channelId}`,
		'delete',
		getAuthToken(),
	)
}

/**
 * Move failed replicas back to pending so the replication worker retries them.
 * @param {string} storageId
 * @returns {Promise<ReplicationStats | void>}
 */
const retryReplication = async (storageId) => {
	return await apiRequest(
		`/storages/${storageId}/replication/retry`,
		'post',
		getAuthToken(),
	)
}

/**
 * @param {string} id
 * @param {string} name
 * @returns {Promise<Storage>}
 */
const renameStorage = async (id, name) => {
	return await apiRequest(`/storages/${id}`, 'put', getAuthToken(), { name })
}

/**
 * @param {string} id
 */
const deleteStorage = async (id) => {
	await apiRequest(`/storages/${id}`, 'delete', getAuthToken())
}

/////////////////////////////////////////////////////////////
////  ACCESS
/////////////////////////////////////////////////////////////

/**
 * @typedef {'R' | 'W' | 'A'} AccessType
 */

/**
 * @typedef {Object} UserWithAccess
 * @property {string} id
 * @property {string} email
 * @property {AccessType} access_type
 */

/**
 *
 * @param {string} storageID
 * @param {string} email
 * @param {AccessType} accessType
 * @returns
 */
const grantAccess = async (storageID, email, accessType) => {
	return await apiRequest(
		`/storages/${storageID}/access`,
		'post',
		getAuthToken(),
		{ user_email: email, access_type: accessType },
	)
}

/**
 *
 * @param {string} storageID
 * @returns {Promise<UserWithAccess[]>}
 */
const listUsersWithAccess = async (storageID) => {
	return await apiRequest(
		`/storages/${storageID}/access`,
		'get',
		getAuthToken(),
	)
}

/**
 *
 * @param {string} storageID
 * @param {string} userID
 * @returns
 */
const restrictAccess = async (storageID, userID) => {
	return await apiRequest(
		`/storages/${storageID}/access`,
		'delete',
		getAuthToken(),
		{ user_id: userID },
	)
}

/////////////////////////////////////////////////////////////
////  STORAGE WORKERS
/////////////////////////////////////////////////////////////

/**
 * @typedef {Object} StorageWorker
 * @property {string} id
 * @property {string} name
 * @property {number} storage_id
 * @property {number} token
 */

/**
 *
 * @param {string} name
 * @param {string} token
 * @param {string | null | undefined} storage_id
 * @returns {Promise<StorageWorker>}
 */
const createStorageWorker = async (name, token, storage_id) => {
	return await apiRequest('/storage_workers', 'post', getAuthToken(), {
		name,
		token,
		storage_id,
	})
}

/**
 *
 * @returns {Promise<StorageWorker[]>}
 */
const listStorageWorkers = async () => {
	return await apiRequest('/storage_workers', 'get', getAuthToken())
}

/**
 *
 * @param {string} id
 */
const deleteStorageWorker = async (id) => {
	await apiRequest(`/storage_workers/${id}`, 'delete', getAuthToken())
}

/////////////////////////////////////////////////////////////
////  FILES
/////////////////////////////////////////////////////////////

/**
 *
 * @param {string} storage_id
 * @param {string} path
 * @param {string} folderName
 * @returns
 */
const createFolder = async (storage_id, path, folderName) => {
	return await apiRequest(
		`/storages/${storage_id}/files/create_folder`,
		'post',
		getAuthToken(),
		{ path, folder_name: folderName },
	)
}

/**
 *
 * @param {string} storage_id
 * @param {string} path
 * @param {File|Blob} file
 * @param {(progress: import('./request').UploadProgressEvent) => void} [onProgress]
 * @param {{ silent?: boolean, signal?: AbortSignal }} [options]
 * @returns
 */
const uploadFile = async (storage_id, path, file, onProgress, options = {}) => {
	const form = new FormData()
	const basename = String(file?.name || 'unnamed')
		.split(/[/\\]/)
		.pop()
		.trim() || 'unnamed'
	form.append('path', path ?? '')
	form.append('filename', basename)
	const lastModified = Number(file?.lastModified)
	if (Number.isFinite(lastModified) && lastModified > 0) {
		form.append('mtime', String(Math.round(lastModified)))
		// Browser File API has no birthtime; lastModified is the best metadata we get.
		form.append('created', String(Math.round(lastModified)))
	}
	form.append('file', file, basename)

	// Build the grid thumbnail here, from the picture already in memory, and ship
	// it with the upload. The server stores it as-is: it never decodes the photo,
	// and this client never downloads back a tile it just made.
	const thumb = await makeThumbBlob(file)
	if (thumb) form.append('thumb', thumb, 'thumb.jpg')

	const result = await apiMultipartRequest(
		`/storages/${storage_id}/files/upload`,
		getAuthToken(),
		form,
		onProgress,
		options,
	)

	if (thumb) {
		const logicalPath = [String(path ?? '').replace(/\/+$/, ''), basename]
			.filter(Boolean)
			.join('/')
		// Nothing to invalidate on a miss: an entry under a path the server
		// renamed (conflict suffix) is simply never read.
		putCachedThumb(storage_id, logicalPath, thumb).catch(() => {})
	}

	return result
}

/**
 * @typedef {Object} FSElement
 * @property {string} path
 * @property {string} name
 * @property {boolean} is_file
 * @property {number} size
 * @property {boolean} has_thumb
 * @property {string|number} [mtime]
 * @property {string|number} [modified_at]
 * @property {string|number} [updated_at]
 * @property {string|number} [date_modified]
 * @property {boolean} [is_favorite]
 */

/**
 *
 * @param {string} storage_id
 * @param {string} path
 * @returns {Promise<FSElement[]>}
 */
const getFSLayer = async (storage_id, path) => {
	const suffix = path ? encodeFilePath(path) : ''
	return await apiRequest(
		`/storages/${storage_id}/files/tree/${suffix}`,
		'get',
		getAuthToken(),
	)
}

/**
 * @typedef {Object} FileInfo
 * @property {string} path
 * @property {string} name
 * @property {number} size
 * @property {boolean} is_file
 * @property {boolean} has_thumb
 * @property {boolean} is_uploaded
 * @property {string|null} [content_type]
 * @property {string|null} [deleted_at]
 * @property {string|null} [added_at]
 * @property {string|null} [created_at]
 * @property {string|null} [modified_at]
 */

/**
 * @param {string} storage_id
 * @param {string} path
 * @returns {Promise<FileInfo>}
 */
const getFileInfo = async (storage_id, path) => {
	const suffix = path ? encodeFilePath(path) : ''
	return await apiRequest(
		`/storages/${storage_id}/files/info/${suffix}`,
		'get',
		getAuthToken(),
	)
}

/**
 *
 * @param {string} storage_id
 * @param {string} path
 * @returns {Promise<Blob>}
 */
const download = async (storage_id, path) => {
	const response = await apiRequest(
		`/storages/${storage_id}/files/download/${encodeFilePath(path)}`,
		'get',
		getAuthToken(),
		undefined,
		true,
	)

	return await response.blob()
}

/**
 * @typedef {Object} DownloadProgress
 * @property {number} received Bytes received so far.
 * @property {number | null} total Bytes total, or `null` when the response
 *   never revealed one — a folder ZIP has no `Content-Length` until the
 *   server finishes building it.
 * @property {number | null} percent 0-100, or `null` when `total` is `null`.
 */

/**
 * Pump a streamed `Response` body, reporting progress as bytes arrive, and
 * resolve with the assembled Blob. Falls back to a plain `response.blob()`
 * when the runtime has no streaming body reader (no progress, same result).
 * @param {Response} response
 * @param {(progress: DownloadProgress) => void} [onProgress] Throttled to
 *   at most once per animation frame — a raw per-chunk callback would
 *   otherwise fire far faster than the UI can usefully redraw.
 * @returns {Promise<Blob>}
 */
const consumeResponseWithProgress = async (response, onProgress) => {
	const reader = response.body?.getReader?.()
	if (!reader) {
		return await response.blob()
	}

	const contentLength = Number(response.headers.get('Content-Length'))
	const total =
		Number.isFinite(contentLength) && contentLength > 0 ? contentLength : null
	const contentType = response.headers.get('Content-Type') || undefined

	const batcher = onProgress ? createRafBatcher(onProgress) : null
	/** @type {Uint8Array[]} */
	const chunks = []
	let received = 0

	try {
		while (true) {
			const { done, value } = await reader.read()
			if (done) break
			if (!value) continue
			chunks.push(value)
			received += value.byteLength
			batcher?.schedule({
				received,
				total,
				percent: total ? Math.min(100, (received / total) * 100) : null,
			})
		}
	} finally {
		// Drop any frame still pending — a stale event after the blob is
		// already handed back would report progress for a download that,
		// as far as the caller is concerned, no longer exists.
		batcher?.cancel()
	}

	return new Blob(chunks, contentType ? { type: contentType } : undefined)
}

/**
 * Parallel-range fetch (see {@link downloadFileRanged}) only kicks in above
 * this size — small files aren't worth the extra round trips.
 */
const PARALLEL_DOWNLOAD_THRESHOLD_BYTES = 8 * 1024 * 1024

/**
 * How many concurrent `Range` requests one file download fans out to. The
 * server's chunk cache is keyed per Telegram chunk, not per byte range (see
 * `ChunkCache`/`SingleFlight` in `files.rs`), so distinct Range windows land
 * on distinct chunks and are pulled from Telegram at the same time instead
 * of the single-chunk-ahead prefetch a lone sequential GET gets.
 */
const PARALLEL_DOWNLOAD_PARTS = 4

/**
 * Fetch one byte range, accumulating it into a single `Uint8Array` while
 * reporting each chunk's size to `onChunk` as it arrives.
 * @param {string} url
 * @param {string} token
 * @param {number} start
 * @param {number} end
 * @param {(bytes: number) => void} [onChunk]
 * @param {AbortSignal} [signal]
 * @returns {Promise<Uint8Array>}
 */
const fetchRangePart = async (url, token, start, end, onChunk, signal) => {
	const response = await fetch(url, {
		headers: { Authorization: `Bearer ${token}`, Range: `bytes=${start}-${end}` },
		signal,
	})
	if (!response.ok && response.status !== 206) {
		throw new Error(`range fetch failed: ${response.status}`)
	}
	const reader = response.body?.getReader?.()
	if (!reader) {
		const buffer = new Uint8Array(await response.arrayBuffer())
		onChunk?.(buffer.byteLength)
		return buffer
	}
	/** @type {Uint8Array[]} */
	const parts = []
	let length = 0
	while (true) {
		const { done, value } = await reader.read()
		if (done) break
		if (!value) continue
		parts.push(value)
		length += value.byteLength
		onChunk?.(value.byteLength)
	}
	const buffer = new Uint8Array(length)
	let offset = 0
	for (const part of parts) {
		buffer.set(part, offset)
		offset += part.byteLength
	}
	return buffer
}

/**
 * Downloads a single file as several concurrent byte-range requests and
 * reassembles them client-side (in order, by plain `Blob` concatenation —
 * these are raw byte ranges of the original file, not a media container, so
 * no decode/mux step is involved) instead of one sequential stream.
 *
 * Throws — deliberately, so the caller falls back to the proven single-
 * stream path — when the size can't be determined, the server ignored
 * `Range` (a proxy stripped it, or the file is empty), or the file is too
 * small to be worth splitting.
 * @param {string} storage_id
 * @param {string} path
 * @param {(progress: DownloadProgress) => void} [onProgress]
 * @param {AbortSignal} [signal]
 * @returns {Promise<Blob>}
 */
const downloadFileRanged = async (storage_id, path, onProgress, signal) => {
	const token = await getFreshAccessToken()
	if (!token) throw new Error('no access token')
	const url = `${API_BASE}/storages/${storage_id}/files/download/${encodeFilePath(path)}`

	// A 1-byte probe reveals the size via `Content-Range` without committing
	// to a full-body GET first.
	const probe = await fetch(url, {
		headers: { Authorization: `Bearer ${token}`, Range: 'bytes=0-0' },
		signal,
	})
	if (!probe.ok && probe.status !== 206) {
		throw new Error(`probe failed: ${probe.status}`)
	}
	if (probe.status !== 206) {
		// Range was ignored (or the file is empty) — this response already
		// carries the whole body, so just consume it.
		return await consumeResponseWithProgress(probe, onProgress)
	}

	const total = Number((probe.headers.get('Content-Range') || '').split('/').pop())
	if (!Number.isFinite(total) || total < PARALLEL_DOWNLOAD_THRESHOLD_BYTES) {
		throw new Error('file too small for parallel download')
	}
	const contentType = probe.headers.get('Content-Type') || undefined

	const partSize = Math.ceil(total / PARALLEL_DOWNLOAD_PARTS)
	const ranges = []
	for (let i = 0; i < PARALLEL_DOWNLOAD_PARTS; i++) {
		const start = i * partSize
		const end = Math.min(start + partSize, total) - 1
		if (start <= end) ranges.push([start, end])
	}

	const batcher = onProgress ? createRafBatcher(onProgress) : null
	let received = 0
	const onChunk = (n) => {
		received += n
		batcher?.schedule({
			received,
			total,
			percent: Math.min(100, (received / total) * 100),
		})
	}

	try {
		const buffers = await Promise.all(
			ranges.map(([start, end]) => fetchRangePart(url, token, start, end, onChunk, signal)),
		)
		return new Blob(buffers, contentType ? { type: contentType } : undefined)
	} finally {
		batcher?.cancel()
	}
}

/**
 * Same download as `download`, but reports progress as bytes arrive instead
 * of buffering silently behind `response.blob()` — the difference between an
 * indeterminate spinner and a real bar on a multi-GB file.
 *
 * For a single file (not a folder ZIP), tries {@link downloadFileRanged}
 * first — parallel byte-range requests reassembled client-side — and falls
 * back to the plain sequential stream below on any failure (proxy strips
 * `Range`, network hiccup, file too small to bother, etc.).
 * @param {string} storage_id
 * @param {string} path
 * @param {(progress: DownloadProgress) => void} [onProgress]
 * @param {AbortSignal} [signal]
 * @returns {Promise<Blob>}
 */
const downloadWithProgress = async (storage_id, path, onProgress, signal) => {
	const isFolder = path.endsWith('/')
	if (!isFolder) {
		try {
			return await downloadFileRanged(storage_id, path, onProgress, signal)
		} catch (err) {
			if (err?.name === 'AbortError' || signal?.aborted) throw err
			// Fall through to the sequential path below.
		}
	}

	const response = await apiRequest(
		`/storages/${storage_id}/files/download/${encodeFilePath(path)}`,
		'get',
		getAuthToken(),
		undefined,
		true,
		false,
		false,
		signal,
	)
	return await consumeResponseWithProgress(response, onProgress)
}

/**
 * Encode each path segment for use in a files API URL.
 * Preserves a trailing slash so folder downloads hit the ZIP path.
 * @param {string} path
 */
const encodeFilePath = (path) => {
	const raw = String(path || '')
	const trailing = raw.endsWith('/')
	const encoded = raw
		.split('/')
		.filter((p) => p.length)
		.map(encodeURIComponent)
		.join('/')
	return trailing && encoded ? `${encoded}/` : encoded
}

/**
 * Authenticated URL for `<video>` / `<audio>` / `<img>` / `<iframe>` streaming.
 * Uses `?access_token=` so the browser can send Range requests without a custom fetch.
 *
 * Refreshes the access token first if it looks expired — these URLs are
 * handed straight to the browser, which can't retry through apiRequest's
 * 401 handling the way JSON calls do.
 *
 * @param {string} storage_id
 * @param {string} path
 * @returns {Promise<string>}
 */
const getInlineMediaUrl = async (storage_id, path) => {
	const token = await getFreshAccessToken()
	const params = new URLSearchParams({
		inline: '1',
		access_token: token || '',
	})
	return `${API_BASE}/storages/${storage_id}/files/download/${encodeFilePath(path)}?${params}`
}

/**
 * Authenticated URL for image preview JPEG (FileViewer).
 * Refreshes the access token first if it looks expired (see {@link getInlineMediaUrl}).
 * @param {string} storage_id
 * @param {string} path
 * @returns {Promise<string>}
 */
const getPreviewUrl = async (storage_id, path) => {
	const token = await getFreshAccessToken()
	const params = new URLSearchParams({
		access_token: token || '',
	})
	return `${API_BASE}/storages/${storage_id}/files/preview/${encodeFilePath(path)}?${params}`
}

/**
 *
 * @param {string} storage_id
 * @param {string} path
 * @param {AbortSignal} [signal]
 * @returns {Promise<Blob>}
 */
const thumb = async (storage_id, path, signal) => {
	// silent: true — a busy storage returns 503 while thumbQueue retries with
	// backoff; that is not a user-facing error, so it must not trigger a toast.
	const response = await apiRequest(
		`/storages/${storage_id}/files/thumb/${encodeFilePath(path)}`,
		'get',
		getAuthToken(),
		undefined,
		true,
		false,
		true,
		signal,
	)

	return await response.blob()
}

/**
 *
 * @param {string} storage_id
 * @param {string} path
 */
const deleteFile = async (storage_id, path) => {
	await apiRequest(
		`/storages/${storage_id}/files/${encodeFilePath(path)}`,
		'delete',
		getAuthToken(),
	)
}

/**
 * @param {string} storage_id
 * @param {string} [path]
 * @returns {Promise<import("./index").FSElement[]>}
 */
const listTrash = async (storage_id, path = '') => {
	const params = new URLSearchParams()
	if (path) params.set('path', path)
	const qs = params.toString()
	return await apiRequest(
		`/storages/${storage_id}/trash${qs ? `?${qs}` : ''}`,
		'get',
		getAuthToken(),
	)
}

/**
 * @param {string} storage_id
 * @param {string} path
 * @param {'replace' | 'rename'} [on_conflict]
 */
const restoreTrash = async (storage_id, path, on_conflict) => {
	const body = { path }
	if (on_conflict) body.on_conflict = on_conflict
	try {
		await apiRequest(
			`/storages/${storage_id}/trash/restore`,
			'post',
			getAuthToken(),
			body,
			false,
			false,
			true,
		)
	} catch (err) {
		// 409 without on_conflict is handled by the restore-conflict dialog.
		if (err.status === 409 && !on_conflict) {
			throw err
		}
		alertStore.addAlert(err.message, 'error')
		throw err
	}
}

/**
 * @param {string} storage_id
 * @param {string} path
 */
const deleteForever = async (storage_id, path) => {
	await apiRequest(
		`/storages/${storage_id}/trash/${encodeFilePath(path)}`,
		'delete',
		getAuthToken(),
	)
}

/**
 * @param {string} storage_id
 */
const emptyTrash = async (storage_id) => {
	await apiRequest(`/storages/${storage_id}/trash`, 'delete', getAuthToken())
}

/**
 * @returns {Promise<{ retention_days: number }>}
 */
const getTrashSettings = async () => {
	return await apiRequest('/settings/trash', 'get', getAuthToken())
}

/**
 * @param {number} retention_days
 * @returns {Promise<{ retention_days: number }>}
 */
const setTrashSettings = async (retention_days) => {
	return await apiRequest('/settings/trash', 'put', getAuthToken(), {
		retention_days,
	})
}

/**
 * Download a backup of the metadata database — settings, storages and their
 * bots, and the whole file tree. Superuser only.
 *
 * POST, not GET: the password rides in the body so it never lands in a URL,
 * a proxy log or browser history.
 *
 * @param {string} [password] Optional. Without one the archive is plain gzip
 *   and anyone holding the file can read every bot token in it.
 * @returns {Promise<{ blob: Blob, filename: string }>}
 */
const createBackup = async (password) => {
	const response = await apiRequest(
		'/settings/backup',
		'post',
		getAuthToken(),
		{ password: password || null },
		true,
	)
	const disposition = response.headers.get('Content-Disposition') || ''
	const match = /filename="?([^";]+)"?/.exec(disposition)
	return {
		blob: await response.blob(),
		filename: match?.[1] || 'sarca-backup.sarcabak',
	}
}

/**
 * @typedef {Object} RestoreResult
 * @property {number} tables Tables copied out of the archive.
 * @property {number} rows Rows written across those tables.
 * @property {string[]} skipped_tables Tables this server has no place for.
 * @property {string | null} safety_copy Server-side path of the pre-restore
 *   copy of the database that was replaced.
 */

/**
 * Replace this server's database with an uploaded backup. Destructive, and it
 * invalidates the current session — the caller must send the user back to login.
 *
 * Talks to `fetch` directly rather than through `apiRequest`: multipart needs
 * the browser to set its own boundary, which a forced JSON content type breaks.
 *
 * @param {File | Blob} file
 * @param {string} [password]
 * @returns {Promise<RestoreResult>}
 */
const restoreBackup = async (file, password) => {
	const form = new FormData()
	form.append('file', file, /** @type {File} */ (file)?.name || 'backup.sarcabak')
	if (password) form.append('password', password)

	const headers = new Headers()
	const token = await getFreshAccessToken()
	if (token) headers.append('Authorization', `Bearer ${token}`)

	const response = await fetch(`${API_BASE}/settings/restore`, {
		method: 'POST',
		headers,
		body: form,
	})

	if (!response.ok) {
		const text = await response.text().catch(() => '')
		const err = new Error(text || 'Restore failed')
		err.status = response.status
		throw err
	}

	return await response.json()
}

/////////////////////////////////////////////////////////////
////  FAVORITES
/////////////////////////////////////////////////////////////

/**
 * @param {string} storage_id
 * @param {{ quiet?: boolean }} [options] When quiet, skip toast (e.g. background path sync)
 * @returns {Promise<import("./index").FSElement[]>}
 */
const listFavorites = async (storage_id, options = {}) => {
	try {
		return await apiRequest(
			`/storages/${storage_id}/favorites`,
			'get',
			getAuthToken(),
			undefined,
			false,
			false,
			true,
		)
	} catch (err) {
		if (!options.quiet) {
			const msg =
				err.status === 404
					? 'Favorites are not available on this server yet'
					: err.message || 'Failed to load favorites'
			alertStore.addAlert(msg, 'error')
		}
		throw err
	}
}

/**
 * Star a file (idempotent). Files only — not folders.
 * @param {string} storage_id
 * @param {string} path
 */
const addFavorite = async (storage_id, path) => {
	try {
		await apiRequest(
			`/storages/${storage_id}/favorites`,
			'put',
			getAuthToken(),
			{ path },
			false,
			false,
			true,
		)
	} catch (err) {
		const msg =
			err.status === 404
				? 'Favorites are not available on this server yet'
				: err.message || 'Failed to star file'
		alertStore.addAlert(msg, 'error')
		throw err
	}
}

/**
 * Unstar a file.
 * @param {string} storage_id
 * @param {string} path
 */
const removeFavorite = async (storage_id, path) => {
	try {
		await apiRequest(
			`/storages/${storage_id}/favorites/${encodeFilePath(path)}`,
			'delete',
			getAuthToken(),
			undefined,
			false,
			false,
			true,
		)
	} catch (err) {
		const msg =
			err.status === 404
				? 'Favorites are not available on this server yet'
				: err.message || 'Failed to unstar file'
		alertStore.addAlert(msg, 'error')
		throw err
	}
}

/////////////////////////////////////////////////////////////
////  RECENT
/////////////////////////////////////////////////////////////

/**
 * @param {string} storage_id
 * @returns {Promise<import("./index").FSElement[]>}
 */
const listRecent = async (storage_id) => {
	try {
		return await apiRequest(
			`/storages/${storage_id}/recent`,
			'get',
			getAuthToken(),
			undefined,
			false,
			false,
			true,
		)
	} catch (err) {
		const msg =
			err.status === 404
				? 'Recent files are not available on this server yet'
				: err.message || 'Failed to load recent files'
		alertStore.addAlert(msg, 'error')
		throw err
	}
}

/**
 * Record a preview open (fire-and-forget friendly). Ignores errors for UX.
 * @param {string} storage_id
 * @param {string} path
 */
const recordRecent = async (storage_id, path) => {
	try {
		await apiRequest(
			`/storages/${storage_id}/recent`,
			'post',
			getAuthToken(),
			{ path },
			false,
			false,
			true,
		)
	} catch {
		/* ignore — preview UX must not depend on recent tracking */
	}
}

/////////////////////////////////////////////////////////////
////  SHARE LINKS (authenticated)
/////////////////////////////////////////////////////////////

/**
 * @typedef {Object} ShareLink
 * @property {string} id
 * @property {string} token
 * @property {string} url_path
 * @property {string} path
 * @property {string|null} expires_at
 * @property {boolean} has_password
 * @property {string} created_at
 */

/**
 * Absolute guest URL for a share token.
 * @param {string} token
 * @param {string} [urlPath] From API (`/s/...`); falls back to `/s/{token}`
 */
const shareAbsoluteUrl = (token, urlPath) => {
	const path =
		urlPath && String(urlPath).startsWith('/')
			? urlPath
			: `/s/${encodeURIComponent(token)}`
	return `${window.location.origin}${path}`
}

/**
 * @param {string} storage_id
 * @param {{ path: string, expires_at?: string|null, password?: string|null }} body
 * @returns {Promise<ShareLink>}
 */
const createShare = async (storage_id, body) => {
	try {
		return await apiRequest(
			`/storages/${storage_id}/shares`,
			'post',
			getAuthToken(),
			body,
			false,
			false,
			true,
		)
	} catch (err) {
		const msg =
			err.status === 404
				? 'Share links are not available on this server yet'
				: err.message || 'Failed to create share link'
		alertStore.addAlert(msg, 'error')
		throw err
	}
}

/**
 * @param {string} storage_id
 * @returns {Promise<ShareLink[]>}
 */
const listShares = async (storage_id) => {
	try {
		const data = await apiRequest(
			`/storages/${storage_id}/shares`,
			'get',
			getAuthToken(),
			undefined,
			false,
			false,
			true,
		)
		return Array.isArray(data)
			? data.filter((s) => !s.revoked_at)
			: (data?.shares || []).filter((s) => !s.revoked_at)
	} catch (err) {
		const msg =
			err.status === 404
				? 'Share links are not available on this server yet'
				: err.message || 'Failed to list share links'
		alertStore.addAlert(msg, 'error')
		throw err
	}
}

/**
 * @param {string} storage_id
 * @param {string} share_id
 */
const revokeShare = async (storage_id, share_id) => {
	try {
		await apiRequest(
			`/storages/${storage_id}/shares/${share_id}`,
			'delete',
			getAuthToken(),
			undefined,
			false,
			false,
			true,
		)
	} catch (err) {
		const msg =
			err.status === 404
				? 'Share links are not available on this server yet'
				: err.message || 'Failed to revoke share link'
		alertStore.addAlert(msg, 'error')
		throw err
	}
}

/////////////////////////////////////////////////////////////
////  PUBLIC SHARES (no JWT; cookies for unlock)
/////////////////////////////////////////////////////////////

/**
 * @typedef {Object} PublicShareMeta
 * @property {string} path
 * @property {string} name
 * @property {boolean} is_file
 * @property {number} [size]
 * @property {boolean} has_password
 */

/**
 * Encode a relative path under a public share (preserves trailing /).
 * @param {string} path
 */
const encodeShareRelPath = (path) => {
	const raw = String(path || '')
	const trailing = raw.endsWith('/')
	const encoded = raw
		.split('/')
		.filter((p) => p.length)
		.map(encodeURIComponent)
		.join('/')
	return trailing && encoded ? `${encoded}/` : encoded
}

/**
 * Public share file URL path. Empty relPath must NOT end with `/` —
 * Axum maps `/download` and `/download/` differently (`/` → 404).
 * @param {string} token
 * @param {'download' | 'inline' | 'thumb' | 'preview'} kind
 * @param {string} [relPath]
 */
const publicShareFilePath = (token, kind, relPath = '') => {
	const base = `/public/shares/${encodeURIComponent(token)}/${kind}`
	const suffix = encodeShareRelPath(relPath)
	return suffix ? `${base}/${suffix}` : base
}

/**
 * @param {string} token
 * @returns {Promise<PublicShareMeta>}
 */
const getPublicShare = async (token) => {
	return await publicApiRequest(
		`/public/shares/${encodeURIComponent(token)}`,
		'get',
		undefined,
		false,
		true,
	)
}

/**
 * @param {string} token
 * @param {string} password
 */
const unlockPublicShare = async (token, password) => {
	return await publicApiRequest(
		`/public/shares/${encodeURIComponent(token)}/unlock`,
		'post',
		{ password },
		false,
		true,
	)
}

/**
 * @param {string} token
 * @param {string} [relPath] Relative to share root
 * @returns {Promise<import("./index").FSElement[]>}
 */
const getPublicShareTree = async (token, relPath = '') => {
	const params = new URLSearchParams()
	if (relPath) params.set('path', relPath)
	const qs = params.toString()
	return await publicApiRequest(
		`/public/shares/${encodeURIComponent(token)}/tree${qs ? `?${qs}` : ''}`,
		'get',
		undefined,
		false,
		true,
	)
}

/**
 * @param {string} token
 * @param {string} [relPath]
 * @returns {Promise<Blob>}
 */
const downloadPublicShare = async (token, relPath = '') => {
	const response = await publicApiRequest(
		publicShareFilePath(token, 'download', relPath),
		'get',
		undefined,
		true,
		true,
	)
	return await response.blob()
}

/**
 * @param {string} token
 * @param {string} [relPath]
 * @param {(progress: DownloadProgress) => void} [onProgress]
 * @param {AbortSignal} [signal]
 * @returns {Promise<Blob>}
 */
const downloadPublicShareWithProgress = async (
	token,
	relPath = '',
	onProgress,
	signal,
) => {
	const response = await publicApiRequest(
		publicShareFilePath(token, 'download', relPath),
		'get',
		undefined,
		true,
		true,
		signal,
	)
	return await consumeResponseWithProgress(response, onProgress)
}

/**
 * @param {string} token
 * @returns {Promise<Blob>}
 */
const downloadPublicShareZip = async (token) => {
	const response = await publicApiRequest(
		`/public/shares/${encodeURIComponent(token)}/download_zip`,
		'get',
		undefined,
		true,
		true,
	)
	return await response.blob()
}

/**
 * A folder ZIP has no `Content-Length` until the server finishes building
 * it — `onProgress` keeps reporting `total: null` (stay on the indeterminate
 * "preparing" copy) until bytes actually start arriving.
 * @param {string} token
 * @param {(progress: DownloadProgress) => void} [onProgress]
 * @param {AbortSignal} [signal]
 * @returns {Promise<Blob>}
 */
const downloadPublicShareZipWithProgress = async (token, onProgress, signal) => {
	const response = await publicApiRequest(
		`/public/shares/${encodeURIComponent(token)}/download_zip`,
		'get',
		undefined,
		true,
		true,
		signal,
	)
	return await consumeResponseWithProgress(response, onProgress)
}

/**
 * @param {string} token
 * @param {string} relPath
 * @param {AbortSignal} [signal]
 * @returns {Promise<Blob>}
 */
const thumbPublicShare = async (token, relPath, signal) => {
	const response = await publicApiRequest(
		publicShareFilePath(token, 'thumb', relPath),
		'get',
		undefined,
		true,
		true,
		signal,
	)
	return await response.blob()
}

/**
 * Cookie-auth URL for `<video>` / `<img>` / `<iframe>` on a public share.
 * @param {string} token
 * @param {string} [relPath]
 * @returns {string}
 */
const getPublicInlineMediaUrl = (token, relPath = '') => {
	return `${API_BASE}${publicShareFilePath(token, 'inline', relPath)}`
}

/**
 * Cookie-auth preview JPEG URL for images on a public share.
 * @param {string} token
 * @param {string} [relPath]
 * @returns {string}
 */
const getPublicPreviewUrl = (token, relPath = '') => {
	return `${API_BASE}${publicShareFilePath(token, 'preview', relPath)}`
}

/////////////////////////////////////////////////////////////
////  SETUP WIZARD
/////////////////////////////////////////////////////////////

/**
 * @typedef {Object} SetupStatus
 * @property {boolean} has_storages
 * @property {boolean} conf_writable
 */

/** @returns {Promise<SetupStatus>} */
const getSetupStatus = async () => {
	return await apiRequest('/setup/status', 'get', getAuthToken())
}

/**
 * @param {string} token
 * @returns {Promise<{ bot_id: number, username: string, channels?: Array<{ chat_id: number, title: string }> }>}
 */
const validateBot = async (token) => {
	// Backstop: the backend now times out each Telegram call individually, but
	// keep a client-side ceiling too so "Validate bot" never stays stuck forever
	// if something upstream still hangs.
	return await apiRequest(
		'/setup/bot/validate',
		'post',
		getAuthToken(),
		{ token },
		false,
		false,
		false,
		AbortSignal.timeout(60_000),
	)
}

/**
 * @param {string} token
 * @param {number[]} [exclude_chat_ids]
 * @param {number[]} [probe_chat_ids]
 * @returns {Promise<{ channels: Array<{ chat_id: number, title: string }>, hint?: string }>}
 */
const pollChannel = async (token, exclude_chat_ids = [], probe_chat_ids = []) => {
	return await apiRequest(
		'/setup/channel/poll',
		'post',
		getAuthToken(),
		{ token, exclude_chat_ids, probe_chat_ids },
		false,
		false,
		false,
		AbortSignal.timeout(60_000),
	)
}

/**
 * @param {string} name
 * @param {string} token
 * @param {number[]} chat_ids
 * @returns {Promise<{ id: string, name: string }>}
 */
const setupCreateStorage = async (name, token, chat_ids) => {
	return await apiRequest('/setup/storages', 'post', getAuthToken(), {
		name,
		token,
		chat_ids,
	})
}

/**
 *
 * @param {string} storage_id
 * @param {string} path current folder path (may be empty)
 * @param {string} search_path search query
 * @returns {Promise<{path: string, is_file: boolean}[]>}
 */
const search = async (storage_id, path, search_path) => {
	const params = new URLSearchParams({ search_path })
	const encoded = path ? encodeFilePath(path) : ''
	const base = encoded ? `search/${encoded}` : 'search'
	return await apiRequest(
		`/storages/${storage_id}/files/${base}?${params}`,
		'get',
		getAuthToken(),
	)
}

/**
 *
 * @param {string} storage_id
 * @param {string} path
 * @param {string} new_name
 */
const rename = async (storage_id, path, new_name) => {
	await apiRequest(
		`/storages/${storage_id}/files/rename`,
		'post',
		getAuthToken(),
		{ path, new_name },
	)
}

/**
 * @param {string} storage_id
 * @param {string} path
 * @param {string} destination_folder
 * @param {'replace' | 'rename'} [on_conflict]
 */
const moveFile = async (storage_id, path, destination_folder, on_conflict) => {
	const body = { path, destination_folder }
	if (on_conflict) body.on_conflict = on_conflict
	try {
		await apiRequest(
			`/storages/${storage_id}/files/move`,
			'post',
			getAuthToken(),
			body,
			false,
			false,
			true,
		)
	} catch (err) {
		if (err.status === 409 && !on_conflict) {
			throw err
		}
		const msg =
			err.status === 404
				? 'Move is not available on this server yet'
				: err.message || 'Failed to move'
		alertStore.addAlert(msg, 'error')
		throw err
	}
}

/**
 * @param {string} storage_id
 * @param {string} path
 * @param {string} destination_folder
 * @param {'replace' | 'rename'} [on_conflict]
 */
const copyFile = async (storage_id, path, destination_folder, on_conflict) => {
	const body = { path, destination_folder }
	if (on_conflict) body.on_conflict = on_conflict
	try {
		await apiRequest(
			`/storages/${storage_id}/files/copy`,
			'post',
			getAuthToken(),
			body,
			false,
			false,
			true,
		)
	} catch (err) {
		if (err.status === 409 && !on_conflict) {
			throw err
		}
		const msg =
			err.status === 404
				? 'Copy is not available on this server yet'
				: err.message || 'Failed to copy'
		alertStore.addAlert(msg, 'error')
		throw err
	}
}

/////////////////////////////////////////////////////////////
////  SYNC
/////////////////////////////////////////////////////////////

const getSyncChangelog = async (storage_id, cursor = 0, limit = 500) => {
	const q = new URLSearchParams({
		cursor: String(cursor),
		limit: String(limit),
	})
	return await apiRequest(
		`/storages/${storage_id}/sync/changelog?${q}`,
		'get',
		getAuthToken(),
	)
}

const getSyncSnapshot = async (storage_id) => {
	return await apiRequest(
		`/storages/${storage_id}/sync/snapshot`,
		'get',
		getAuthToken(),
	)
}

/////////////////////////////////////////////////////////////
////  API
/////////////////////////////////////////////////////////////

const API = {
	users: {
		createUser,
		listUsers,
		changeMyPassword,
		setUserPassword,
		setUserDisabled,
		listUserDirectory,
	},
	auth: {
		login,
		logout,
		refresh,
		me,
		meSilent,
	},
	storages: {
		listStorages,
		getStorage,
		getStorageDetail,
		renameStorage,
		deleteStorage,
		addChannel,
		refreshChannels,
		setStorageBot,
		updateChannel,
		removeChannel,
		retryReplication,
	},
	access: {
		grantAccess,
		listUsersWithAccess,
		restrictAccess,
	},
	storageWorkers: {
		createStorageWorker,
		listStorageWorkers,
		deleteStorageWorker,
	},
	files: {
		createFolder,
		uploadFile,
		getFSLayer,
		getFileInfo,
		download,
		downloadWithProgress,
		getInlineMediaUrl,
		getPreviewUrl,
		thumb,
		deleteFile,
		listTrash,
		restoreTrash,
		deleteForever,
		emptyTrash,
		listFavorites,
		addFavorite,
		removeFavorite,
		listRecent,
		recordRecent,
		search,
		rename,
		moveFile,
		copyFile,
	},
	sync: {
		getSyncChangelog,
		getSyncSnapshot,
	},
	shares: {
		createShare,
		listShares,
		revokeShare,
		shareAbsoluteUrl,
	},
	publicShares: {
		getPublicShare,
		unlockPublicShare,
		getPublicShareTree,
		downloadPublicShare,
		downloadPublicShareWithProgress,
		downloadPublicShareZip,
		downloadPublicShareZipWithProgress,
		thumbPublicShare,
		getPublicInlineMediaUrl,
		getPublicPreviewUrl,
	},
	settings: {
		getTrashSettings,
		setTrashSettings,
		createBackup,
		restoreBackup,
	},
	setup: {
		getSetupStatus,
		validateBot,
		pollChannel,
		setupCreateStorage,
	},
}

const getAuthToken = () => {
	const [store, _setStore] = createLocalStore()
	return `Bearer ${store.access_token}`
}

export default API
