use std::path::Path;

use anyhow::{bail, Context, Result};
use reqwest::{
    multipart::{Form, Part},
    Client,
};
use serde::{Deserialize, Serialize};
use tokio::fs::File;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::types::{ChangelogResponse, SnapshotResponse};

#[derive(Debug, Clone, Deserialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub email_verified: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageSummary {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct StoragesResponse {
    pub storages: Vec<StorageSummary>,
}

#[derive(Clone)]
pub struct SarcaApi {
    client: Client,
    base_url: String,
    access_token: String,
}

impl SarcaApi {
    pub fn new(base_url: impl Into<String>, access_token: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            access_token: access_token.into(),
        }
    }

    pub fn set_token(&mut self, token: impl Into<String>) {
        self.access_token = token.into();
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    /// `Authorization` header value (`Bearer …`) when an access token is present.
    pub fn authorization_header(&self) -> Option<String> {
        authorization_header_value(&self.access_token)
    }

    fn require_access_token(&self) -> Result<()> {
        if self.access_token.trim().is_empty() {
            bail!(
                "Not authenticated — missing access token. Sign in again so Sync can use your session."
            );
        }
        Ok(())
    }

    /// Password login against `{base}/api/auth/login` (no prior token required).
    pub async fn login(
        base_url: impl AsRef<str>,
        email: impl AsRef<str>,
        password: impl AsRef<str>,
    ) -> Result<LoginResponse> {
        let base = normalize_server_url(base_url.as_ref())?;
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .context("failed to create HTTP client")?;
        let url = format!("{base}/api/auth/login");
        let resp = match client
            .post(&url)
            .json(&serde_json::json!({
                "email": email.as_ref(),
                "password": password.as_ref(),
            }))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(err) => {
                if err.is_timeout() {
                    bail!("Cannot reach server — connection timed out. Check the URL and network.");
                }
                if err.is_connect()
                    || err.is_request()
                    || err
                        .to_string()
                        .to_ascii_lowercase()
                        .contains("dns")
                {
                    bail!(
                        "Cannot reach server — no connection. Check the URL and that the server is running."
                    );
                }
                bail!("Cannot reach server: {err}");
            }
        };
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status.as_u16() == 401 || status.as_u16() == 403 {
                bail!("Invalid email or password");
            }
            if status.is_server_error() {
                bail!("Server error ({status}). Try again later.");
            }
            bail!("Login failed ({status}): {body}");
        }
        Ok(resp.json().await.context("invalid login response")?)
    }

    /// Exchange a refresh token for a new access/refresh pair.
    pub async fn refresh(
        base_url: impl AsRef<str>,
        refresh_token: impl AsRef<str>,
    ) -> Result<LoginResponse> {
        let base = normalize_server_url(base_url.as_ref())?;
        let refresh = refresh_token.as_ref().trim();
        if refresh.is_empty() {
            bail!("Missing refresh token — sign in again");
        }
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .context("failed to create HTTP client")?;
        let url = format!("{base}/api/auth/refresh");
        let resp = client
            .post(&url)
            .json(&serde_json::json!({ "refresh_token": refresh }))
            .send()
            .await
            .context("token refresh request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            bail!("token refresh failed: {status}");
        }
        Ok(resp.json().await.context("invalid refresh response")?)
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.bearer_auth(&self.access_token)
    }

    pub async fn list_storages(&self) -> Result<Vec<StorageSummary>> {
        self.require_access_token()?;
        let url = format!("{}/api/storages", self.base_url);
        let resp = self
            .auth(self.client.get(url))
            .send()
            .await?
            .error_for_status()?;
        let body: StoragesResponse = resp.json().await.context("invalid storages response")?;
        Ok(body.storages)
    }

    pub async fn snapshot(&self, storage_id: Uuid) -> Result<SnapshotResponse> {
        let url = format!("{}/api/storages/{storage_id}/sync/snapshot", self.base_url);
        let resp = self
            .auth(self.client.get(url))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn changelog(
        &self,
        storage_id: Uuid,
        cursor: i64,
        limit: i64,
    ) -> Result<ChangelogResponse> {
        let url = format!(
            "{}/api/storages/{storage_id}/sync/changelog?cursor={cursor}&limit={limit}",
            self.base_url
        );
        let resp = self
            .auth(self.client.get(url))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn download_to(
        &self,
        storage_id: Uuid,
        remote_path: &str,
        dest: &Path,
    ) -> Result<()> {
        let encoded = remote_path
            .split('/')
            .map(|p| urlencoding_encode(p))
            .collect::<Vec<_>>()
            .join("/");
        let url = format!(
            "{}/api/storages/{storage_id}/files/download/{encoded}",
            self.base_url
        );
        let resp = self
            .auth(self.client.get(url))
            .send()
            .await?
            .error_for_status()?;
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let bytes = resp.bytes().await?;
        tokio::fs::write(dest, bytes)
            .await
            .with_context(|| format!("write {}", dest.display()))?;
        Ok(())
    }

    pub async fn delete_remote(&self, storage_id: Uuid, remote_path: &str) -> Result<()> {
        let encoded = remote_path
            .split('/')
            .map(|p| urlencoding_encode(p))
            .collect::<Vec<_>>()
            .join("/");
        let url = format!(
            "{}/api/storages/{storage_id}/files/{encoded}",
            self.base_url
        );
        let resp = self.auth(self.client.delete(url)).send().await?;
        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            bail!("delete failed: {}", resp.status());
        }
        Ok(())
    }

    pub async fn upload_file(
        &self,
        storage_id: Uuid,
        parent_path: &str,
        filename: &str,
        local_path: &Path,
        mtime_ms: Option<i64>,
        content_hash: Option<&str>,
    ) -> Result<()> {
        let file = File::open(local_path).await?;
        let meta = file.metadata().await?;
        let stream = ReaderStream::new(file);
        let body = reqwest::Body::wrap_stream(stream);
        let part = Part::stream_with_length(body, meta.len())
            .file_name(filename.to_owned())
            .mime_str("application/octet-stream")?;

        let mut form = Form::new()
            .text("path", parent_path.to_owned())
            .text("filename", filename.to_owned())
            .part("file", part);
        if let Some(ms) = mtime_ms {
            form = form.text("mtime", ms.to_string());
        }
        if let Some(hash) = content_hash {
            form = form.text("content_hash", hash.to_owned());
        }

        let url = format!("{}/api/storages/{storage_id}/files/upload", self.base_url);
        let resp = self
            .auth(self.client.post(url).multipart(form))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("upload failed: {status} {body}");
        }
        // Drain NDJSON progress stream.
        let _ = resp.bytes().await?;
        Ok(())
    }

    pub async fn create_folder(
        &self,
        storage_id: Uuid,
        parent: &str,
        folder_name: &str,
    ) -> Result<()> {
        self.require_access_token()?;
        let url = format!(
            "{}/api/storages/{storage_id}/files/create_folder",
            self.base_url
        );
        let body = serde_json::json!({
            "path": parent,
            "folder_name": folder_name,
        });
        let resp = self.auth(self.client.post(url).json(&body)).send().await?;
        if !resp.status().is_success() && resp.status().as_u16() != 409 {
            let status = resp.status();
            let detail = resp.text().await.unwrap_or_default();
            if detail.trim().is_empty() {
                bail!("create_folder failed: {status}");
            }
            bail!("create_folder failed: {status} {detail}");
        }
        Ok(())
    }
}

