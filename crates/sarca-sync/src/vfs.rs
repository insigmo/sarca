//! Stage 2 virtual drive surface. Folder sync is the MVP; VFS mounts come later.

use std::path::Path;

use anyhow::{bail, Result};

/// Platform virtual / network drive backend.
pub trait VirtualDrive: Send + Sync {
    fn mount(&self, binding_id: &str, mount_point: &Path) -> Result<()>;
    fn unmount(&self, binding_id: &str) -> Result<()>;
    fn is_mounted(&self, binding_id: &str) -> bool;
}

/// Placeholder until Stage 2 backends ship.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnsupportedVirtualDrive;

impl VirtualDrive for UnsupportedVirtualDrive {
    fn mount(&self, _binding_id: &str, _mount_point: &Path) -> Result<()> {
        bail!("virtual drive is Stage 2 and not enabled in this build")
    }

    fn unmount(&self, _binding_id: &str) -> Result<()> {
        bail!("virtual drive is Stage 2 and not enabled in this build")
    }

    fn is_mounted(&self, _binding_id: &str) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_errors_on_mount() {
        let d = UnsupportedVirtualDrive;
        assert!(d.mount("x", Path::new("/tmp/sarca-vfs")).is_err());
        assert!(!d.is_mounted("x"));
    }
}
