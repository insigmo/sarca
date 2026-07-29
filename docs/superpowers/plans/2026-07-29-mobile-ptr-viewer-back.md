# Mobile PTR + Viewer System-Back Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On mobile Files, pull-to-refresh reloads the current listing; system/edge back closes the file preview when open without leaving the folder.

**Architecture:** Pure helpers for pull-gesture math and viewer history state; wire PTR touch handlers + indicator onto `.files-canvas`; replace the naive `popstate → reload` path with a handler that closes `viewerFile` first and uses `pushState`/`history.back()` so Android swipe-back matches expected app behavior.

**Tech Stack:** SolidJS (`ui/`), Vitest, existing Fluent icons + Sarca CSS tokens. No new npm dependencies.

**Spec:** `docs/superpowers/specs/2026-07-29-mobile-ptr-viewer-back-design.md`  
**Acceptance:** `.cursor/acceptance/2026-07-29-mobile-ptr-viewer-back.md`

## Global Constraints

- Mobile PTR only (`max-width: 840px` / equivalent runtime check); desktop unchanged.
- PTR on `.files-canvas` → `refreshCurrent()` (shared mode remains no-op via existing `refreshCurrent`).
- Viewer open → `history.pushState({ sarcaViewer: 1 }, …)`; system back closes viewer, stays in folder.
- UI close (X / Escape / backdrop) must not leave a stuck history entry.
- Prev/next inside viewer does not touch history.
- No swipe row actions, no viewer next/prev swipe, no PTR on Storages/Settings.
- Sarca `--sarca-*` tokens only (no Drive theme).
- `docs/` is gitignored — `git add -f` only when committing under `docs/superpowers/`.
- Prefer a dedicated feature branch off current `master`.

## File map

| File | Responsibility |
|------|----------------|
| `ui/src/common/pullToRefresh.js` | Pure pull-gesture helpers + `attachPullToRefresh(el, opts)` |
| `ui/src/common/pullToRefresh.test.js` | Vitest for threshold / cancel / refresh gate |
| `ui/src/common/viewerHistory.js` | Pure helpers: detect viewer history state; open/close stack ops |
| `ui/src/common/viewerHistory.test.js` | Vitest for open/close/`shouldHandlePopstate` |
| `ui/src/components/FluentIcon.jsx` | Register `arrowSync` (or `arrowClockwise`) glyph |
| `ui/src/pages/Files/index.jsx` | Wire PTR + viewer history / unified `popstate` |
| `ui/src/index.css` | `.files-ptr-indicator` styles (mobile) |

---

### Task 1: Viewer history helpers (TDD)

**Files:**
- Create: `ui/src/common/viewerHistory.js`
- Create: `ui/src/common/viewerHistory.test.js`

**Interfaces:**
- Produces:
  - `VIEWER_HISTORY_KEY = 'sarcaViewer'`
  - `isViewerHistoryState(state: unknown): boolean`
  - `pushViewerHistory(history = window.history, url = window.location.href): void` — pushes `{ sarcaViewer: 1 }`
  - `shouldCloseViewerOnPopstate({ viewerOpen: boolean, state: unknown }): boolean` — true when viewer is open (prefer closing over folder reload)
  - Note: actual `history.back()` / ignoring self-initiated back stays in Files page (needs Solid signals); helpers stay pure.

- [ ] **Step 1: Write failing tests**

```js
// ui/src/common/viewerHistory.test.js
import { describe, it, expect, vi } from 'vitest'
import {
	VIEWER_HISTORY_KEY,
	isViewerHistoryState,
	pushViewerHistory,
	shouldCloseViewerOnPopstate,
} from './viewerHistory'

describe('isViewerHistoryState', () => {
	it('detects sarcaViewer flag', () => {
		expect(isViewerHistoryState({ [VIEWER_HISTORY_KEY]: 1 })).toBe(true)
		expect(isViewerHistoryState({})).toBe(false)
		expect(isViewerHistoryState(null)).toBe(false)
	})
})

describe('pushViewerHistory', () => {
	it('pushState with viewer flag and current url', () => {
		const history = { pushState: vi.fn() }
		pushViewerHistory(history, 'https://example/files/s1/')
		expect(history.pushState).toHaveBeenCalledWith(
			{ [VIEWER_HISTORY_KEY]: 1 },
			'',
			'https://example/files/s1/',
		)
	})
})

describe('shouldCloseViewerOnPopstate', () => {
	it('closes when viewer open regardless of state payload', () => {
		expect(
			shouldCloseViewerOnPopstate({ viewerOpen: true, state: null }),
		).toBe(true)
		expect(
			shouldCloseViewerOnPopstate({ viewerOpen: false, state: { sarcaViewer: 1 } }),
		).toBe(false)
	})
})
```

