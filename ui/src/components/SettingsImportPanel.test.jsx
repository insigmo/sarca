import { render, fireEvent, waitFor } from '@solidjs/testing-library'
import { describe, it, expect, vi, beforeEach } from 'vitest'

const { addAlert, listImports, createImport, resumeImport } = vi.hoisted(
	() => ({
		addAlert: vi.fn(),
		listImports: vi.fn(),
		createImport: vi.fn(),
		resumeImport: vi.fn(),
	}),
)

vi.mock('../common/nativeBridge', () => ({
	formatBytes: (n) => `${n} B`,
}))

vi.mock('./AlertStack', () => ({
	alertStore: { addAlert },
}))

vi.mock('../api', () => ({
	default: {
		imports: {
			listImports: (...a) => listImports(...a),
			createImport: (...a) => createImport(...a),
			resumeImport: (...a) => resumeImport(...a),
			pauseImport: vi.fn(),
			retryImport: vi.fn(),
			cancelImport: vi.fn(),
			deleteImport: vi.fn(),
		},
	},
}))

import SettingsImportPanel from './SettingsImportPanel'

const storages = [
	{ id: 's1', name: 'Photos' },
	{ id: 's2', name: 'Docs' },
]

const job = (over = {}) => ({
	id: 'j1',
	storage_id: 's1',
	source_kind: 'webdav',
	source_url: 'https://cloud.example.com/dav/',
	username: 'alice',
	has_password: true,
	target_path: 'Imported',
	status: 'running',
	last_error: null,
	created_at: '2026-10-22T10:00:00Z',
	total_files: 10,
	total_bytes: 1000,
	done_files: 3,
	done_bytes: 400,
	skipped_files: 1,
	failed_files: 0,
	...over,
})

const field = (container, label) =>
	[...container.querySelectorAll('label')]
		.find((l) => l.textContent.includes(label))
		?.parentElement?.querySelector('input')

describe('SettingsImportPanel', () => {
	beforeEach(() => {
		vi.clearAllMocks()
		listImports.mockResolvedValue({ imports: [] })
		createImport.mockResolvedValue(job())
	})

	it('sends the form to the server and clears the password after', async () => {
		const { container, getByText } = render(() => (
			<SettingsImportPanel storages={storages} defaultStorageId="s2" />
		))
		const start = getByText('Check and start').closest('button')
		expect(start.disabled).toBe(true)

		fireEvent.input(field(container, 'WebDAV folder address'), {
			target: { value: ' https://cloud.example.com/dav/ ' },
		})
		fireEvent.input(field(container, 'Username'), {
			target: { value: 'alice' },
		})
		const password = field(container, 'Password')
		fireEvent.input(password, { target: { value: 'secret' } })
		await waitFor(() => expect(start.disabled).toBe(false))
		fireEvent.click(start)

		await waitFor(() =>
			expect(createImport).toHaveBeenCalledWith({
				storage_id: 's2',
				url: 'https://cloud.example.com/dav/',
				username: 'alice',
				password: 'secret',
				target_path: 'Imported',
			}),
		)
		await waitFor(() => expect(password.value).toBe(''))
		expect(addAlert).toHaveBeenCalledWith('Import started', 'success')
	})

	it('shows progress counting skipped files as done', async () => {
		listImports.mockResolvedValue({ imports: [job()] })
		const { findByText } = render(() => (
			<SettingsImportPanel storages={storages} />
		))
		expect(await findByText(/4 of 10 files/)).toBeTruthy()
		expect(await findByText('Copying')).toBeTruthy()
		expect(await findByText('Pause')).toBeTruthy()
	})

	it('asks for the password again for a job restored without one', async () => {
		listImports.mockResolvedValue({
			imports: [
				job({
					status: 'failed',
					has_password: false,
					last_error: 'no password',
				}),
			],
		})
		resumeImport.mockResolvedValue(job())
		const { container, findByText } = render(() => (
			<SettingsImportPanel storages={storages} />
		))
		const resume = (await findByText('Resume')).closest('button')
		expect(resume.disabled).toBe(true)

		fireEvent.input(field(container, 'Password for this source'), {
			target: { value: 'new-secret' },
		})
		await waitFor(() => expect(resume.disabled).toBe(false))
		fireEvent.click(resume)
		await waitFor(() =>
			expect(resumeImport).toHaveBeenCalledWith('j1', 'new-secret'),
		)
	})
})
