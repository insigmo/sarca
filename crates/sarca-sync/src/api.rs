use std::path::Path;

use anyhow::{bail, Context, Result};
use reqwest::{
    multipart::{Form, Part},
    Client,
};
use tokio::fs::File;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::types::{ChangelogResponse, SnapshotResponse};

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

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.bearer_auth(&self.access_token)
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
            bail!("create_folder failed: {}", resp.status());
        }
        Ok(())
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
