//! On-demand loopback reverse proxy for the desktop/mobile webview.
//!
//! The Rust HTTP client can pin a server key (see [`crate::pinning`]), but the
//! platform webview cannot: it validates through the OS trust store, and a
//! Sarca certificate names only the server's public address. Loading the UI
//! from a LAN address therefore fails the handshake on every platform.
//!
//! So the client serves the UI from `http://127.0.0.1:<port>` and forwards each
//! request upstream over the pinned client. Loopback is a secure context in
//! every webview engine, and the TLS leg still gets the pin check.
//!
//! The proxy is started when it is needed and dropped when it is not: the port
//! is bound only while a server that needs it is open.

use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use bytes::Bytes;
use futures::TryStreamExt;
use http_body_util::{BodyExt, BodyStream, StreamBody};
use hyper::{
    body::{Frame, Incoming},
    header::{HeaderName, HeaderValue, HOST, LOCATION},
    service::service_fn,
    Request, Response, StatusCode,
};
use hyper_util::rt::TokioIo;
use reqwest::Client;
use tokio::{net::TcpListener, sync::oneshot};

/// Headers that describe a single hop and must not be forwarded (RFC 9110 §7.6.1).
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

type ProxyBody = http_body_util::combinators::BoxBody<Bytes, std::io::Error>;

/// Bind `port`, retrying briefly before giving up.
///
/// Reconnecting stops the old proxy and starts a new one, and the accept loop
/// only releases the socket once it observes the shutdown signal. Without the
/// retry that handover lost the race with itself and silently moved the webview
/// to a new origin — which is the entire thing a fixed port exists to prevent.
async fn bind_preferred(port: u16) -> Option<TcpListener> {
    const ATTEMPTS: u32 = 10;
    const PAUSE: Duration = Duration::from_millis(20);

    for attempt in 0..ATTEMPTS {
        match TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await {
            Ok(listener) => return Some(listener),
            Err(e) => {
                if attempt + 1 == ATTEMPTS {
                    tracing::warn!(port, error = %e, "preferred proxy port unavailable");
                    return None;
                }
                tokio::time::sleep(PAUSE).await;
            },
        }
    }
    None
}

async fn bind_ephemeral() -> Result<TcpListener> {
    TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .context("failed to bind a loopback port for the webview proxy")
}

/// A running proxy. Dropping it shuts the listener down and frees the port.
pub struct LocalProxy {
    port: u16,
    shutdown: Option<oneshot::Sender<()>>,
}

impl LocalProxy {
    /// Bind an ephemeral loopback port and forward everything to `base_url`.
    pub async fn start(base_url: &str) -> Result<Self> {
        Self::start_on(base_url, None).await
    }

    /// Bind `preferred` (falling back to an ephemeral port) and forward
    /// everything to `base_url`.
    ///
    /// The port is part of the webview's origin, and every web storage API —
    /// localStorage, IndexedDB, the HTTP cache — is partitioned by origin. A
    /// fresh ephemeral port per launch therefore handed the UI an empty
    /// profile every time: the preview and thumbnail caches started cold, and
    /// the session had to be re-established from disk. Reusing one port keeps
    /// that state addressable across restarts.
    pub async fn start_on(base_url: &str, preferred: Option<u16>) -> Result<Self> {
        let upstream = base_url.trim().trim_end_matches('/').to_owned();
        anyhow::ensure!(!upstream.is_empty(), "proxy needs an upstream base URL");

        let listener = match preferred.filter(|p| *p != 0) {
            Some(port) => match bind_preferred(port).await {
                Some(listener) => listener,
                // Something else holds it for real. A cold cache beats
                // refusing to start.
                None => bind_ephemeral().await?,
            },
            None => bind_ephemeral().await?,
        };
        let port = listener.local_addr()?.port();
        let (tx, mut rx) = oneshot::channel();

        // No timeout: a request may be a multi-gigabyte upload or download.
        let client = Arc::new(crate::api::proxy_http_client()?);
        let upstream = Arc::new(upstream);

        tokio::spawn(async move {
            loop {
                let stream = tokio::select! {
                    _ = &mut rx => break,
                    accepted = listener.accept() => match accepted {
                        Ok((stream, _)) => stream,
                        Err(e) => {
                            tracing::warn!(error = %e, "webview proxy accept failed");
                            continue;
                        }
                    },
                };
                let client = client.clone();
                let upstream = upstream.clone();
                tokio::spawn(async move {
                    let service =
                        service_fn(move |req| forward(client.clone(), upstream.clone(), req, port));
                    if let Err(e) = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await
                    {
                        tracing::debug!(error = %e, "webview proxy connection ended");
                    }
                });
            }
        });

        tracing::info!(port, "webview proxy started");
        Ok(Self {
            port,
            shutdown: Some(tx),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Origin the webview should be pointed at.
    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Stop serving and release the port.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
            tracing::info!(port = self.port, "webview proxy stopped");
        }
    }
}

impl Drop for LocalProxy {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn forward(
    client: Arc<Client>,
    upstream: Arc<String>,
    req: Request<Incoming>,
    port: u16,
) -> Result<Response<ProxyBody>, Infallible> {
    Ok(match proxy_request(&client, &upstream, req, port).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::warn!(error = %e, "webview proxy request failed");
            let mut resp = Response::new(body_from_bytes(Bytes::from_static(
                b"Sarca client could not reach the server.",
            )));
            *resp.status_mut() = StatusCode::BAD_GATEWAY;
            resp
        }
    })
}