- [ ] **Step 2: Run tests — expect FAIL**

Run: `cd ui && npm test -- src/common/viewerHistory.test.js`  
Expected: FAIL (module missing)

- [ ] **Step 3: Implement helpers**

```js
// ui/src/common/viewerHistory.js
export const VIEWER_HISTORY_KEY = 'sarcaViewer'

/** @param {unknown} state */
export function isViewerHistoryState(state) {
	return Boolean(state && typeof state === 'object' && state[VIEWER_HISTORY_KEY])
}

/**
 * @param {{ pushState: (data: object, unused: string, url?: string) => void }} history
 * @param {string} url
 */
export function pushViewerHistory(history, url) {
	history.pushState({ [VIEWER_HISTORY_KEY]: 1 }, '', url)
}

/**
 * @param {{ viewerOpen: boolean, state?: unknown }} opts
 */
export function shouldCloseViewerOnPopstate({ viewerOpen }) {
	return Boolean(viewerOpen)
}
```

- [ ] **Step 4: Run tests — expect PASS**

Run: `cd ui && npm test -- src/common/viewerHistory.test.js`  
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add ui/src/common/viewerHistory.js ui/src/common/viewerHistory.test.js
git commit -m "$(cat <<'EOF'
feat(ui): add viewer history helpers for mobile back

EOF
)"
```

---

### Task 2: Wire viewer close on system back in Files

**Files:**
- Modify: `ui/src/pages/Files/index.jsx` (viewer open/close + `popstate`)

**Interfaces:**
- Consumes: `pushViewerHistory`, `shouldCloseViewerOnPopstate` from Task 1
- Produces: `closeViewer()` used by FileViewer `onClose` and popstate path

**Behavior to implement:**
1. `let viewerHistoryPushed = false` + `let ignoringPopstate = false` (module-scoped locals inside component).
2. `createEffect` watching `viewerFile()`:
   - when becomes non-null and `!viewerHistoryPushed`: `pushViewerHistory(window.history, window.location.href)`; `viewerHistoryPushed = true`
   - when becomes null and `viewerHistoryPushed`: set `ignoringPopstate = true`, `viewerHistoryPushed = false`, `history.back()`, then clear `ignoringPopstate` on next microtask/popstate (see step below)
3. Replace `window.addEventListener('popstate', reload)` with:

```js
const onPopState = async (event) => {
	if (ignoringPopstate) {
		ignoringPopstate = false
		return
	}
	if (shouldCloseViewerOnPopstate({ viewerOpen: Boolean(viewerFile()), state: event.state })) {
		viewerHistoryPushed = false
		setViewerFile(null)
		return
	}
	await reload()
}
```

4. `closeViewer = () => { setViewerFile(null) }` — effect handles `history.back()` when flag set.
5. FileViewer: `onClose={closeViewer}` (instead of inline `setViewerFile(null)`).
6. Keep clearing viewer in enterTrash/Favorites/Recent/Shared as today (`setViewerFile(null)`); effect will sync history.

**Edge case:** If open viewer then navigate modes that clear viewer without user back — effect must `history.back()` once so stack stays clean.

- [ ] **Step 1: Implement wiring in Files/index.jsx** as above (import helpers; `createEffect` already imported).

- [ ] **Step 2: Manual smoke (desktop browser DevTools device mode is enough for history)**

- Open a file preview → browser Back → preview closes, URL/folder unchanged.
- Open preview → press Escape/X → preview closes; then Back once leaves the folder (one step), not a no-op.

- [ ] **Step 3: Commit**

```bash
git add ui/src/pages/Files/index.jsx
git commit -m "$(cat <<'EOF'
feat(ui): close file preview on system back

