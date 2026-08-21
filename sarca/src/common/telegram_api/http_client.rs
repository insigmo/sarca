//! Single shared, connection-pooled `reqwest::Client` for outbound Telegram Bot
//! API calls, used by both `bot_api.rs` and `token_client.rs`.
//!
//! Every call used to build its own `reqwest::Client`. A fresh client starts
//! with an empty connection pool, so every `getFile` and every chunk
//! download/upload paid a full TCP + TLS handshake to `api.telegram.org`
//! before any bytes moved. On a degraded route that handshake can be most of
//! the wall time a caller sees. Reusing one pooled client keeps warm
//! connections around across calls.
//!
//! This is also the only place `TELEGRAM_PROXY_URL` gets wired up. The TLS
//! session to `api.telegram.org` is always negotiated end to end by this
//! client — nothing here ever sets `danger_accept_invalid_certs` — so a
//! configured proxy sees at most a `CONNECT host:443` (HTTP proxy) or a plain
//! byte stream (SOCKS5). It learns the destination host and traffic volume,
//! never bot tokens or file contents.

use std::{sync::OnceLock, time::Duration};

/// Bounds only the TCP+TLS handshake. Chunk uploads/downloads are large and
/// legitimately slow, so the clients built here carry no total-request
/// timeout beyond this — a caller that wants one (see `client_with_timeout`)
/// asks for it explicitly.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Schemes `reqwest::Proxy` can act on. `socks5h` is preferred over `socks5`:
/// it resolves the target hostname at the proxy, so our local resolver never
/// sees `api.telegram.org`. `socks5` resolves the hostname locally first,
/// then only tunnels the connection — the proxy operator still can't read
/// the traffic, but a local resolver/observer learns the destination.
const ALLOWED_PROXY_SCHEMES: [&str; 4] = ["http://", "https://", "socks5://", "socks5h://"];

/// Whether `url` starts with a scheme this client can proxy through.
///
/// Shared with `config.rs` so an invalid `TELEGRAM_PROXY_URL` fails fast at
/// startup instead of silently falling back to a direct connection later.
pub fn has_allowed_scheme(url: &str) -> bool {
    ALLOWED_PROXY_SCHEMES.iter().any(|scheme| url.starts_with(scheme))
}

/// `TELEGRAM_PROXY_URL`, resolved once. `Config::new` already rejects an
/// unrecognized scheme at startup, so by the time anything here runs the
/// value (if present) is known-good; the scheme check is only defense in
/// depth against something changing the environment after that.
fn configured_proxy_url() -> Option<&'static str> {
    static RESOLVED: OnceLock<Option<String>> = OnceLock::new();
    RESOLVED
        .get_or_init(|| {
            let raw = std::env::var("TELEGRAM_PROXY_URL").ok()?;
            let trimmed = raw.trim();
            (!trimmed.is_empty() && has_allowed_scheme(trimmed)).then(|| trimmed.to_owned())
        })
        .as_deref()
}

fn build(timeout: Option<Duration>) -> reqwest::Client {
    build_with(configured_proxy_url(), timeout)
}

/// The whole client construction, with the proxy passed in rather than read
/// from the environment.
///
/// `build` exists to supply `configured_proxy_url()`; the split is what lets
/// the tests below drive the proxy branch itself. Reading the env var inside
/// the tests instead would race, because env vars are process-global and the
/// rest of this binary's tests run in parallel against the same process.
fn build_with(proxy: Option<&str>, timeout: Option<Duration>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder().connect_timeout(CONNECT_TIMEOUT);
    if let Some(t) = timeout {
        builder = builder.timeout(t);
    }
    if let Some(url) = proxy {
        match reqwest::Proxy::all(url) {
            Ok(proxy) => builder = builder.proxy(proxy),
            // The scheme passed `has_allowed_scheme`, so this only fires on a
            // malformed authority (e.g. a bad host/port). Degrade to a direct
            // connection rather than take all Telegram access down over a
            // proxy typo — the operator still has TELEGRAM_API_BASE_URL logs.
            Err(e) => {
                tracing::error!("[TELEGRAM API] invalid TELEGRAM_PROXY_URL, ignoring: {e}");
            },
        }
    }
    // Mirrors the fallback `TelegramTokenClient` already used: building a
    // rustls-backed client only fails on a broken TLS backend, never here.
    builder.build().unwrap_or_default()
}