/// Whether `host` is a literal loopback authority for our port.
///
/// The proxy listens on loopback, but loopback is not a boundary a browser
/// respects: a page on `evil.example` whose DNS name resolves to 127.0.0.1
/// (DNS rebinding) becomes *same-origin* with us, and from there it reads
/// every proxied response and drives the whole Sarca API. The port is
/// ephemeral, which only means the attacker has to scan for it.
///
/// A literal-IP `Host` cannot be produced by rebinding, because the browser
/// sends the hostname it was given, not the address it resolved to. So require
/// one. Requests with no `Host` at all are HTTP/1.1 protocol errors anyway.
fn is_loopback_host(host: &str, port: u16) -> bool {
    let Some((name, host_port)) = host.rsplit_once(':') else {
        // No port means port 80; we never listen there.
        return false;
    };
    if host_port.parse::<u16>() != Ok(port) {
        return false;
    }
    matches!(name, "127.0.0.1" | "localhost" | "[::1]")
}

async fn proxy_request(
    client: &Client,
    upstream: &str,
    req: Request<Incoming>,
    port: u16,
) -> Result<Response<ProxyBody>> {
    let (parts, body) = req.into_parts();

    let host_ok = parts
        .headers
        .get(HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| is_loopback_host(h, port));
    if !host_ok {
        tracing::warn!("webview proxy rejected a request with a non-loopback Host header");
        let mut resp = Response::new(body_from_bytes(Bytes::from_static(
            b"Sarca client proxy accepts loopback requests only.",
        )));
        *resp.status_mut() = StatusCode::FORBIDDEN;
        return Ok(resp);
    }

    let path = parts.uri.path_and_query().map_or("/", |p| p.as_str());
    let url = format!("{upstream}{path}");

    let mut outgoing = client.request(parts.method, &url);
    for (name, value) in &parts.headers {
        // `Host` belongs to the proxy origin; reqwest sets the upstream one.
        if name == HOST || is_hop_by_hop(name) {
            continue;
        }
        outgoing = outgoing.header(name, value);
    }

    // Stream the request body: uploads are multipart streams, not buffers.
    let stream = BodyStream::new(body)
        .try_filter_map(|frame| async move { Ok(frame.into_data().ok()) })
        .map_err(std::io::Error::other);
    let response = outgoing
        .body(reqwest::Body::wrap_stream(stream))
        .send()
        .await?;

    let mut builder = Response::builder().status(response.status());
    for (name, value) in response.headers() {
        if is_hop_by_hop(name) {
            continue;
        }
        if name == LOCATION {
            if let Some(rewritten) = rewrite_location(value, upstream, port) {
                builder = builder.header(name, rewritten);
                continue;
            }
        }
        builder = builder.header(name, value);
    }

    let body = response
        .bytes_stream()
        .map_err(std::io::Error::other)
        .map_ok(Frame::data);
    Ok(builder.body(StreamBody::new(body).boxed())?)
}

/// Point a redirect at the proxy when it targets the upstream origin.
///
/// A redirect to anywhere else is left alone: following it is the webview's
/// business, and rewriting it would route third-party traffic through us.
fn rewrite_location(value: &HeaderValue, upstream: &str, port: u16) -> Option<HeaderValue> {
    let location = value.to_str().ok()?;
    let rest = location.strip_prefix(upstream)?;
    HeaderValue::from_str(&format!("http://127.0.0.1:{port}{rest}")).ok()
}

fn is_hop_by_hop(name: &HeaderName) -> bool {
    HOP_BY_HOP.contains(&name.as_str())
}

fn body_from_bytes(bytes: Bytes) -> ProxyBody {
    http_body_util::Full::new(bytes)
        .map_err(|e: Infallible| match e {})
        .boxed()
}

