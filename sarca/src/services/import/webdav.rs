//! The read side of WebDAV that an import needs: list a folder, download a file.
//!
//! Written against what Nextcloud, ownCloud and Seafile's SeafDAV actually send —
//! each picks its own namespace prefix (`d:`, `D:`, none), so elements are matched
//! by local name only. Folders are walked one level at a time (`Depth: 1`):
//! `Depth: infinity` is off by default on several servers and unbounded on the
//! rest.

use std::time::Duration;

use chrono::{DateTime, Utc};
use percent_encoding::percent_decode_str;
use quick_xml::{Reader, events::Event};
use reqwest::{Method, StatusCode, Url, header};

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/></d:prop>
</d:propfind>"#;

/// A source error worded for the person who set the import up.
///
/// It never carries the password, and never the request URL either: `reqwest`
/// errors print it, and on some servers the URL is all the address there is to
/// protect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebDavError(pub String);

impl std::fmt::Display for WebDavError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

const AUTH_REJECTED: &str = "the server rejected the username or password (for Nextcloud with \
                             two-factor sign-in, use an app password)";

impl WebDavError {
    /// The source refused the credentials. Every other file would fail the same
    /// way, so the job stops instead of burning through its queue.
    pub fn is_auth(&self) -> bool {
        self.0 == AUTH_REJECTED
    }
}

fn transport_error(e: &reqwest::Error) -> WebDavError {
    if e.is_timeout() {
        WebDavError("the server did not answer in time".to_owned())
    } else if e.is_connect() {
        WebDavError("could not connect to the server".to_owned())
    } else {
        WebDavError(format!("network error: {}", describe(e)))
    }
}

/// The error and its causes, minus the request URL `reqwest` puts in `Display`.
pub fn describe(e: &reqwest::Error) -> String {
    let text = e.to_string();
    let mut out = text.split(" for url (").next().unwrap_or(&text).to_owned();
    let mut source = std::error::Error::source(e);
    while let Some(cause) = source {
        out.push_str(": ");
        out.push_str(&cause.to_string());
        source = cause.source();
    }
    out
}

fn status_error(status: StatusCode) -> WebDavError {
    WebDavError(match status {
        StatusCode::UNAUTHORIZED => AUTH_REJECTED.to_owned(),
        StatusCode::FORBIDDEN => "the server refused access to this folder".to_owned(),
        StatusCode::NOT_FOUND => "this folder does not exist on the server".to_owned(),
        StatusCode::METHOD_NOT_ALLOWED => {
            "this address is not a WebDAV folder — check the URL".to_owned()
        },
        other => format!("the server answered {other}"),
    })
}

/// One entry of a folder listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DavEntry {
    /// Path under the import root, `/`-separated, without leading or trailing
    /// slash; percent-decoded.
    pub rel_path: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: Option<DateTime<Utc>>,
}

#[derive(Clone)]
pub struct WebDavClient {
    http: reqwest::Client,
    /// The import root, always ending in `/`.
    root: Url,
    username: String,
    password: String,
}

impl std::fmt::Debug for WebDavClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebDavClient")
            .field("root", &self.root.as_str())
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

