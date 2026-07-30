use std::{
    io::Cursor,
    net::SocketAddr,
    sync::{Arc, RwLock},
};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use h3_quinn::quinn;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig;
use hyper_util::service::TowerToHyperService;
use tokio_rustls::TlsAcceptor;

use super::{CertStore, TlsIdentity, TlsError};

/// In-memory ACME http-01 challenge tokens (`token` → key authorization).
pub type ChallengeStore = Arc<RwLock<std::collections::HashMap<String, String>>>;

/// Loaded or generated TLS key material.
pub struct TlsMaterial {
    pub cert_chain: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
}

/// Runtime TLS settings shared by TCP HTTPS and QUIC listeners.
#[derive(Clone)]
pub struct TlsRuntime {
    pub https_addr: SocketAddr,
    pub acme_addr: SocketAddr,
    pub challenges: ChallengeStore,
    pub https_redirect_base: String,
    server_config: Arc<RwLock<Arc<ServerConfig>>>,
    quinn_config: quinn::ServerConfig,
}

const QUIC_ALPN: &[&[u8]] = &[b"h3"];
const TCP_ALPN: &[&[u8]] = &[b"h2", b"http/1.1"];

/// Load PEM from [`CertStore`] or generate a self-signed cert for first boot.
pub async fn load_or_generate_material(
    store: &CertStore,
    identity: Option<&TlsIdentity>,
) -> Result<TlsMaterial, TlsError> {
    if let (Some(cert_pem), Some(key_pem)) = (
        store.load_cert().await.map_err(|e| TlsError::Io(e.to_string()))?,
        store.load_key().await.map_err(|e| TlsError::Io(e.to_string()))?,
    ) {
        return parse_pem_material(&cert_pem, &key_pem);
    }

    let id = identity.ok_or(TlsError::NoIdentityForSelfSigned)?;
    let (cert_pem, key_pem, material) = generate_self_signed_pem(id)?;
    store.save_cert(&cert_pem).await.map_err(|e| TlsError::Io(e.to_string()))?;
    store.save_key(&key_pem).await.map_err(|e| TlsError::Io(e.to_string()))?;
    Ok(material)
}

pub fn generate_self_signed(identity: &TlsIdentity) -> Result<TlsMaterial, TlsError> {
    generate_self_signed_pem(identity).map(|(_, _, material)| material)
}

fn generate_self_signed_pem(
    identity: &TlsIdentity,
) -> Result<(String, String, TlsMaterial), TlsError> {
    let mut subject_alt_names = match identity {
        TlsIdentity::Dns(name) => vec![name.clone(), "localhost".to_owned()],
        TlsIdentity::Ip(ip) => vec![ip.to_string(), "127.0.0.1".to_owned(), "localhost".to_owned()],
    };
    subject_alt_names.sort_unstable();
    subject_alt_names.dedup();

    let cert = rcgen::generate_simple_self_signed(subject_alt_names).map_err(|_| TlsError::CertGen)?;
    let cert_pem = cert.cert.pem();
    let key_pem = cert.key_pair.serialize_pem();
    let key = PrivateKeyDer::Pkcs8(cert.key_pair.serialize_der().into());
    let cert_chain = vec![CertificateDer::from(cert.cert)];
    Ok((cert_pem, key_pem, TlsMaterial { cert_chain, key }))
}

pub fn parse_pem_material(cert_pem: &str, key_pem: &str) -> Result<TlsMaterial, TlsError> {
    let mut cert_reader = Cursor::new(cert_pem.as_bytes());
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| TlsError::InvalidPem)?;

    let mut key_reader = Cursor::new(key_pem.as_bytes());
    let key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|_| TlsError::InvalidPem)?
        .ok_or(TlsError::InvalidPem)?;

    if certs.is_empty() {
        return Err(TlsError::InvalidPem);
    }

    Ok(TlsMaterial { cert_chain: certs, key })
}

pub fn build_rustls_config(material: &TlsMaterial, alpn: &[&[u8]]) -> Arc<ServerConfig> {
    let mut config = ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
    .expect("protocols")
    .with_no_client_auth()
    .with_single_cert(material.cert_chain.clone(), material.key.clone_key())
    .expect("valid tls material");
    config.alpn_protocols = alpn.iter().map(|p| (*p).to_vec()).collect();
    config.max_early_data_size = u32::MAX;
    Arc::new(config)
}

pub fn build_quinn_config(rustls: Arc<ServerConfig>) -> quinn::ServerConfig {
    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(rustls).expect("quic tls"),
    ));
    server_config.migration(true);
    let transport = Arc::get_mut(&mut server_config.transport).expect("transport config");
    transport
        .max_concurrent_bidi_streams(256u32.into())
        .max_concurrent_uni_streams(256u32.into());
    server_config
}

pub fn new_runtime(
    https_addr: SocketAddr,
    acme_addr: SocketAddr,
    material: &TlsMaterial,
    https_redirect_base: String,
) -> TlsRuntime {
    let tcp = build_rustls_config(material, TCP_ALPN);
    let quic = build_rustls_config(material, QUIC_ALPN);
    TlsRuntime {
        https_addr,
        acme_addr,
        challenges: ChallengeStore::default(),
        https_redirect_base,
        server_config: Arc::new(RwLock::new(tcp)),
        quinn_config: build_quinn_config(quic),
    }
}