/// Shared client for calls with no total-request timeout: uploads and chunk
/// downloads that can legitimately run for minutes.
pub fn client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| build(None)).clone()
}

/// Shared client for calls that must give up instead of hanging forever.
///
/// For short setup-time probes (`getMe`, `getChat`, …) against a host that
/// may be unreachable. Cached separately from [`client`] because it carries a
/// different timeout contract; callers that need a different bound should add
/// their own cached function here rather than fight this one over a shared
/// instance.
pub fn client_with_timeout(timeout: Duration) -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| build(Some(timeout))).clone()
}

#[cfg(test)]
mod tests {
    use std::{io::Read, net::TcpListener, time::Duration};

    use super::{build_with, has_allowed_scheme};

    #[test]
    fn accepts_every_documented_scheme() {
        for scheme in ["http://", "https://", "socks5://", "socks5h://"] {
            assert!(has_allowed_scheme(&format!("{scheme}proxy.example:1080")), "{scheme}");
        }
    }

    #[test]
    fn rejects_unknown_scheme() {
        assert!(!has_allowed_scheme("ftp://proxy.example:21"));
        assert!(!has_allowed_scheme("proxy.example:1080"));
    }

    /// Proves the client actually routes through the configured proxy address
    /// (not just that the builder didn't error): with an HTTP proxy set, a
    /// plain-HTTP request is sent to the proxy itself, using the absolute-URI
    /// request line RFC 7230 requires for proxied requests.
    #[tokio::test]
    async fn routes_through_configured_http_proxy() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let accept = tokio::task::spawn_blocking(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 512];
            let n = socket.read(&mut buf).unwrap_or(0);
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });

        let proxy = format!("http://{addr}");
        let client = build_with(Some(&proxy), None);
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            client.get("http://example.invalid/ping").send(),
        )
        .await;

        let request_line = tokio::time::timeout(Duration::from_secs(5), accept)
            .await
            .expect("proxy listener accepted a connection")
            .unwrap();
        assert!(
            request_line.starts_with("GET http://example.invalid/ping"),
            "expected an absolute-URI proxied request, got: {request_line}"
        );
    }

    /// Without a proxy configured, the client dials the target directly —
    /// the request line is origin-form, not the absolute-URI form a proxy
    /// would receive.
    #[tokio::test]
    async fn dials_directly_when_no_proxy_configured() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let accept = tokio::task::spawn_blocking(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 512];
            let n = socket.read(&mut buf).unwrap_or(0);
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });

        let client = build_with(None, None);
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            client.get(format!("http://{addr}/ping")).send(),
        )
        .await;

        let request_line = tokio::time::timeout(Duration::from_secs(5), accept)
            .await
            .expect("target listener accepted a connection")
            .unwrap();
        assert!(request_line.starts_with("GET /ping"), "got: {request_line}");
    }

    /// A proxy URL with an accepted scheme but an unusable authority must not
    /// take Telegram access down: `build_with` logs and falls back to a direct
    /// connection, so the request still reaches the target.
    #[tokio::test]
    async fn a_malformed_proxy_authority_falls_back_to_a_direct_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let accept = tokio::task::spawn_blocking(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 512];
            let n = socket.read(&mut buf).unwrap_or(0);
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });

        let client = build_with(Some("http://"), None);
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            client.get(format!("http://{addr}/ping")).send(),
        )
        .await;

        let request_line = tokio::time::timeout(Duration::from_secs(5), accept)
            .await
            .expect("target listener accepted a direct connection")
            .unwrap();
        assert!(request_line.starts_with("GET /ping"), "got: {request_line}");
    }
}