EOF
)"
```

---

### Task 3: Pull-to-refresh helpers (TDD)

**Files:**
- Create: `ui/src/common/pullToRefresh.js`
- Create: `ui/src/common/pullToRefresh.test.js`

**Interfaces:**
- Produces:
  - `PTR_THRESHOLD_PX = 64`
  - `PTR_MAX_PX = 96`
  - `canBeginPull({ scrollTop, refreshing }): boolean`
  - `pullDelta({ startY, currentY }): number` — `max(0, currentY - startY)`
  - `isHorizontalGesture({ startX, startY, currentX, currentY }): boolean` — `|dx| > |dy|` and `|dx| > 10`
  - `shouldTriggerRefresh(pullPx): boolean` — `pullPx >= PTR_THRESHOLD_PX`
  - `attachPullToRefresh(el, { onRefresh, isEnabled, getPullEl? }): () => void` — returns detach; manages touch listeners; calls `onPullChange(px)` / `onRefreshingChange(bool)` if provided; while pulling at top, `preventDefault` on `touchmove` only after vertical pull started (passive: false)

- [ ] **Step 1: Write failing tests for pure helpers**

```js
// ui/src/common/pullToRefresh.test.js
import { describe, it, expect } from 'vitest'
import {
	PTR_THRESHOLD_PX,
	canBeginPull,
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
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cd ui && npm test -- src/common/pullToRefresh.test.js`

- [ ] **Step 3: Implement `pullToRefresh.js`**

Implement pure helpers exactly as tested, plus `attachPullToRefresh`:

```js
/**
 * @param {HTMLElement} el scroll container
 * @param {{
 *   onRefresh: () => Promise<void> | void,
 *   isEnabled?: () => boolean,
 *   onPullChange?: (px: number) => void,
 *   onRefreshingChange?: (v: boolean) => void,
 * }} opts
 * @returns {() => void} detach
 */
export function attachPullToRefresh(el, opts) {
	let startX = 0
	let startY = 0
	let pulling = false
	let armed = false
	let refreshing = false
	let pullPx = 0

	const setPull = (px) => {
		pullPx = Math.min(PTR_MAX_PX, Math.max(0, px))
		opts.onPullChange?.(pullPx)
	}

	const onStart = (e) => {
		if (opts.isEnabled && !opts.isEnabled()) return
		if (!canBeginPull({ scrollTop: el.scrollTop, refreshing })) return
		const t = e.touches[0]
		startX = t.clientX
		startY = t.clientY
		armed = true
		pulling = false
	}

	const onMove = (e) => {
		if (!armed || refreshing) return
		const t = e.touches[0]
		if (
			isHorizontalGesture({
				startX,
				startY,
				currentX: t.clientX,
				currentY: t.clientY,
			})
		) {
			armed = false
			setPull(0)
			return
		}
		const d = pullDelta({ startY, currentY: t.clientY })
		if (d > 0 && el.scrollTop <= 0) {
			pulling = true
			if (e.cancelable) e.preventDefault()
			setPull(d)
		}
	}

	const onEnd = async () => {
		if (!armed && !pulling) return
		armed = false
		const trigger = pulling && shouldTriggerRefresh(pullPx)
		pulling = false
		if (!trigger) {
			setPull(0)
			return
		}
		refreshing = true
		opts.onRefreshingChange?.(true)
		setPull(PTR_THRESHOLD_PX)
		try {
			await opts.onRefresh()
		} finally {
			refreshing = false
			opts.onRefreshingChange?.(false)
			setPull(0)
		}
	}

	el.addEventListener('touchstart', onStart, { passive: true })
	el.addEventListener('touchmove', onMove, { passive: false })
	el.addEventListener('touchend', onEnd)
	el.addEventListener('touchcancel', onEnd)
	return () => {
		el.removeEventListener('touchstart', onStart)
		el.removeEventListener('touchmove', onMove)
		el.removeEventListener('touchend', onEnd)
		el.removeEventListener('touchcancel', onEnd)
	}
}
```

- [ ] **Step 4: Run — expect PASS**

Run: `cd ui && npm test -- src/common/pullToRefresh.test.js`

- [ ] **Step 5: Commit**

```bash
git add ui/src/common/pullToRefresh.js ui/src/common/pullToRefresh.test.js
git commit -m "$(cat <<'EOF'
feat(ui): add pull-to-refresh gesture helpers

EOF
)"
```

---

### Task 4: Fluent refresh icon + PTR CSS + Files wiring

**Files:**
- Modify: `ui/src/components/FluentIcon.jsx` — add `arrowClockwise` from `@fluentui/svg-icons/icons/arrow_clockwise_24_regular.svg?raw`
- Modify: `ui/src/index.css` — indicator styles
- Modify: `ui/src/pages/Files/index.jsx` — attach PTR; render indicator inside `.files-canvas`

**Interfaces:**
- Consumes: `attachPullToRefresh` from Task 3; `refreshCurrent` already in Files

- [ ] **Step 1: Register icon** in `FluentIcon.jsx` as `arrowSync`.

- [ ] **Step 2: Add CSS** (near `.files-canvas` mobile rules):

```css
.files-ptr-indicator {
	display: none;
}
@media (max-width: 840px) {
	.files-ptr-indicator {
		display: flex;
		align-items: center;
		justify-content: center;
		height: 0;
		overflow: hidden;
		color: var(--sarca-accent-deep);
		transition: height 0.15s ease;
		pointer-events: none;
	}
	.files-ptr-indicator--visible {
		/* height set inline from pull px, or min 48px while refreshing */
	}
	.files-ptr-indicator__icon {
		transition: transform 0.15s ease;
	}
	.files-ptr-indicator--spin .files-ptr-indicator__icon {
		animation: sarca-ptr-spin 0.8s linear infinite;
	}
	@keyframes sarca-ptr-spin {
		to { transform: rotate(360deg); }
	}
}
```

- [ ] **Step 3: Wire in Files**

Inside the non-shared `files-canvas` branch:

```jsx
const [ptrPull, setPtrPull] = createSignal(0)
const [ptrRefreshing, setPtrRefreshing] = createSignal(false)