/// Build `Authorization: Bearer …` value, or `None` when the token is empty.
pub fn authorization_header_value(access_token: &str) -> Option<String> {
    let token = access_token.trim();
    if token.is_empty() {
        None
    } else {
        Some(format!("Bearer {token}"))
    }
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        }
    }
    out
}

/// Normalize a Sarca server base URL.
/// Accepts `http://…`, `https://…`, or a host/IP without scheme (defaults to `http://`).
pub fn normalize_server_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        bail!("Server URL is required");
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("http://{trimmed}")
    };
    let parsed = reqwest::Url::parse(&with_scheme).map_err(|_| {
        anyhow::anyhow!("Invalid server URL. Use http:// or https:// and a valid host.")
    })?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => bail!("Unsupported URL scheme '{other}'. Use http:// or https://."),
    }
    if parsed.host_str().is_none() {
        bail!("Invalid server URL — missing host.");
    }
    // Drop path/query; API client always appends `/api/...`.
    let mut out = format!(
        "{}://{}",
        parsed.scheme(),
        parsed.host_str().unwrap_or_default()
    );
    if let Some(port) = parsed.port() {
        out.push(':');
        out.push_str(&port.to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn normalizes_bare_host_to_http() {
        assert_eq!(
            normalize_server_url("192.168.1.40:8001").unwrap(),
            "http://192.168.1.40:8001"
        );
    }

    #[test]
    fn keeps_https() {
        assert_eq!(
            normalize_server_url("https://sarca.example.com/").unwrap(),
            "https://sarca.example.com"
        );
    }

    #[test]
    fn rejects_bad_scheme() {
        assert!(normalize_server_url("ftp://x").is_err());
    }

    #[test]
    fn authorization_header_present_when_token_set() {
        assert_eq!(
            authorization_header_value("abc.def.ghi").as_deref(),
            Some("Bearer abc.def.ghi")
        );
        let api = SarcaApi::new("http://127.0.0.1:9", "tok-123");
        assert_eq!(
            api.authorization_header().as_deref(),
            Some("Bearer tok-123")
        );
    }

    #[test]
    fn authorization_header_none_when_token_missing() {
        assert_eq!(authorization_header_value(""), None);
        assert_eq!(authorization_header_value("   "), None);
        let api = SarcaApi::new("http://127.0.0.1:9", "");
        assert_eq!(api.authorization_header(), None);
    }

    #[tokio::test]
    async fn create_folder_fails_clearly_without_access_token() {
        let api = SarcaApi::new("http://127.0.0.1:9", "  ");
        let err = api
            .create_folder(Uuid::nil(), "", "Camera")
            .await
            .expect_err("empty token must fail before HTTP");
        let msg = err.to_string();
        assert!(
            msg.contains("access token") || msg.contains("Not authenticated"),
            "unexpected error: {msg}"
        );
    }

    #[tokio::test]
    async fn create_folder_sends_authorization_bearer_header() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            let _ = tx.send(req);
            let _ = sock
                .write_all(b"HTTP/1.1 409 Conflict\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
        });

        let api = SarcaApi::new(format!("http://{addr}"), "test-access-token");
        api.create_folder(Uuid::nil(), "", "Camera")
            .await
            .expect("409 Conflict is treated as success (folder exists)");

        let req = rx.await.expect("server must receive request");
        assert!(
            req.to_ascii_lowercase()
                .contains("authorization: bearer test-access-token"),
            "Authorization header missing in request:\n{req}"
        );
        assert!(
            req.contains("/files/create_folder"),
            "wrong path in request:\n{req}"
        );
    }
}