/// Check and normalise the folder URL a user typed: http(s) only, no
/// credentials inside it, and a trailing `/` so relative paths join under it.
pub fn parse_root_url(raw: &str) -> Result<Url, WebDavError> {
    let mut url = Url::parse(raw.trim())
        .map_err(|_| WebDavError("the address is not a valid URL".to_owned()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(WebDavError("the address must start with https:// or http://".to_owned()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(WebDavError(
            "put the username and password in their own fields, not in the address".to_owned(),
        ));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(WebDavError("the address has no host".to_owned()));
    }
    url.set_query(None);
    url.set_fragment(None);
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

impl WebDavClient {
    pub fn new(root: Url, username: String, password: String) -> Result<Self, WebDavError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            // Per read, not per request: a multi-gigabyte download is fine as long
            // as bytes keep arriving.
            .read_timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::limited(5))
            .user_agent(concat!("sarca-import/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| WebDavError(format!("could not set up the HTTP client: {e}")))?;
        Ok(Self {
            http,
            root,
            username,
            password,
        })
    }

    /// URL of `rel_path` (a folder when `dir`) under the root.
    fn url_for(&self, rel_path: &str, dir: bool) -> Url {
        let mut url = self.root.clone();
        if !rel_path.is_empty() {
            if let Ok(mut segments) = url.path_segments_mut() {
                // The root ends in `/`, which leaves an empty last segment.
                segments.pop_if_empty();
                segments.extend(rel_path.split('/'));
                if dir {
                    segments.push("");
                }
            }
        }
        url
    }

    /// List one folder (`""` is the root). The folder itself is left out.
    pub async fn list(&self, rel_dir: &str) -> Result<Vec<DavEntry>, WebDavError> {
        let url = self.url_for(rel_dir, true);
        let method = Method::from_bytes(b"PROPFIND").unwrap_or(Method::GET);
        let response = self
            .http
            .request(method, url.clone())
            .basic_auth(&self.username, Some(&self.password))
            .header("Depth", "1")
            .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
            .body(PROPFIND_BODY)
            .timeout(Duration::from_mins(5))
            .send()
            .await
            .map_err(|e| transport_error(&e))?;
        let status = response.status();
        if status != StatusCode::MULTI_STATUS {
            return Err(if status.is_success() {
                WebDavError("this address is not a WebDAV folder — check the URL".to_owned())
            } else {
                status_error(status)
            });
        }
        let body = response.text().await.map_err(|e| transport_error(&e))?;
        // Relative paths come out of the hrefs against the URL that was
        // actually answered, so a redirect to a canonical spelling still lines up.
        let base = url;
        let entries = parse_multistatus(&body)?;
        let root_path = decoded_path(&self.root);
        let listed_dir = decoded_path(&base);
        let mut out = Vec::with_capacity(entries.len());
        for raw in entries {
            let Ok(href) = base.join(&raw.href) else {
                continue;
            };
            let path = decoded_path(&href);
            if path.trim_end_matches('/') == listed_dir.trim_end_matches('/') {
                continue;
            }
            let Some(rel) = path.strip_prefix(&root_path) else {
                tracing::warn!("[IMPORT] skipping an entry outside the import root");
                continue;
            };
            let rel = rel.trim_matches('/').to_owned();
            if rel.is_empty()
                || rel.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..")
            {
                continue;
            }
            out.push(DavEntry {
                rel_path: rel,
                is_dir: raw.is_dir,
                size: raw.size,
                mtime: raw.mtime,
            });
        }
        Ok(out)
    }

    /// Start downloading a file. The caller streams the body.
    pub async fn get(&self, rel_path: &str) -> Result<reqwest::Response, WebDavError> {
        let response = self
            .http
            .get(self.url_for(rel_path, false))
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .map_err(|e| transport_error(&e))?;
        if response.status().is_success() {
            Ok(response)
        } else {
            Err(status_error(response.status()))
        }
    }
}

fn decoded_path(url: &Url) -> String {
    percent_decode_str(url.path()).decode_utf8_lossy().into_owned()
}

#[derive(Debug, Default)]
struct RawEntry {
    href: String,
    is_dir: bool,
    size: u64,
    mtime: Option<DateTime<Utc>>,
}

/// Which element's text is being collected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Href,
    Size,
    Mtime,
    Status,
}

/// Pull `href`, collection flag, size and mtime out of a 207 body. Properties
/// from a `propstat` whose status is not 200 are ignored — that is how a server
/// says "this property does not exist here".
fn parse_multistatus(body: &str) -> Result<Vec<RawEntry>, WebDavError> {
    let mut reader = Reader::from_str(body);
    let mut entries = Vec::new();
    let mut current: Option<RawEntry> = None;
    // Properties of the propstat being read, applied once its status is known.
    let mut pending = RawEntry::default();
    let mut propstat_ok = true;
    let mut field: Option<Field> = None;
    // Text of `field`. quick-xml hands entities (`&amp;`) over as separate
    // events, so one value can arrive in several pieces.
    let mut text = String::new();

    loop {
        let event = reader.read_event().map_err(|e| {
            WebDavError(format!("the server sent a listing that is not valid XML: {e}"))
        })?;
        match event {
            Event::Start(e) => {
                let next = match e.local_name().as_ref() {
                    "response" => {
                        current = Some(RawEntry::default());
                        None
                    },
                    "propstat" => {
                        pending = RawEntry::default();
                        propstat_ok = true;
                        None
                    },
                    "collection" => {
                        pending.is_dir = true;
                        None
                    },
                    "href" => Some(Field::Href),
                    "getcontentlength" => Some(Field::Size),
                    "getlastmodified" => Some(Field::Mtime),
                    "status" => Some(Field::Status),
                    _ => None,
                };
                if next.is_some() {
                    field = next;
                    text.clear();
                }
            },
            Event::Empty(e) => {
                if e.local_name().as_ref() == "collection" {
                    pending.is_dir = true;
                }
            },
            Event::Text(t) if field.is_some() => text.push_str(&t.xml10_content()),
            Event::CData(t) if field.is_some() => text.push_str(&t.into_inner()),
            Event::GeneralRef(r) if field.is_some() => {
                if let Ok(Some(ch)) = r.resolve_char_ref() {
                    text.push(ch);
                } else if let Some(s) =
                    quick_xml::escape::resolve_predefined_entity(&r.xml10_content())
                {
                    text.push_str(s);
                }
            },
            Event::End(e) => {
                match e.local_name().as_ref() {
                    "href" | "getcontentlength" | "getlastmodified" | "status" => {
                        let value = text.trim();
                        match field.take() {
                            Some(Field::Href) => {
                                if let Some(entry) = current.as_mut() {
                                    value.clone_into(&mut entry.href);
                                }
                            },
                            Some(Field::Size) => pending.size = value.parse().unwrap_or(0),
                            Some(Field::Mtime) => {
                                pending.mtime = DateTime::parse_from_rfc2822(value)
                                    .ok()
                                    .map(|t| t.with_timezone(&Utc));
                            },
                            // "HTTP/1.1 200 OK"
                            Some(Field::Status) => {
                                propstat_ok = value.split_whitespace().nth(1) == Some("200");
                            },
                            None => {},
                        }
                        text.clear();
                    },
                    "propstat" if propstat_ok => {
                        if let Some(entry) = current.as_mut() {
                            entry.is_dir |= pending.is_dir;
                            if pending.size > 0 {
                                entry.size = pending.size;
                            }
                            if pending.mtime.is_some() {
                                entry.mtime = pending.mtime;
                            }
                        }
                    },
                    "response" => {
                        if let Some(entry) = current.take() {
                            if !entry.href.is_empty() {
                                entries.push(entry);
                            }
                        }
                    },
                    _ => {},
                }
            },
            Event::Eof => break,
            _ => {},
        }
    }
    Ok(entries)
}

#[cfg(test)]
#[path = "webdav_tests.rs"]
mod tests;