onMount(() => {
	// existing mount work…
})

// After filesCanvasEl is set — use createEffect:
createEffect(() => {
	const el = filesCanvasEl
	if (!el) return
	const detach = attachPullToRefresh(el, {
		isEnabled: () => window.matchMedia('(max-width: 840px)').matches,
		onPullChange: setPtrPull,
		onRefreshingChange: setPtrRefreshing,
		onRefresh: () => refreshCurrent(),
	})
	onCleanup(detach)
})
```

Render at top of canvas children:

```jsx
<div
	class="files-ptr-indicator"
	classList={{
		'files-ptr-indicator--visible': ptrPull() > 0 || ptrRefreshing(),
		'files-ptr-indicator--spin': ptrRefreshing(),
	}}
	style={{
		height: ptrRefreshing()
			? '48px'
			: ptrPull() > 0
				? `${Math.min(96, ptrPull())}px`
				: '0px',
	}}
	aria-hidden="true"
>
	<span class="files-ptr-indicator__icon">
		<FluentIcon name="arrowClockwise" size={22} />
	</span>
</div>
```

Do **not** attach PTR on the shared-mode inner canvas (shared has its own panel); browse/trash/favorites/recent share the main canvas.

- [ ] **Step 4: Build UI**

Run: `cd /home/beta/git/sarca && task ui`  
Expected: build success

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/FluentIcon.jsx ui/src/index.css ui/src/pages/Files/index.jsx
git commit -m "$(cat <<'EOF'
feat(ui): mobile pull-to-refresh on Files canvas

EOF
)"
```

---

### Task 5: Acceptance verification

**Files:**
- Modify: `.cursor/acceptance/2026-07-29-mobile-ptr-viewer-back.md` (status + checks)

- [ ] **Step 1: Run unit tests**

Run: `cd ui && npm test -- src/common/viewerHistory.test.js src/common/pullToRefresh.test.js`  
Expected: all PASS

- [ ] **Step 2: Deploy local UI if using docker compose.dev**

Run: `docker compose -f compose.yml -f compose.dev.yml --env-file sarca.conf up -d --force-recreate sarca` (only if that is the active local stack)

- [ ] **Step 3: Manual on phone / remote UI**

1. Files at top → pull down → icon → list reloads.
2. Open preview → edge/system back → preview closes, same folder.
3. Open preview → X → folder back still works.
4. Long-press still opens menu; scroll still works; desktop no PTR chrome.

- [ ] **Step 4: Mark acceptance**

Set checklist Status `done` only when Must have items have fresh evidence.

---

## Spec coverage (self-review)

| Spec requirement | Task |
|------------------|------|
| PTR mobile Files → refreshCurrent | 3, 4 |
| System back closes preview | 1, 2 |
| UI close cleans history | 2 (`history.back` via effect) |
| No prev/next history churn | 2 (only open/close) |
| Desktop unchanged / no Drive theme | 4 CSS media query + tokens |
| Shared no-op OK | 4 uses `refreshCurrent` |

No placeholders remaining after self-review.
