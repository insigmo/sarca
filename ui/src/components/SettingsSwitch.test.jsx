import { render, fireEvent } from '@solidjs/testing-library'
import { describe, it, expect, vi } from 'vitest'
import SettingsSwitch from './SettingsSwitch'

describe('SettingsSwitch', () => {
	it('exposes role=switch and toggles', async () => {
		const onChange = vi.fn()
		const { getByRole } = render(() => (
			<SettingsSwitch checked={false} onChange={onChange} />
		))
		const sw = getByRole('switch')
		expect(sw).toHaveAttribute('aria-checked', 'false')
		fireEvent.click(sw)
		expect(onChange).toHaveBeenCalledWith(true)
	})

	it('exposes an aria-label when provided', () => {
		const { getByRole } = render(() => (
			<SettingsSwitch checked={false} ariaLabel="Folder auto-upload: /pics" />
		))
		expect(getByRole('switch')).toHaveAttribute(
			'aria-label',
			'Folder auto-upload: /pics',
		)
	})

	it('stops label ancestors from receiving the click', () => {
		const onChange = vi.fn()
		const onLabelClick = vi.fn()
		const { getByRole } = render(() => (
			<label onClick={onLabelClick}>
				<span>Toggle</span>
				<SettingsSwitch checked={false} onChange={onChange} />
			</label>
		))
		fireEvent.click(getByRole('switch'))
		expect(onChange).toHaveBeenCalledTimes(1)
		expect(onChange).toHaveBeenCalledWith(true)
		expect(onLabelClick).not.toHaveBeenCalled()
	})
})