/// Probe timeout for [`reachable_without_proxy`].
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether the webview can load `base_url` itself.
///
/// The webview validates through the OS trust store, so this is a plain
/// unpinned request: if *we* need the pin to connect, so would it.
pub async fn reachable_without_proxy(base_url: &str) -> bool {
    let Ok(client) = Client::builder().timeout(PROBE_TIMEOUT).build() else {
        return false;
    };
    let url = format!("{}/", base_url.trim().trim_end_matches('/'));
    client.get(url).send().await.is_ok()
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener as StdListener;

    use super::*;

    #[tokio::test]
    async fn a_restart_reuses_the_recorded_port() {
        let first = LocalProxy::start("https://example.invalid").await.unwrap();
        let port = first.port();
        first.stop();

        let again = LocalProxy::start_on("https://example.invalid", Some(port))
            .await
            .unwrap();
        assert_eq!(again.port(), port, "webview origin must survive a restart");
        assert_eq!(again.origin(), format!("http://127.0.0.1:{port}"));
    }

    #[tokio::test]
    async fn a_taken_port_falls_back_instead_of_failing_to_start() {
        let squatter = StdListener::bind(("127.0.0.1", 0)).unwrap();
        let taken = squatter.local_addr().unwrap().port();

        let proxy = LocalProxy::start_on("https://example.invalid", Some(taken))
            .await
            .unwrap();
        assert_ne!(proxy.port(), taken);
        assert_ne!(proxy.port(), 0);
    }

    #[test]
    fn only_literal_loopback_authorities_are_accepted() {
        assert!(is_loopback_host("127.0.0.1:8080", 8080));
        assert!(is_loopback_host("localhost:8080", 8080));
        assert!(is_loopback_host("[::1]:8080", 8080));
        // A rebound name: resolves to 127.0.0.1, but the browser sends this.
        assert!(!is_loopback_host("evil.example:8080", 8080));
        assert!(!is_loopback_host("evil.example", 8080));
        // Right name, wrong port: another local service, not us.
        assert!(!is_loopback_host("127.0.0.1:9090", 8080));
        assert!(!is_loopback_host("127.0.0.1", 8080));
        assert!(!is_loopback_host("", 8080));
    }

    #[tokio::test]
    async fn a_rebound_host_header_is_refused_before_reaching_upstream() {
        let (upstream_url, _up) = upstream().await;
        let proxy = LocalProxy::start(&upstream_url).await.unwrap();

        let response = Client::new()
            .get(format!("{}/api/files", proxy.origin()))
            .header(HOST, "evil.example")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    /// Upstream echo server: reports method, path and body, plus a redirect route.
    async fn upstream() -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let service = service_fn(|req: Request<Incoming>| async move {
                        let method = req.method().clone();
                        let uri = req.uri().to_string();
                        let host = req
                            .headers()
                            .get(HOST)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("");
                        let marker = req
                            .headers()
                            .get("x-marker")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("");
                        let host = host.to_owned();
                        let marker = marker.to_owned();
                        if uri.starts_with("/redirect") {
                            let target = format!("http://{}/after", host);
                            return Ok::<_, Infallible>(
                                Response::builder()
                                    .status(302)
                                    .header(LOCATION, target)
                                    .body(body_from_bytes(Bytes::new()))
                                    .unwrap(),
                            );
                        }
                        let body = req.into_body().collect().await.unwrap().to_bytes();
                        let echo = format!(
                            "{method} {uri} host={host} marker={marker} body={}",
                            String::from_utf8_lossy(&body)
                        );
                        Ok::<_, Infallible>(Response::new(body_from_bytes(Bytes::from(echo))))
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn forwards_method_path_headers_and_body() {
        let (base, _server) = upstream().await;
        let proxy = LocalProxy::start(&base).await.unwrap();

        let client = Client::new();
        let resp = client
            .post(format!("{}/api/files?take=2", proxy.origin()))
            .header("x-marker", "kept")
            .body("payload")
            .send()
            .await
            .unwrap();
        let text = resp.text().await.unwrap();

        assert!(text.starts_with("POST /api/files?take=2 "), "{text}");
        assert!(text.contains("marker=kept"), "{text}");
        assert!(text.ends_with("body=payload"), "{text}");
        // The upstream must see its own Host, not the proxy's.
        let upstream_host = base.trim_start_matches("http://");
        assert!(text.contains(&format!("host={upstream_host}")), "{text}");
    }

    #[tokio::test]
    async fn rewrites_a_redirect_back_to_the_proxy() {
        let (base, _server) = upstream().await;
        let proxy = LocalProxy::start(&base).await.unwrap();

        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let resp = client
            .get(format!("{}/redirect", proxy.origin()))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 302);
        assert_eq!(
            resp.headers().get(LOCATION).unwrap().to_str().unwrap(),
            format!("http://127.0.0.1:{}/after", proxy.port())
        );
    }

    #[tokio::test]
    async fn stop_frees_the_port() {
        let (base, _server) = upstream().await;
        let proxy = LocalProxy::start(&base).await.unwrap();
        let port = proxy.port();
        proxy.stop();

        // The accept loop closes the listener; the port must be bindable again.
        let mut bound = None;
        for _ in 0..50 {
            if let Ok(l) = StdListener::bind(("127.0.0.1", port)) {
                bound = Some(l);
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(bound.is_some(), "port {port} was never released");
    }
}
