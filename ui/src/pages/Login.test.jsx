import { render, fireEvent } from '@solidjs/testing-library'
import { Router, Routes, Route, useLocation } from '@solidjs/router'
import { describe, it, expect, vi, beforeEach } from 'vitest'

const login = vi.fn()
const meSilent = vi.fn()

vi.mock('../api', () => ({
	default: {
		auth: {
			login: (...args) => login(...args),
			meSilent: (...args) => meSilent(...args),
		},
	},
}))

import Login from './Login'

const Home = () => {
	const location = useLocation()
	return <div data-testid="home">home at {location.pathname}</div>
}

const renderApp = () =>
	render(() => (
		<Router>
			<Routes>
				<Route path="/login" component={Login} />
				<Route path="/storages" component={Home} />
				<Route path="/" component={Home} />
			</Routes>
		</Router>
	))

const submitLogin = async (container) => {
	const email = container.querySelector('input[name=email]')
	const password = container.querySelector('input[name=password]')
	fireEvent.input(email, { target: { value: 'user@example.com' } })
	fireEvent.input(password, { target: { value: 'secret' } })
	fireEvent.submit(container.querySelector('form'))
	// login + meSilent + navigate all settle within a few microtasks.
	for (let i = 0; i < 10; i++) await Promise.resolve()
}

describe('Login', () => {
	beforeEach(() => {
		localStorage.clear()
		login.mockReset()
		meSilent.mockReset()
		login.mockResolvedValue({
			access_token: 'access',
			refresh_token: 'refresh',
			email: 'user@example.com',
			email_verified: true,
		})
		meSilent.mockResolvedValue(null)
		window.history.replaceState(null, '', '/login')
	})

	it('leaves the login page after a successful sign in', async () => {
		const { container, findByTestId } = renderApp()

		await submitLogin(container)

		expect(await findByTestId('home')).toBeInTheDocument()
	})

	// Regression: a stale `redirect` pointing back at /login (written by the
	// 401 handler or the guard while the user was already on the login page)
	// made post-login navigate() a no-op — the token was stored, the page
	// never moved, and only a full restart looked "logged in".
	it('ignores a stale redirect that points back at the login page', async () => {
		localStorage.setItem('redirect', JSON.stringify('/login'))

		const { container, findByTestId } = renderApp()

		await submitLogin(container)

		expect(await findByTestId('home')).toBeInTheDocument()
	})
})
