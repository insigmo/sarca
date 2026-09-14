import createLocalStore from '../../libs'

/**
 * Accounts the login screen was asked to remember.
 *
 * "Remember me" here means the next sign-in is one click: picking a saved
 * account has to fill *both* fields, so the password is stored alongside the
 * email. localStorage is plain text — anyone with the device, or any script
 * running on the origin, can read it — which is why nothing is written unless
 * the user ticks the box, and why forgetting an account is one click too.
 *
 * @typedef {Object} SavedAccount
 * @property {string} email
 * @property {string} password
 */

const KEY = 'saved_accounts'
/** Enough for a shared machine without turning the card into a scroll area. */
const MAX_ACCOUNTS = 5

const [store, setStore] = createLocalStore()

const sameEmail = (a, b) =>
	String(a).trim().toLowerCase() === String(b).trim().toLowerCase()

/**
 * Saved accounts, newest first. Reactive when read inside a tracking scope.
 * @returns {SavedAccount[]}
 */
export const savedAccounts = () => {
	const raw = store[KEY]
	if (!Array.isArray(raw)) return []
	return raw.filter(
		(entry) =>
			entry &&
			typeof entry.email === 'string' &&
			entry.email &&
			typeof entry.password === 'string',
	)
}

/**
 * Remember an account, or refresh the password of one already stored — a
 * password change must not leave the old value behind to fail on next login.
 * @param {string} email
 * @param {string} password
 */
export const rememberAccount = (email, password) => {
	const trimmed = String(email).trim()
	if (!trimmed || !password) return
	const rest = savedAccounts().filter((entry) => !sameEmail(entry.email, trimmed))
	setStore(KEY, [{ email: trimmed, password }, ...rest].slice(0, MAX_ACCOUNTS))
}

/**
 * Drop a stored account. Also used after an unticked sign-in, so unticking the
 * box is how you stop being remembered.
 * @param {string} email
 */
export const forgetAccount = (email) => {
	const trimmed = String(email).trim()
	if (!trimmed) return
	const current = savedAccounts()
	const next = current.filter((entry) => !sameEmail(entry.email, trimmed))
	// Nothing matched: don't write an empty list into storage for every
	// sign-in that left the box unticked.
	if (next.length === current.length) return
	setStore(KEY, next)
}

/**
 * Whether this email is already remembered — drives the checkbox's initial
 * state so re-signing in as a saved account keeps it saved.
 * @param {string} email
 * @returns {boolean}
 */
export const isRemembered = (email) =>
	savedAccounts().some((entry) => sameEmail(entry.email, email))