pub fn acme_router(challenges: ChallengeStore, https_redirect_base: String) -> Router {
    Router::new()
        .route(
            "/.well-known/acme-challenge/{token}",
            get(move |axum::extract::Path(token): axum::extract::Path<String>| {
                let challenges = challenges.clone();
                async move {
                    let map = challenges.read().expect("challenge lock");
                    match map.get(&token) {
                        Some(body) => Response::builder()
                            .status(StatusCode::OK)
                            .header("content-type", "text/plain")
                            .body(Body::from(body.clone()))
                            .unwrap(),
                        None => StatusCode::NOT_FOUND.into_response(),
                    }
                }
            }),
        )
        .fallback(move |req: Request<Body>| {
            let base = https_redirect_base.clone();
            async move {
                let path_and_query = req
                    .uri()
                    .path_and_query()
                    .map(|pq| pq.as_str())
                    .unwrap_or("/");
                let target = format!("{base}{path_and_query}");
                Response::builder()
                    .status(StatusCode::MOVED_PERMANENTLY)
                    .header(header::LOCATION, target)
                    .body(Body::empty())
                    .unwrap()
                    .into_response()
            }
        })
}

/// Install rustls crypto provider (required for rustls 0.23).
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Serve Axum over TCP TLS, QUIC HTTP/3, and ACME/redirect port concurrently.
pub async fn serve_dual_tls(router: Router, ui_dir: std::path::PathBuf, runtime: TlsRuntime) {
    let tcp_router = router.clone();
    let h3_router = router;
    let acme_router = acme_router(runtime.challenges.clone(), runtime.https_redirect_base.clone());

    let https_addr = runtime.https_addr;
    let acme_addr = runtime.acme_addr;
    let tcp_cfg = runtime.server_config.clone();
    let quinn_cfg = runtime.quinn_config.clone();

    eprintln!();
    eprintln!("========================================");
    eprintln!("  Sarca is running (TLS)");
    eprintln!("  HTTPS:   https://127.0.0.1:{}", https_addr.port());
    eprintln!("  HTTP/3:  https://127.0.0.1:{} (UDP)", https_addr.port());
    eprintln!("  ACME:    http://127.0.0.1:{}", acme_addr.port());
    eprintln!("  UI dir:  {}", ui_dir.display());
    eprintln!("========================================");
    eprintln!();

    let tcp_task = tokio::spawn(async move {
        if let Err(e) = serve_tcp_tls(https_addr, tcp_router, tcp_cfg).await {
            tracing::error!("TCP TLS listener failed: {e}");
        }
    });

    let h3_task = tokio::spawn(async move {
        if let Err(e) = serve_h3(https_addr, h3_router, quinn_cfg).await {
            tracing::error!("HTTP/3 listener failed: {e}");
        }
    });

    let acme_task = tokio::spawn(async move {
        if let Err(e) = serve_acme_http(acme_addr, acme_router).await {
            tracing::error!("ACME HTTP listener failed: {e}");
        }
    });

    let _ = tokio::join!(tcp_task, h3_task, acme_task);
}

async fn serve_tcp_tls(
    addr: SocketAddr,
    router: Router,
    cfg: Arc<RwLock<Arc<ServerConfig>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("TCP TLS listening on {addr}");

    loop {
        let (stream, remote) = listener.accept().await?;
        let tls_cfg = cfg.read().expect("tls lock").clone();
        let acceptor = TlsAcceptor::from(tls_cfg);
        let mut router = router.clone();
        tokio::spawn(async move {
            match acceptor.accept(stream).await {
                Ok(tls_stream) => {
                    let io = hyper_util::rt::TokioIo::new(tls_stream);
                    let service = TowerToHyperService::new(router);
                    if let Err(e) = hyper_util::server::conn::auto::Builder::new(
                        hyper_util::rt::TokioExecutor::new(),
                    )
                    .serve_connection(io, service)
                    .await
                    {
                        tracing::debug!("TCP TLS connection from {remote} ended: {e}");
                    }
                },
                Err(e) => tracing::debug!("TLS handshake from {remote} failed: {e}"),
            }
        });
    }
}

async fn serve_h3(
    addr: SocketAddr,
    router: Router,
    server_config: quinn::ServerConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let endpoint = quinn::Endpoint::server(server_config, addr)?;
    tracing::info!("HTTP/3 listening on {addr} (UDP)");

    while let Some(incoming) = endpoint.accept().await {
        let app = router.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_h3_connection(incoming, app).await {
                tracing::debug!("HTTP/3 connection error: {e}");
            }
        });
    }
    Ok(())
}

async fn handle_h3_connection(
    incoming: quinn::Incoming,
    app: Router,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let conn = incoming.await?;
    let mut h3_conn = h3::server::builder().build(h3_quinn::Connection::new(conn)).await?;
    loop {
        match h3_conn.accept().await {
            Ok(Some(resolver)) => {
                let app = app.clone();
                tokio::spawn(async move {
                    if let Err(e) = h3_axum::serve_h3_with_axum(app, resolver).await {
                        tracing::debug!("HTTP/3 request error: {e}");
                    }
                });
            },
            Ok(None) => break,
            Err(e) => {
                if h3_axum::is_graceful_h3_close(&e) {
                    break;
                }
                return Err(e.into());
            },
        }
    }
    Ok(())
}

async fn serve_acme_http(
    addr: SocketAddr,
    router: Router,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("ACME HTTP listening on {addr}");
    axum::serve(listener, router).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_signed_roundtrip_parse() {
        let id = TlsIdentity::Dns("test.local".into());
        let (cert_pem, key_pem, _) = generate_self_signed_pem(&id).unwrap();
        let parsed = parse_pem_material(&cert_pem, &key_pem).unwrap();
        assert!(!parsed.cert_chain.is_empty());
    }
}
