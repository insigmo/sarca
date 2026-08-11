import { describe, it, expect, beforeAll, afterAll } from 'vitest'

import { validateChatId } from './StorageSettingsModal'
import { i18n, DEFAULT_LOCALE } from '../common/i18n'

// Assert the resolved English text, not `t(key)` on both sides of the
// comparison: that form passes even when the key is missing, because both
// sides then fall through to the key string and the test stops checking that
// the message is the right one at all.
const REQUIRED = 'Chat id is required'
const NEGATIVE_INTEGER = 'Chat id must be a negative integer'

describe('validateChatId', () => {
	beforeAll(() => i18n.setLocale('en'))
	afterAll(() => i18n.setLocale(DEFAULT_LOCALE))

	it('requires a value', () => {
		expect(validateChatId('')).toBe(REQUIRED)
		expect(validateChatId(null)).toBe(REQUIRED)
		expect(validateChatId(undefined)).toBe(REQUIRED)
	})

	it('accepts a negative integer', () => {
		expect(validateChatId('-1001234567890')).toBeNull()
		expect(validateChatId('-1')).toBeNull()
	})

	it('rejects non-negative values', () => {
		expect(validateChatId('0')).toBe(NEGATIVE_INTEGER)
		expect(validateChatId('100')).toBe(NEGATIVE_INTEGER)
	})

	it('rejects non-numeric input', () => {
		expect(validateChatId('abc')).toBe(NEGATIVE_INTEGER)
		expect(validateChatId('-')).toBe(NEGATIVE_INTEGER)
	})

	// Regression: saveChannel() converts the validated string with
	// parseInt(value, 10), which truncates fractional input. The old check
	// only tested Number.isFinite, so "-100.5" passed validation and was
	// then silently saved as chat id -100 — a different channel than the
	// user intended, with no warning that their value was altered.
	it('rejects fractional chat ids instead of silently truncating them', () => {
		expect(validateChatId('-100.5')).toBe(NEGATIVE_INTEGER)
		expect(validateChatId('-0.1')).toBe(NEGATIVE_INTEGER)
	})
})
