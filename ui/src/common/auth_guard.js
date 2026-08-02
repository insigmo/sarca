import { useLocation, useNavigate } from '@solidjs/router'
import createLocalStore from '../../libs'
import { safeRedirectPath } from './auth'

export function checkAuth() {
	const [store, setStore] = createLocalStore()
	const navigate = useNavigate()
	const location = useLocation()

	if (!store.access_token) {
		setStore('redirect', safeRedirectPath(location.pathname))

		navigate('/login')
	}
}
