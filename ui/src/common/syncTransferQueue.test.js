import { describe, it, expect } from 'vitest'
import {
	countOpenTransfers,
	sortTransferItems,
} from './syncTransferQueue'

describe('sortTransferItems', () => {
	it('orders Active → Waiting → Done, then by name', () => {
		const sorted = sortTransferItems([
			{ name: 'z.jpg', status: 'done' },
			{ name: 'b.jpg', status: 'waiting' },
			{ name: 'a.jpg', status: 'active' },
			{ name: 'c.jpg', status: 'waiting' },
			{ name: 'm.jpg', status: 'done' },
			{ name: 'b2.jpg', status: 'active' },
		])
		expect(sorted.map((i) => `${i.status}:${i.name}`)).toEqual([
			'active:a.jpg',
			'active:b2.jpg',
			'waiting:b.jpg',
			'waiting:c.jpg',
			'done:m.jpg',
			'done:z.jpg',
		])
	})
})

describe('countOpenTransfers', () => {
	it('counts active and waiting for a direction', () => {
		const items = [
			{ direction: 'upload', status: 'active' },
			{ direction: 'upload', status: 'waiting' },
			{ direction: 'upload', status: 'done' },
			{ direction: 'download', status: 'active' },
			{ direction: 'download', status: 'done' },
		]
		expect(countOpenTransfers(items, 'upload')).toBe(2)
		expect(countOpenTransfers(items, 'download')).toBe(1)
	})
})
