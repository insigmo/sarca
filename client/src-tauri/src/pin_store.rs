//! On-disk trust-on-first-use pin store.
//!
//! A Sarca server's certificate names only its public address, so connecting
//! over a LAN address or loopback cannot validate through the web PKI. The
//! first successful sight of a host records the server's public key hash here,
//! and every later connection to that host must present the same key.
//!
//! Stored as `pinned_keys.json`: host to base16 `SHA-256(subjectPublicKeyInfo)`.
//! The file is written 0600 like `server.json` — it is not a secret, but a
//! writable pin file is a way to install a different key, so it stays as
//! locked down as the rest of the client state.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::RwLock,
};

use sarca_sync::PinStore;

use crate::state::write_private;

pub struct FilePinStore {
    path: PathBuf,
    /// Mirror of the file, so a handshake never blocks on disk.
    pins: RwLock<HashMap<String, String>>,
}

impl FilePinStore {
    pub fn new(data_dir: &Path) -> Self {
        let path = data_dir.join("pinned_keys.json");
        let pins = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<HashMap<String, String>>(&s).ok())
            .unwrap_or_default();
        Self {
            path,
            pins: RwLock::new(pins),
        }
    }

    fn persist(&self, pins: &HashMap<String, String>) {
        let Ok(json) = serde_json::to_string_pretty(pins) else {
            return;
        };
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(e) = write_private(&self.path, json.as_bytes()) {
            tracing::warn!(error = %e, "failed to persist pinned server keys");
        }
    }
}

impl PinStore for FilePinStore {
    fn get(&self, host: &str) -> Option<[u8; 32]> {
        let hex = self.pins.read().ok()?.get(host).cloned()?;
        decode_pin(&hex)
    }

    fn put(&self, host: &str, pin: [u8; 32]) {
        let snapshot = {
            let Ok(mut pins) = self.pins.write() else {
                return;
            };
            pins.insert(host.to_owned(), hex::encode(pin));
            pins.clone()
        };
        tracing::info!(host, "trusting server key on first use");
        self.persist(&snapshot);
    }
}

fn decode_pin(hex: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(hex).ok()?;
    bytes.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_survives_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePinStore::new(dir.path());
        assert_eq!(store.get("192.168.1.39"), None);

        store.put("192.168.1.39", [7u8; 32]);
        assert_eq!(store.get("192.168.1.39"), Some([7u8; 32]));

        let reloaded = FilePinStore::new(dir.path());
        assert_eq!(reloaded.get("192.168.1.39"), Some([7u8; 32]));
        assert_eq!(reloaded.get("10.0.0.1"), None);
    }

    #[test]
    fn a_corrupt_file_does_not_pin_anything() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("pinned_keys.json"), "{ not json").unwrap();

        let store = FilePinStore::new(dir.path());
        assert_eq!(store.get("192.168.1.39"), None);
        // A short value is not a pin either — never pad it into one.
        store
            .pins
            .write()
            .unwrap()
            .insert("h".into(), "aabb".into());
        assert_eq!(store.get("h"), None);
    }
}
