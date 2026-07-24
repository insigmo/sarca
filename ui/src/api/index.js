import createLocalStore from '../../libs'

import apiRequest, {
	apiMultipartRequest,
	publicApiRequest,
	API_BASE,
} from './request'
import { alertStore } from '../components/AlertStack'

/////////////////////////////////////////////////////////////
////  USERS
/////////////////////////////////////////////////////////////

/**
 * @typedef {Object} TokenData
 * @property {string} access_token
 */

/**
 *
 * @param {string} email
 * @param {string} password
 * @returns {Promise<any>}
 */
const register = async (email, password) => {
	return await apiRequest('/users', 'post', undefined, {
		email,
		password,
	})
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
 * @typedef {Object} AuthProviders
 * @property {boolean} google
 * @property {boolean} github
 * @property {boolean} [smtp]
 */

/**
 * @typedef {Object} AuthMe
 * @property {string} email
 * @property {boolean} email_verified
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
 * Which OAuth/SMTP features the server has configured.
 * Missing endpoint → all disabled (no toast).
 * @returns {Promise<AuthProviders>}
 */
const getProviders = async () => {
	try {
		const data = await apiRequest(
			'/auth/providers',
			'get',
			undefined,
			undefined,
			false,
			false,
			true,
		)
		return {
			google: !!data?.google,
			github: !!data?.github,
			smtp: !!data?.smtp,
		}
	} catch {
		return { google: false, github: false, smtp: false }
	}
}

/**
 * @returns {Promise<AuthMe>}
 */
const me = async () => {
	return await apiRequest('/auth/me', 'get', getAuthToken())
}

/**
 * Soft-fail variant for app shell (banner). No toast on missing endpoint.
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

/**
 * Resend verification email (auth required).
 * @returns {Promise<void>}
 */
const requestVerify = async () => {
	return await apiRequest('/auth/verify/request', 'post', getAuthToken())
}

/**
 * Consume email verification token.
 * @param {string} token
 * @returns {Promise<void>}
 */
const verifyEmail = async (token) => {
	return await apiRequest('/auth/verify', 'post', undefined, { token })
}

/**
 * Always 204 when backend is present (no email enumeration).
 * @param {string} email
 * @returns {Promise<void>}
 */
const forgotPassword = async (email) => {
	return await apiRequest('/auth/password/forgot', 'post', undefined, {
		email,
	})
}

/**
 * @param {string} token
 * @param {string} new_password
 * @returns {Promise<void>}
 */
const resetPassword = async (token, new_password) => {
	return await apiRequest('/auth/password/reset', 'post', undefined, {
		token,
		new_password,
	})
}

/**
 * Exchange OAuth one-time code for JWTs.
 * @param {string} code
 * @returns {Promise<TokenData>}
 */
const exchangeOAuth = async (code) => {
	return await apiRequest('/auth/oauth/exchange', 'post', undefined, {
		code,
	})
}

/**
 * Browser redirect URL for OAuth start (full navigation).
 * @param {'google'|'github'} provider
 * @returns {string}
 */
const oauthStartUrl = (provider) => `${API_BASE}/auth/oauth/${provider}/start`

/////////////////////////////////////////////////////////////
////  STORAGES
/////////////////////////////////////////////////////////////

/**
 * @typedef {Object} ChannelInput
 * @property {number} chat_id
 * @property {string} [name]
 */

/**
 *
 * @param {string} name
 * @param {ChannelInput[]} channels 1..3 Telegram channels replicating this storage
 * @returns
 */
const createStorage = async (name, channels) => {
	return await apiRequest('/storages', 'post', getAuthToken(), {
		name,
		channels,
	})
}

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
 * @typedef {Object} StorageDetailProperties
 * @property {boolean} has_dead_channel
 * @property {StorageChannel[]} channels
 * @property {ReplicationStats} replication
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
 * @param {(progress: number) => void} [onProgress]
 * @param {{ silent?: boolean }} [options]
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
	form.append('file', file, basename)

	return await apiMultipartRequest(
		`/storages/${storage_id}/files/upload`,
		getAuthToken(),
		form,
		onProgress,
		options,
	)
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
 * @param {string} storage_id
 * @param {string} path
 * @returns {string}
 */
const getInlineMediaUrl = (storage_id, path) => {
	const [store] = createLocalStore()
	const params = new URLSearchParams({
		inline: '1',
		access_token: store.access_token || '',
	})
	return `${API_BASE}/storages/${storage_id}/files/download/${encodeFilePath(path)}?${params}`
}

/**
 *
 * @param {string} storage_id
 * @param {string} path
 * @returns {Promise<Blob>}
 */
const thumb = async (storage_id, path) => {
	const response = await apiRequest(
		`/storages/${storage_id}/files/thumb/${encodeFilePath(path)}`,
		'get',
		getAuthToken(),
		undefined,
		true,
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
		return Array.isArray(data) ? data : data?.shares || []
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
 * @param {'download' | 'inline' | 'thumb'} kind
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
 * @param {string} token
 * @param {string} relPath
 * @returns {Promise<Blob>}
 */
const thumbPublicShare = async (token, relPath) => {
	const response = await publicApiRequest(
		publicShareFilePath(token, 'thumb', relPath),
		'get',
		undefined,
		true,
		true,
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

/////////////////////////////////////////////////////////////
////  SETUP WIZARD
/////////////////////////////////////////////////////////////

/**
 * @typedef {Object} SetupStatus
 * @property {boolean} has_storages
 * @property {boolean} uses_local_api
 * @property {boolean} local_api_ready
 * @property {boolean} local_api_skipped
 * @property {boolean} needs_local_api_phase
 * @property {boolean} conf_writable
 */

/** @returns {Promise<SetupStatus>} */
const getSetupStatus = async () => {
	return await apiRequest('/setup/status', 'get', getAuthToken())
}

/**
 * @param {string} api_id
 * @param {string} api_hash
 */
const saveLocalApi = async (api_id, api_hash) => {
	return await apiRequest('/setup/local-api', 'post', getAuthToken(), {
		api_id,
		api_hash,
	})
}

/** @returns {Promise<{ ok: boolean, uses_local_api: boolean, message: string }>} */
const verifyLocalApi = async () => {
	return await apiRequest('/setup/local-api/verify', 'post', getAuthToken())
}

const skipLocalApi = async () => {
	return await apiRequest('/setup/local-api/skip', 'post', getAuthToken())
}

/**
 * @param {string} token
 * @returns {Promise<{ bot_id: number, username: string }>}
 */
const validateBot = async (token) => {
	return await apiRequest('/setup/bot/validate', 'post', getAuthToken(), {
		token,
	})
}

/**
 * @param {string} token
 * @param {number[]} [exclude_chat_ids]
 * @returns {Promise<{ found: boolean, chat_id?: number, title?: string, hint?: string }>}
 */
const pollChannel = async (token, exclude_chat_ids = []) => {
	return await apiRequest('/setup/channel/poll', 'post', getAuthToken(), {
		token,
		exclude_chat_ids,
	})
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
////  API
/////////////////////////////////////////////////////////////

const API = {
	users: {
		register,
	},
	auth: {
		login,
		refresh,
		getProviders,
		me,
		meSilent,
		requestVerify,
		verifyEmail,
		forgotPassword,
		resetPassword,
		exchangeOAuth,
		oauthStartUrl,
	},
	storages: {
		createStorage,
		listStorages,
		getStorage,
		getStorageDetail,
		renameStorage,
		deleteStorage,
		addChannel,
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
		download,
		getInlineMediaUrl,
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
		downloadPublicShareZip,
		thumbPublicShare,
		getPublicInlineMediaUrl,
	},
	settings: {
		getTrashSettings,
		setTrashSettings,
	},
	setup: {
		getSetupStatus,
		saveLocalApi,
		verifyLocalApi,
		skipLocalApi,
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
