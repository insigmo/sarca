import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

import { describe, expect, it } from 'vitest'

// Read the stylesheet as text: jsdom does no layout, so the only way to assert
// which side a fixed element is pinned to is to look at the rule itself.
const css = readFileSync(resolve(process.cwd(), 'src/index.css'), 'utf8')

/** The body of the first rule whose selector list contains `selector`. */
const ruleBody = (selector) => {
	const at = css.indexOf(`${selector} {`)
	expect(at, `no rule for ${selector}`).toBeGreaterThan(-1)
	return css.slice(at, css.indexOf('}', at))
}

// Arabic puts <html dir="rtl"> (see i18n.install), which mirrors flow-relative
// layout for free. Anything pinned with a *physical* `left`/`right` stays put —
// which is how the "+" FAB ended up alone on the right of an otherwise mirrored
// page. Fixed-position chrome has to use logical insets to travel with it.
describe('RTL chrome', () => {
	it('pins the new-file FAB with a logical inset, not a physical one', () => {
		const rule = ruleBody('.files-new-fab')
		expect(rule).toMatch(/inset-inline-end:/)
		expect(rule, 'a physical `right` does not flip under dir="rtl"').not.toMatch(
			/^\s*right:/m,
		)
		expect(rule, 'a physical `left` does not flip under dir="rtl"').not.toMatch(
			/^\s*left:/m,
		)
	})

	it('flips the safe-area inline insets for rtl', () => {
		// `env(safe-area-inset-*)` is always physical, so `inset-inline-end` has
		// to be fed the matching side per direction.
		expect(ruleBody(':root')).toMatch(
			/--sarca-safe-inline-end:\s*var\(--sarca-safe-right\)/,
		)
		expect(ruleBody("[dir='rtl']")).toMatch(
			/--sarca-safe-inline-end:\s*var\(--sarca-safe-left\)/,
		)
	})

	it('spaces the FAB icon from its label with a logical margin', () => {
		expect(ruleBody('.menu-fab-icon')).toMatch(/margin-inline-end:/)
	})
})
