import { describe, it, expect } from 'vitest'

import { validateChatId } from './StorageSettingsModal'

describe('validateChatId', () => {
	it('requires a value', () => {
		expect(validateChatId('')).toBe('Chat id is required')
		expect(validateChatId(null)).toBe('Chat id is required')
		expect(validateChatId(undefined)).toBe('Chat id is required')
	})

	it('accepts a negative integer', () => {
		expect(validateChatId('-1001234567890')).toBeNull()
		expect(validateChatId('-1')).toBeNull()
	})

	it('rejects non-negative values', () => {
		expect(validateChatId('0')).toBe('Chat id must be a negative integer')
		expect(validateChatId('100')).toBe('Chat id must be a negative integer')
	})

	it('rejects non-numeric input', () => {
		expect(validateChatId('abc')).toBe('Chat id must be a negative integer')
		expect(validateChatId('-')).toBe('Chat id must be a negative integer')
	})

	// Regression: saveChannel() converts the validated string with
	// parseInt(value, 10), which truncates fractional input. The old check
	// only tested Number.isFinite, so "-100.5" passed validation and was
	// then silently saved as chat id -100 — a different channel than the
	// user intended, with no warning that their value was altered.
	it('rejects fractional chat ids instead of silently truncating them', () => {
		expect(validateChatId('-100.5')).toBe('Chat id must be a negative integer')
		expect(validateChatId('-0.1')).toBe('Chat id must be a negative integer')
	})
})
