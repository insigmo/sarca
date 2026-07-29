import { describe, it, expect } from 'vitest'
import {
	PTR_ARM_PX,
	PTR_THRESHOLD_PX,
	canBeginPull,
	isPullArmed,
	pullDelta,
	isHorizontalGesture,
	shouldTriggerRefresh,
} from './pullToRefresh'

describe('canBeginPull', () => {
	it('only at scroll top when not refreshing', () => {
		expect(canBeginPull({ scrollTop: 0, refreshing: false })).toBe(true)
		expect(canBeginPull({ scrollTop: 5, refreshing: false })).toBe(false)
		expect(canBeginPull({ scrollTop: 0, refreshing: true })).toBe(false)
	})
})

describe('pullDelta / trigger', () => {
	it('computes downward pull and threshold', () => {
		expect(pullDelta({ startY: 100, currentY: 180 })).toBe(80)
		expect(pullDelta({ startY: 100, currentY: 90 })).toBe(0)
		expect(shouldTriggerRefresh(PTR_THRESHOLD_PX)).toBe(true)
		expect(shouldTriggerRefresh(PTR_THRESHOLD_PX - 1)).toBe(false)
	})
})

describe('isPullArmed', () => {
	it('waits for an 8px downward deadzone before claiming the gesture', () => {
		expect(isPullArmed(PTR_ARM_PX - 1)).toBe(false)
		expect(isPullArmed(PTR_ARM_PX)).toBe(true)
	})
})

describe('isHorizontalGesture', () => {
	it('ignores sideways swipes', () => {
		expect(
			isHorizontalGesture({
				startX: 0,
				startY: 0,
				currentX: 40,
				currentY: 5,
			}),
		).toBe(true)
		expect(
			isHorizontalGesture({
				startX: 0,
				startY: 0,
				currentX: 5,
				currentY: 40,
			}),
		).toBe(false)
	})
})
