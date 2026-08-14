import { For, createSignal, onMount } from 'solid-js'
import Box from '@suid/material/Box'
import TextField from '@suid/material/TextField'
import Button from '@suid/material/Button'
import Paper from '@suid/material/Paper'
import Stack from '@suid/material/Stack'
import MenuMUI from '@suid/material/Menu'
import MenuItem from '@suid/material/MenuItem'
import ListItemText from '@suid/material/ListItemText'
import createLocalStore from '../../libs'
import { useNavigate } from '@solidjs/router'

import API from '../api'
import { safeRedirectPath } from '../common/auth'
import logoUrl from '../assets/logo.svg'
import { i18n, LOCALES, t } from '../common/i18n'
import FluentIcon from '../components/FluentIcon'

/** Language picker shown on the login screen, mirrors the sidebar's switcher. */
const LoginLanguageSwitcher = () => {
	const [anchorEl, setAnchorEl] = createSignal(null)
	const open = () => Boolean(anchorEl())
	const closeMenu = () => setAnchorEl(null)
	const current = () => LOCALES.find((l) => l.code === i18n.locale()) || LOCALES[0]

	return (
		<>
			<button
				type="button"
				class="auth-language-switch"
				aria-label={t('sidebar.language')}
				title={current().label}
				aria-haspopup="menu"
				aria-expanded={open()}
				onClick={(e) => setAnchorEl(e.currentTarget)}
			>
				<FluentIcon name="localLanguage" size={18} />
				<span>{current().label}</span>
			</button>
			<MenuMUI anchorEl={anchorEl()} open={open()} onClose={closeMenu}>
				<For each={LOCALES}>
					{(entry) => (
						<MenuItem
							selected={entry.code === i18n.locale()}
							lang={entry.code}
							onClick={() => {
								i18n.setLocale(entry.code)
								closeMenu()
							}}
						>
							<ListItemText>{entry.label}</ListItemText>
						</MenuItem>
					)}
				</For>
			</MenuMUI>
		</>
	)
}

const Login = () => {
	const [store, setStore] = createLocalStore()
	const navigate = useNavigate()

	onMount(() => {
		if (store.access_token) {
			navigate('/')
		}
	})

	/**
	 * @param {SubmitEvent} event
	 */
	const handleSubmit = async (event) => {
		event.preventDefault()
		const data = new FormData(event.currentTarget)
		const email = data.get('email')
		const password = data.get('password')

		const tokenData = await API.auth.login(email, password)

		setStore('access_token', tokenData.access_token)
		setStore('refresh_token', tokenData.refresh_token)
		setStore('user', {
			email: tokenData.email || email,
			email_verified: tokenData.email_verified,
		})

		try {
			const me = await API.auth.meSilent()
			if (me) {
				setStore('user', {
					email: me.email,
					email_verified: me.email_verified,
					is_superuser: !!me.is_superuser,
				})
			}
		} catch {
			/* keep login payload */
		}

		const redirect_url = safeRedirectPath(store.redirect)
		// One-shot: a deep link consumed here must not steer the next sign-in.
		setStore('redirect', '/')
		navigate(redirect_url)
	}

	return (
		<div class="auth-page">
			<LoginLanguageSwitcher />
			<Paper class="auth-card" elevation={0}>
				<Box
					sx={{
						px: { xs: 3, sm: 4.5 },
						py: { xs: 3.5, sm: 4 },
						display: 'flex',
						flexDirection: 'column',
						gap: 2,
					}}
				>
					<div class="auth-brand">
						<img src={logoUrl} alt="Sarca" />
						<h1>Sarca</h1>
						<p>{t('auth.login.tagline')}</p>
					</div>

					<Box
						component="form"
						onSubmit={handleSubmit}
						sx={{ display: 'flex', flexDirection: 'column', gap: 2 }}
					>
						<TextField
							name="email"
							label={t('auth.login.email')}
							type="email"
							autoComplete="email"
							required
						/>
						<TextField
							name="password"
							label={t('auth.login.password')}
							type="password"
							autoComplete="current-password"
							required
						/>

						<Stack spacing={1.5} sx={{ mt: 0.5 }}>
							<Button type="submit" variant="contained" color="secondary" size="large">
								{t('auth.login.signIn')}
							</Button>
						</Stack>
					</Box>
				</Box>
			</Paper>
		</div>
	)
}

export default Login
