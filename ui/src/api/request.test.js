import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

describe('apiRequest 401 handling', () => {
	beforeEach(() => {
		localStorage.clear()
		localStorage.setItem('access_token', JSON.stringify('old-access'))
		localStorage.setItem('refresh_token', JSON.stringify('stored-refresh'))
		Object.defineProperty(window, 'location', {
			configurable: true,
			value: { ...window.location, assign: vi.fn(), pathname: '/files/x' },
		})
	})

	afterEach(() => {
		vi.restoreAllMocks()
		vi.resetModules()
	})

	it('forces re-login instead of surfacing a raw alert when the post-refresh retry also 401s', async () => {
		vi.resetModules()
		const { alertStore } = await import('../components/AlertStack')
		const addAlertSpy = vi.spyOn(alertStore, 'addAlert')

		let call = 0
		global.fetch = vi.fn(async (url) => {
			call += 1
			if (String(url).includes('/auth/refresh')) {
				return {
					ok: true,
					status: 200,
					json: async () => ({
						access_token: 'new-access',
						refresh_token: 'new-refresh',
					}),
				}
			}
			// Both the original request and the post-refresh retry hit the
			// same folder-listing endpoint and both come back 401.
			return {
				ok: false,
				status: 401,
				text: async () => `unauthorized (call ${call})`,
			}
		})

		const { default: apiRequest } = await import('./request')

		await expect(
			apiRequest('/storages/x/files/tree/', 'get', 'Bearer old-access'),
		).rejects.toThrow()

		expect(window.location.assign).toHaveBeenCalledWith('/login')
		expect(addAlertSpy).not.toHaveBeenCalled()
		expect(localStorage.getItem('access_token')).toBeNull()
		expect(localStorage.getItem('refresh_token')).toBeNull()
	})
})
