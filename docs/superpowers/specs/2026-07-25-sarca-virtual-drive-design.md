# Virtual drive (Stage 2)

**Date:** 2026-07-25  
**Status:** Deferred — depends on stable folder sync  
**Scope:** Present Sarca storage as a virtual / network drive after classic folder sync is production-ready.

## Goals

1. Browse and open remote files without full local mirror (lazy download / on-demand).
2. Keep writes flowing through the same `sarca-sync` engine and server APIs.
3. Platform-native feel: drive letter / mount point.

## Non-goals (this stage)

- Replacing folder sync (classic bindings remain the default).
- Selective sync UI beyond what folder sync already provides.

## Approach by platform

| Platform | Mechanism | Notes |
| --- | --- | --- |
| Windows | ProjFS or Dokany/WinFsp network drive | Prefer ProjFS when available; fallback WinFsp |
| Linux | FUSE (fuse3) | User-space mount under `~/Sarca` or chosen path |
| macOS | FSKit / FUSE-T / macFUSE | Prefer system FSKit when targeting recent macOS |
| iOS / Android | Not a full VFS | Document provider / SAF; out of Stage 2 VFS |

## Engine integration

Expose trait in `sarca-sync`:

```rust
pub trait VirtualDrive: Send + Sync {
    fn mount(&self, binding_id: &str, mount_point: &Path) -> anyhow::Result<()>;
    fn unmount(&self, binding_id: &str) -> anyhow::Result<()>;
    fn is_mounted(&self, binding_id: &str) -> bool;
}
```

Default: `UnsupportedVirtualDrive` (errors with “not available on this build”).

Desktop Tauri command surface (future): `mount_virtual_drive`, `unmount_virtual_drive`.

Reads miss → download chunk/file via existing API into local cache → serve. Writes → upload queue. Deletes → remote delete. Conflicts → same prompt as folder sync.

## Cache

Per-binding cache dir under app data: `vfs-cache/<binding_id>/`. Eviction by LRU size cap (configurable).

## Exit criteria to start implementation

1. Folder sync + auto-upload stable on desktop for ≥1 release.
2. Changelog/snapshot APIs unchanged or versioned.
3. CI can build at least one VFS backend (Linux FUSE or Windows WinFsp).

## Tracking

Stub: `crates/sarca-sync/src/vfs.rs` (`VirtualDrive` trait + unsupported default).
