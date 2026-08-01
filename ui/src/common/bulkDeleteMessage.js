/**
 * Success-toast text for a bulk delete/delete-forever operation.
 * @param {number} count number of items that actually succeeded
 * @param {string} [singleName] name of the item, used only when count === 1
 * @param {boolean} [permanent] true for trash "delete forever"
 * @returns {string}
 */
export const formatBulkDeleteMessage = (count, singleName, permanent) => {
	if (count === 1) {
		return permanent
			? `Permanently deleted "${singleName}"`
			: `Deleted "${singleName}"`
	}
	return permanent ? `Permanently deleted ${count} items` : `Deleted ${count} items`
}
