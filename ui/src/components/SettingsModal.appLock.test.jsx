import { render, fireEvent } from '@solidjs/testing-library'
import { describe, it, expect, vi } from 'vitest'
import AppLockToggle from './AppLockToggle'

describe('SettingsModal app lock', () => {
	it('renders SettingsSwitch wired to lock state', () => {
		const onChange = vi.fn()
		const { getByRole } = render(() => (
			<AppLockToggle checked={false} onChange={onChange} />
		))
		const sw = getByRole('switch')
		expect(sw).toHaveAttribute('id', 'settings-app-lock-switch')
		expect(sw).toHaveAttribute('aria-checked', 'false')
		fireEvent.click(sw)
		expect(onChange).toHaveBeenCalledWith(true)
	})

	it('reflects enabled lock state', () => {
		const { getByRole } = render(() => (
			<AppLockToggle checked={true} onChange={() => {}} />
		))
		expect(getByRole('switch')).toHaveAttribute('aria-checked', 'true')
	})
})
