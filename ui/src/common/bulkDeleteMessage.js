import { t } from './i18n'

/**
 * Success-toast text for a bulk delete/delete-forever operation.
 * @param {number} count number of items that actually succeeded
 * @param {string} [singleName] name of the item, used only when count === 1
 * @param {boolean} [permanent] true for trash "delete forever"
 * @returns {string}
 */
export const formatBulkDeleteMessage = (count, singleName, permanent) => {
	if (count === 1) {
		return t(permanent ? 'files.deletedForeverOne' : 'files.deletedOne', {
			name: singleName,
		})
	}
	return t(permanent ? 'files.deletedForeverMany' : 'files.deletedMany', { count })
}
