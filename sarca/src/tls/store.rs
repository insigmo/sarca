use std::path::{Path, PathBuf};

const CERT_FILE: &str = "cert.pem";
const KEY_FILE: &str = "key.pem";

/// PEM certificate and key files under a single directory.
#[derive(Debug, Clone)]
pub struct CertStore {
    dir: PathBuf,
}

impl CertStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn cert_path(&self) -> PathBuf {
        self.dir.join(CERT_FILE)
    }

    pub fn key_path(&self) -> PathBuf {
        self.dir.join(KEY_FILE)
    }

    pub async fn ensure_dir(&self) -> std::io::Result<()> {
        tokio::fs::create_dir_all(&self.dir).await
    }

    pub async fn load_cert(&self) -> std::io::Result<Option<String>> {
        Self::load_pem(&self.cert_path()).await
    }

    pub async fn load_key(&self) -> std::io::Result<Option<String>> {
        Self::load_pem(&self.key_path()).await
    }

    pub async fn save_cert(&self, pem: &str) -> std::io::Result<()> {
        self.ensure_dir().await?;
        tokio::fs::write(self.cert_path(), pem).await
    }

    pub async fn save_key(&self, pem: &str) -> std::io::Result<()> {
        self.ensure_dir().await?;
        tokio::fs::write(self.key_path(), pem).await
    }

    /// Read a PEM file by path, `None` when it does not exist.
    pub async fn load_pem_at(path: &Path) -> std::io::Result<Option<String>> {
        Self::load_pem(path).await
    }

    async fn load_pem(path: &Path) -> std::io::Result<Option<String>> {
        match tokio::fs::read_to_string(path).await {
            Ok(contents) => Ok(Some(contents)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn save_and_load_pem_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = CertStore::new(dir.path());

        assert!(store.load_cert().await.unwrap().is_none());
        assert!(store.load_key().await.unwrap().is_none());

        store
            .save_cert("-----BEGIN CERTIFICATE-----\ncert\n-----END CERTIFICATE-----\n")
            .await
            .unwrap();
        store
            .save_key("-----BEGIN PRIVATE KEY-----\nkey\n-----END PRIVATE KEY-----\n")
            .await
            .unwrap();

        assert!(store.cert_path().exists());
        assert!(store.key_path().exists());
        assert_eq!(
            store.load_cert().await.unwrap().unwrap(),
            "-----BEGIN CERTIFICATE-----\ncert\n-----END CERTIFICATE-----\n"
        );
        assert_eq!(
            store.load_key().await.unwrap().unwrap(),
            "-----BEGIN PRIVATE KEY-----\nkey\n-----END PRIVATE KEY-----\n"
        );
    }
}
