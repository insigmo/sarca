import { render, fireEvent } from '@solidjs/testing-library'
import { describe, it, expect, vi, beforeEach } from 'vitest'

import { alertStore } from './AlertStack'
import CreateFolderDialog from './CreateFolderDialog'

describe('CreateFolderDialog', () => {
	beforeEach(() => {
		// Drain any alerts left over from a previous test.
		for (const a of alertStore.alertList()) alertStore.dismissAlert(a.id)
	})

	// Regression: the <form onSubmit={onCreate}> handler took no event and
	// never called preventDefault(). Every other form in this codebase
	// (Login, GrantAccess, ShareLinkDialog, StorageSettingsModal) does. On a
	// real browser, submitting this form (Enter key, or clicking the
	// type="submit" Create button) fell through to the native default action
	// — a full-page GET reload of the current URL — wiping all app state.
	it('prevents the native form submission', async () => {
		render(() => (
			<CreateFolderDialog
				isOpened={true}
				onCreate={vi.fn().mockResolvedValue(undefined)}
				onClose={vi.fn()}
			/>
		))

		await fireEvent.input(document.getElementById('folder-name'), {
			target: { value: 'Photos' },
		})

		const form = document.querySelector('form')
		const submitEvent = new Event('submit', { bubbles: true, cancelable: true })
		form.dispatchEvent(submitEvent)

		expect(submitEvent.defaultPrevented).toBe(true)
	})

	it('calls onCreate with the typed name and closes on success', async () => {
		const onCreate = vi.fn().mockResolvedValue(undefined)
		const onClose = vi.fn()
		render(() => (
			<CreateFolderDialog isOpened={true} onCreate={onCreate} onClose={onClose} />
		))

		await fireEvent.input(document.getElementById('folder-name'), {
			target: { value: 'Photos' },
		})
		const form = document.querySelector('form')
		await fireEvent.submit(form)

		expect(onCreate).toHaveBeenCalledWith('Photos')
		expect(onClose).toHaveBeenCalledTimes(1)
	})

	// Regression: onClose() (which clears the input and unmounts the dialog)
	// used to run *before* `await props.onCreate(...)`, with no try/catch —
	// a rejected create (name conflict, network error, server-side validation)
	// became an unhandled promise rejection and the dialog just silently
	// closed with no feedback to the user.
	it('keeps the dialog open and surfaces an alert when onCreate rejects', async () => {
		const onCreate = vi.fn().mockRejectedValue(new Error('name already exists'))
		const onClose = vi.fn()
		render(() => (
			<CreateFolderDialog isOpened={true} onCreate={onCreate} onClose={onClose} />
		))

		await fireEvent.input(document.getElementById('folder-name'), {
			target: { value: 'Photos' },
		})
		const form = document.querySelector('form')
		await fireEvent.submit(form)
		// let the rejected promise's microtask settle
		await Promise.resolve()
		await Promise.resolve()

		expect(onCreate).toHaveBeenCalledWith('Photos')
		expect(onClose).not.toHaveBeenCalled()
		expect(
			alertStore.alertList().some((a) => /name already exists/.test(a.msg)),
		).toBe(true)
	})
})
