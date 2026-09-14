import { For, Show, createSignal, onMount } from 'solid-js'
import Box from '@suid/material/Box'
import TextField from '@suid/material/TextField'
import Button from '@suid/material/Button'
import Paper from '@suid/material/Paper'
import Stack from '@suid/material/Stack'
import MenuMUI from '@suid/material/Menu'
import MenuItem from '@suid/material/MenuItem'
import ListItemText from '@suid/material/ListItemText'
import Checkbox from '@suid/material/Checkbox'
import FormControlLabel from '@suid/material/FormControlLabel'
import createLocalStore from '../../libs'
import { useNavigate } from '@solidjs/router'

import API from '../api'
import { safeRedirectPath } from '../common/auth'
import {
	forgetAccount,
	isRemembered,
	rememberAccount,
	savedAccounts,
} from '../common/savedAccounts'
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
	// A native shell that reached the login screen with a remembered server is
	// still connected at the transport level: offer a way out of that loop.
	const [showDisconnect, setShowDisconnect] = createSignal(false)
	const [disconnecting, setDisconnecting] = createSignal(false)
	// The fields are controlled so picking a saved account can fill them.
	const [email, setEmail] = createSignal('')
	const [password, setPassword] = createSignal('')
	const [remember, setRemember] = createSignal(false)
	// Once the user works the checkbox themselves, typing in the email field
	// must stop overriding it — otherwise ticking the box and then correcting
	// a typo in the address silently unticks it again.
	const [rememberTouched, setRememberTouched] = createSignal(false)
	const accounts = () => savedAccounts()

	/** One click to sign in as a remembered account: fill both fields. */
	const useAccount = (account) => {
		setEmail(account.email)
		setPassword(account.password)
		setRemember(true)
		setRememberTouched(true)
	}

	/**
	 * Drop a remembered account without signing in as it. Stops the click from
	 * reaching the row button underneath.
	 * @param {MouseEvent} event
	 * @param {string} accountEmail
	 */
	const dropAccount = (event, accountEmail) => {
		event.stopPropagation()
		forgetAccount(accountEmail)
		if (email() === accountEmail) {
			setEmail('')
			setPassword('')
			setRemember(false)
			setRememberTouched(false)
		}
	}

	onMount(() => {
		if (store.access_token) {
			navigate('/')
			return
		}
		import('../common/nativeClient').then(({ isNativeClient }) => {
			setShowDisconnect(isNativeClient())
		}).catch(() => {})
	})

	/** Tear the native session down and reload the connect shell. */
	const handleDisconnect = async () => {
		if (disconnecting()) return
		setDisconnecting(true)
		try {
			const { nativeInvoke } = await import('../common/nativeBridge')
			await nativeInvoke('disconnect')
		} catch {
			// The connect shell reload below still lands the user somewhere sane.
		} finally {
			setDisconnecting(false)
		}
	}

	/**
	 * @param {SubmitEvent} event
	 */
	const handleSubmit = async (event) => {
		event.preventDefault()
		// Read the DOM, not the signals: a browser password manager can fill
		// these fields without ever firing an input event.
		const data = new FormData(event.currentTarget)
		const submittedEmail = String(data.get('email') || '')
		const submittedPassword = String(data.get('password') || '')

		// apiRequest already alerted the user about a rejected sign-in; without
		// this the throw escapes the event handler as an unhandled rejection.
		let tokenData
		try {
			tokenData = await API.auth.login(submittedEmail, submittedPassword)
		} catch {
			return
		}

		// Only after the server accepted them — storing a rejected password
		// would hand the user a one-click path to a failing login.
		if (remember()) {
			rememberAccount(submittedEmail, submittedPassword)
		} else {
			forgetAccount(submittedEmail)
		}

		setStore('access_token', tokenData.access_token)
		setStore('refresh_token', tokenData.refresh_token)
		setStore('user', {
			email: tokenData.email || submittedEmail,
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

					<Show when={accounts().length}>
						<div class="auth-accounts">
							<p class="auth-accounts__title">{t('auth.login.savedAccounts')}</p>
							<For each={accounts()}>
								{(account) => (
									<div class="auth-accounts__row">
										<button
											type="button"
											class="auth-accounts__pick"
											classList={{
												'auth-accounts__pick--active': email() === account.email,
											}}
											aria-label={t('auth.login.useAccountAria', {
												email: account.email,
											})}
											onClick={() => useAccount(account)}
										>
											<FluentIcon name="person" size={18} />
											<span class="auth-accounts__email">{account.email}</span>
										</button>
										<button
											type="button"
											class="auth-accounts__forget"
											aria-label={t('auth.login.forgetAccountAria', {
												email: account.email,
											})}
											title={t('auth.login.forgetAccount')}
											onClick={(e) => dropAccount(e, account.email)}
										>
											<FluentIcon name="dismiss" size={16} />
										</button>
									</div>
								)}
							</For>
						</div>
					</Show>

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
							value={email()}
							onChange={(e) => {
								setEmail(e.target.value)
								// Re-signing in as a remembered account keeps it
								// remembered without a second tick of the box.
								if (!rememberTouched()) {
									setRemember(isRemembered(e.target.value))
								}
							}}
						/>
						<TextField
							name="password"
							label={t('auth.login.password')}
							type="password"
							autoComplete="current-password"
							required
							value={password()}
							onChange={(e) => setPassword(e.target.value)}
						/>

						<FormControlLabel
							class="auth-remember"
							control={
								<Checkbox
									name="remember"
									color="secondary"
									checked={remember()}
									onChange={(_, checked) => {
										setRemember(checked)
										setRememberTouched(true)
									}}
								/>
							}
							label={t('auth.login.rememberMe')}
						/>
						<p class="auth-remember__hint">{t('auth.login.rememberMeHint')}</p>

						<Stack spacing={1.5} sx={{ mt: 0.5 }}>
							<Button type="submit" variant="contained" color="secondary" size="large">
								{t('auth.login.signIn')}
							</Button>
							<Show when={showDisconnect()}>
								<Button
									type="button"
									variant="outlined"
									color="inherit"
									size="large"
									disabled={disconnecting()}
									onClick={handleDisconnect}
									startIcon={<FluentIcon name="plugDisconnected" size={18} />}
								>
									{t('sidebar.disconnect')}
								</Button>
							</Show>
						</Stack>
					</Box>
				</Box>
			</Paper>
		</div>
	)
}

export default Login
