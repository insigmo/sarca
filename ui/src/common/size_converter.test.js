import { describe, expect, it } from 'vitest'

import { convertSize } from './size_converter'

describe('convertSize', () => {
	it('formats bytes without decimals', () => {
		expect(convertSize(0)).toBe('0 bytes')
		expect(convertSize(512)).toBe('512 bytes')
	})

	it('formats larger units with one decimal below 10', () => {
		expect(convertSize(1024)).toBe('1.0 KB')
		expect(convertSize(1536)).toBe('1.5 KB')
		expect(convertSize(1024 * 1024 * 10)).toBe('10 MB')
	})

	it('caps at the largest unit (TB)', () => {
		expect(convertSize(1024 ** 5)).toBe('1024 TB')
	})

	// Regression: FileInfo.jsx passes a folder's `size` straight through, and
	// folders coming back from the API have no `size` field at all. Before this
	// fix, convertSize(undefined) threw "Cannot read properties of undefined
	// (reading 'toFixed')" because the initial `n >= 1024` check silently fails
	// for undefined/NaN while `n.toFixed` still runs unconditionally below.
	it('does not throw and falls back to 0 bytes for non-numeric input', () => {
		expect(() => convertSize(undefined)).not.toThrow()
		expect(convertSize(undefined)).toBe('0 bytes')
		expect(convertSize(null)).toBe('0 bytes')
		expect(convertSize(Number.NaN)).toBe('0 bytes')
		expect(convertSize(-5)).toBe('0 bytes')
	})
})
