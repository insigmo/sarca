import { For, Show, createSignal, onCleanup, onMount } from 'solid-js'
import Button from '@suid/material/Button'
import TextField from '@suid/material/TextField'
import Typography from '@suid/material/Typography'

import API from '../api'
import { t } from '../common/i18n'
import { formatBytes } from '../common/nativeBridge'
import { alertStore } from './AlertStack'
import FluentIcon from './FluentIcon'

/** How often the job list refreshes while the tab is open. */
const POLL_MS = 4000

const ACTIVE = ['scanning', 'running']

/**
 * Import tab: copy a WebDAV folder (Nextcloud, ownCloud, Seafile) into a
 * storage. The copy runs on the server; this panel only starts jobs and shows
 * their progress. Superuser only — the server refuses anyone else.
 * @param {{ storages: Array<{ id: string, name: string }>, defaultStorageId?: string }} props
 */
const SettingsImportPanel = (props) => {
	const { addAlert } = alertStore
	/** @type {[import("solid-js").Accessor<import("../api").ImportJob[]>, any]} */
	const [jobs, setJobs] = createSignal([])
	const [url, setUrl] = createSignal('')
	const [username, setUsername] = createSignal('')
	const [password, setPassword] = createSignal('')
	const [storageId, setStorageId] = createSignal('')
	const [targetPath, setTargetPath] = createSignal('Imported')
	const [starting, setStarting] = createSignal(false)
	// Job id → password typed for a job that lost its own (restored backup).
	const [resumePasswords, setResumePasswords] = createSignal({})
	const [busyJob, setBusyJob] = createSignal('')

	const selectedStorage = () =>
		storageId() || props.defaultStorageId || props.storages[0]?.id || ''

	const refresh = async () => {
		try {
			const data = await API.imports.listImports()
			setJobs(data?.imports || [])
		} catch (err) {
			console.error(err)
		}
	}

	onMount(() => {
		refresh()
		const timer = setInterval(refresh, POLL_MS)
		onCleanup(() => clearInterval(timer))
	})

	const canStart = () =>
		!starting() &&
		url().trim() &&
		username().trim() &&
		password() &&
		selectedStorage()

	const start = async () => {
		if (!canStart()) return
		setStarting(true)
		try {
			await API.imports.createImport({
				storage_id: selectedStorage(),
				url: url().trim(),
				username: username().trim(),
				password: password(),
				target_path: targetPath().trim(),
			})
			setPassword('')
			addAlert(t('imports.started'), 'success')
			await refresh()
		} catch (err) {
			// apiRequest already alerted with the server's message.
			console.error(err)
		} finally {
			setStarting(false)
		}
	}

	/** @param {string} id @param {() => Promise<unknown>} action */
	const act = async (id, action) => {
		setBusyJob(id)
		try {
			await action()
			await refresh()
		} catch (err) {
			console.error(err)
		} finally {
			setBusyJob('')
		}
	}

	const storageName = (id) =>
		props.storages.find((s) => s.id === id)?.name || ''

	/** @param {import("../api").ImportJob} job */
	const percent = (job) =>
		job.total_bytes > 0
			? Math.min(100, Math.round((job.done_bytes / job.total_bytes) * 100))
			: 0

	return (
		<div class="settings-account settings-import">
			<p class="settings-account__label">{t('imports.title')}</p>
			<Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
				{t('imports.hint')}
			</Typography>

			<div class="settings-import__form">
				<TextField
					label={t('imports.urlLabel')}
					fullWidth
					value={url()}
					placeholder="https://cloud.example.com/remote.php/dav/files/alice/Photos/"
					onChange={(e) => setUrl(e.target.value)}
				/>
				<p class="settings-account__hint">{t('imports.urlHint')}</p>
				<TextField
					label={t('imports.usernameLabel')}
					fullWidth
					autoComplete="off"
					value={username()}
					onChange={(e) => setUsername(e.target.value)}
				/>
				<TextField
					type="password"
					label={t('imports.passwordLabel')}
					fullWidth
					autoComplete="new-password"
					value={password()}
					onChange={(e) => setPassword(e.target.value)}
				/>
				<p class="settings-account__hint">{t('imports.passwordHint')}</p>
				<label class="settings-select-field">
					<span class="settings-select-field__label">
						{t('imports.storageLabel')}
					</span>
					<select
						class="settings-select"
						value={selectedStorage()}
						onChange={(e) => setStorageId(e.currentTarget.value)}
					>
						<Show when={!props.storages.length}>
							<option value="" disabled>
								{t('settings.noStorages')}
							</option>
						</Show>
						<For each={props.storages}>
							{(storage) => <option value={storage.id}>{storage.name}</option>}
						</For>
					</select>
				</label>
				<TextField
					label={t('imports.targetLabel')}
					fullWidth
					value={targetPath()}
					onChange={(e) => setTargetPath(e.target.value)}
				/>
				<p class="settings-account__hint">{t('imports.targetHint')}</p>
				<div>
					<Button
						variant="contained"
						color="secondary"
						disabled={!canStart()}
						startIcon={<FluentIcon name="arrowImport" size={18} />}
						onClick={start}
					>
						{starting() ? t('imports.starting') : t('imports.start')}
					</Button>
				</div>
			</div>

			<p class="settings-account__label settings-import__jobs-title">
				{t('imports.jobsTitle')}
			</p>
			<Show
				when={jobs().length}
				fallback={<p class="settings-account__hint">{t('imports.empty')}</p>}
			>
				<ul class="settings-import__jobs">
					<For each={jobs()}>
						{(job) => (
							<li class="settings-import__job" data-status={job.status}>
								<div class="settings-import__job-head">
									<span
										class="settings-import__job-source"
										title={job.source_url}
									>
										{job.username} · {job.source_url}
									</span>
									<span class="settings-import__job-status">
										{t(`imports.status.${job.status}`)}
									</span>
								</div>
								<div class="settings-import__job-target">
									→ {storageName(job.storage_id)}/{job.target_path}
								</div>
								<div
									class="settings-import__bar"
									role="progressbar"
									aria-valuenow={percent(job)}
									aria-valuemin="0"
									aria-valuemax="100"
								>
									<div style={{ width: `${percent(job)}%` }} />
								</div>
								<div class="settings-account__hint">
									{t('imports.progress', {
										done: job.done_files + job.skipped_files,
										total: job.total_files,
										bytes: `${formatBytes(job.done_bytes)} / ${formatBytes(job.total_bytes)}`,
									})}
									<Show when={job.skipped_files > 0}>
										{' · '}
										{t('imports.skipped', { count: job.skipped_files })}
									</Show>
									<Show when={job.failed_files > 0}>
										{' · '}
										{t('imports.failedCount', { count: job.failed_files })}
									</Show>
								</div>
								<Show when={job.last_error}>
									<p class="settings-import__error">{job.last_error}</p>
								</Show>
								<Show
									// A failed job may have stopped on a rejected password,
									// so offer a new one there even when one is stored.
									when={
										job.status === 'failed' ||
										(!job.has_password && job.status === 'paused')
									}
								>
									<TextField
										type="password"
										size="small"
										fullWidth
										autoComplete="new-password"
										label={t('imports.newPasswordLabel')}
										value={resumePasswords()[job.id] || ''}
										onChange={(e) =>
											setResumePasswords({
												...resumePasswords(),
												[job.id]: e.target.value,
											})
										}
									/>
								</Show>
								<div class="settings-import__actions">
									<Show when={ACTIVE.includes(job.status)}>
										<Button
											size="small"
											disabled={busyJob() === job.id}
											onClick={() =>
												act(job.id, () => API.imports.pauseImport(job.id))
											}
										>
											{t('imports.pause')}
										</Button>
									</Show>
									<Show
										when={job.status === 'paused' || job.status === 'failed'}
									>
										<Button
											size="small"
											disabled={
												busyJob() === job.id ||
												(!job.has_password && !resumePasswords()[job.id])
											}
											onClick={() =>
												act(job.id, () =>
													API.imports.resumeImport(
														job.id,
														resumePasswords()[job.id],
													),
												)
											}
										>
											{t('imports.resume')}
										</Button>
									</Show>
									<Show
										when={
											job.failed_files > 0 &&
											job.has_password &&
											(job.status === 'done' || job.status === 'failed')
										}
									>
										<Button
											size="small"
											disabled={busyJob() === job.id}
											onClick={() =>
												act(job.id, () => API.imports.retryImport(job.id))
											}
										>
											{t('imports.retry')}
										</Button>
									</Show>
									<Show when={[...ACTIVE, 'paused'].includes(job.status)}>
										<Button
											size="small"
											color="error"
											disabled={busyJob() === job.id}
											onClick={() =>
												act(job.id, () => API.imports.cancelImport(job.id))
											}
										>
											{t('imports.cancel')}
										</Button>
									</Show>
									<Show when={!ACTIVE.includes(job.status)}>
										<Button
											size="small"
											color="error"
											disabled={busyJob() === job.id}
											startIcon={<FluentIcon name="delete" size={16} />}
											onClick={() =>
												act(job.id, () => API.imports.deleteImport(job.id))
											}
										>
											{t('imports.remove')}
										</Button>
									</Show>
								</div>
							</li>
						)}
					</For>
				</ul>
			</Show>
		</div>
	)
}

export default SettingsImportPanel
