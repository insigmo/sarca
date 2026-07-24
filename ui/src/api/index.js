import createLocalStore from '../../libs'

import apiRequest, { apiMultipartRequest, API_BASE } from './request'

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

/////////////////////////////////////////////////////////////
////  STORAGES
/////////////////////////////////////////////////////////////

/**
 *
 * @param {string} name
 * @param {number} chat_id
 * @returns
 */
const createStorage = async (name, chat_id) => {
	return await apiRequest('/storages', 'post', getAuthToken(), {
		name,
		chat_id,
	})
}

/**
 * @typedef {Object} Storage
 * @property {string} id
 * @property {string} name
 * @property {number} chat_id
 */

/**
 * @typedef {Object} StorageWithInfoProperties
 * @property {number} size
 * @property {number} files_amount
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
 *
 * @param {string} storage_id
 * @param {string} path
 * @param {File|Blob} file
 * @param {(progress: number) => void} [onProgress]
 * @param {{ silent?: boolean }} [options]
 * @returns
 */
const uploadFileTo = async (storage_id, path, file, onProgress, options = {}) => {
	const form = new FormData()
	const basename = String(file?.name || 'unnamed')
		.split(/[/\\]/)
		.pop()
		.trim() || 'unnamed'
	form.append('path', path ?? '')
	form.append('filename', basename)
	form.append('file', file, basename)

	return await apiMultipartRequest(
		`/storages/${storage_id}/files/upload_to`,
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
 *
 * @param {string} storage_id
 * @param {string} path
 * @param {string} destination_folder
 */
const moveFile = async (storage_id, path, destination_folder) => {
	await apiRequest(
		`/storages/${storage_id}/files/move`,
		'post',
		getAuthToken(),
		{ path, destination_folder },
	)
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
	},
	storages: {
		createStorage,
		listStorages,
		getStorage,
		renameStorage,
		deleteStorage,
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
		uploadFileTo,
		getFSLayer,
		download,
		getInlineMediaUrl,
		thumb,
		deleteFile,
		search,
		rename,
		moveFile,
	},
}

const getAuthToken = () => {
	const [store, _setStore] = createLocalStore()
	return `Bearer ${store.access_token}`
}

export default API
