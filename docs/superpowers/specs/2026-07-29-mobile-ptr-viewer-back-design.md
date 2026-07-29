# Design: Mobile pull-to-refresh + viewer system-back

- Date: 2026-07-29
- Status: approved (chat)
- Approach: history back for FileViewer + custom PTR on Files canvas

## Goal

On mobile:

1. **Pull-to-refresh** on the Files list (pull down → refresh indicator → reload current listing).
2. **System back / edge swipe-left** closes the file preview when it is open, instead of leaving the folder. Outside preview, existing back/navigation behavior stays unchanged.

## Out of scope

- Swipe actions on file rows/tiles (delete, favorite, etc.)
- Swipe next/prev inside the preview
- PTR on Storages, Settings, or other screens
- Theme / Drive chrome changes
- Desktop PTR

## Behavior

### File preview + system back

- Opening the preview (`viewerFile` set) pushes `history.pushState({ sarcaViewer: 1 }, "", url)`.
- System back / edge swipe / hardware back: if preview is open, close it (`viewerFile = null`) and **do not** change the current folder listing.
- Closing via UI (X, Escape, backdrop): call a shared close helper that clears `viewerFile` and pops the viewer history entry if present (so the stack stays clean).
- Prev/next navigation inside the viewer does **not** push or pop history.
- Existing folder `popstate` reload remains for non-viewer back navigation.

### Pull-to-refresh

- Mobile only (`max-width: 840px` or equivalent runtime check).
- Attached to `.files-canvas` (nested scroll container).
- Activates only when `scrollTop === 0` and the gesture is primarily vertical downward.
- Crossing a pull threshold shows a Sarca-token refresh indicator, then runs `refreshCurrent()` (browse / trash / favorites / recent). In shared-links mode, trigger the panel’s existing `load()` if wired; otherwise no-op is acceptable for v1 if shared has its own loader.
- While refreshing, ignore further PTR until the promise settles.
- Must not break: vertical scroll, long-press context menu, FAB, existing desktop tile/list behavior.

## Architecture

### Viewer history (`ui/src/pages/Files/index.jsx`)

- Shared `closeViewer()` used by FileViewer `onClose` and by the `popstate` path.
- `createEffect` (or open/close helpers): on open → `pushState`; on programmatic close → `history.back()` once if our viewer entry is on the stack, guarded by a “closing ourselves” flag so the ensuing `popstate` does not call `fetchFSLayer` / folder reload.
- `popstate` handler: if `viewerFile` is set (or state flag indicates viewer layer), close viewer and return; else keep current `reload()` behavior.

### Pull-to-refresh

- Small helper: e.g. `ui/src/common/usePullToRefresh.js` (touch listeners + pull distance + refreshing state).
- Wire from Files page onto the canvas element; CSS for the indicator in `ui/src/index.css` using `--sarca-*` tokens.
- No new npm dependency.

## Error handling

- If `refreshCurrent()` fails, end the PTR animation and surface existing alert/error paths if any; do not leave the indicator stuck.
- If history sync races (double back), prefer “preview closed, stay in folder” over navigating away.

## Testing / verification

- Mobile Files at top: pull down → indicator → list reloads.
- Open preview → system/edge back → preview closes, same folder.
- Open preview → close with X → back from folder still goes to parent (no extra dead history).
- Long-press menu and normal scroll still work.
- Desktop unchanged (no PTR UI).
