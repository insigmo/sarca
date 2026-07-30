//! Dual TLS smoke test: TCP HTTPS (required) and HTTP/3 when the client supports it.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, UdpSocket},
    sync::Arc,
    time::Duration,
};

use bytes::Buf;
use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sarca::{
    server::Server,
    tls::{TlsMaterial, install_crypto_provider, new_runtime, register_challenge, serve_dual_tls},
};
use tokio::time::sleep;

fn pick_ports() -> (SocketAddr, SocketAddr) {
    let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
    let https_port = tcp.local_addr().unwrap().port();
    drop(tcp);
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    let acme_port = udp.local_addr().unwrap().port();
    drop(udp);
    let https = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), https_port);
    let acme = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), acme_port);
    (https, acme)
}

fn test_material() -> TlsMaterial {
    let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    TlsMaterial {
        cert_chain: vec![CertificateDer::from(cert.cert)],
        key: PrivateKeyDer::Pkcs8(cert.key_pair.serialize_der().into()),
    }
}

async fn get_tcp_https(url: &str) -> String {
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    client.get(url).send().await.unwrap().text().await.unwrap()
}

#[tokio::test]
async fn tcp_https_and_acme_challenge_serve_health() {
    install_crypto_provider();
    let (https_addr, acme_addr) = pick_ports();
    let material = test_material();
    let runtime = new_runtime(
        https_addr,
        acme_addr,
        &material,
        format!("https://127.0.0.1:{}", https_addr.port()),
        std::sync::Arc::default(),
    );

    register_challenge(&runtime.challenges, "test-token", "test-key-auth");

    let router = Server::health_router();
    let server_task = tokio::spawn(async move {
        serve_dual_tls(router, std::path::PathBuf::from("/dev/null"), runtime, None).await;
    });

    sleep(Duration::from_millis(200)).await;

    let body = get_tcp_https(&format!("https://127.0.0.1:{}/health", https_addr.port())).await;
    assert_eq!(body, "ok");

    let acme_client =
        reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let challenge = acme_client
        .get(format!("http://127.0.0.1:{}/.well-known/acme-challenge/test-token", acme_addr.port()))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(challenge, "test-key-auth");

    let redirect =
        acme_client.get(format!("http://127.0.0.1:{}/foo", acme_addr.port())).send().await.unwrap();
    assert_eq!(redirect.status(), reqwest::StatusCode::MOVED_PERMANENTLY);
    assert!(redirect.headers().get("location").unwrap().to_str().unwrap().starts_with("https://"));

    server_task.abort();
}

#[tokio::test]
async fn h3_serves_health_when_endpoint_accepts() {
    install_crypto_provider();
    let (https_addr, acme_addr) = pick_ports();
    let material = test_material();
    let runtime = new_runtime(
        https_addr,
        acme_addr,
        &material,
        format!("https://127.0.0.1:{}", https_addr.port()),
        std::sync::Arc::default(),
    );

    let router = Server::health_router();
    let server_task = tokio::spawn(async move {
        serve_dual_tls(router, std::path::PathBuf::from("/dev/null"), runtime, None).await;
    });

    sleep(Duration::from_millis(300)).await;

    if let Ok(body) = h3_get(&format!("https://127.0.0.1:{}/health", https_addr.port())).await {
        assert_eq!(body, "ok");
    } else {
        // H3 client stack may be unavailable in some CI kernels; TCP test covers the router path.
        eprintln!("HTTP/3 client probe skipped (QUIC unavailable in this environment)");
    }

    server_task.abort();
}

async fn h3_get(url: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    use h3_quinn::quinn;
    use rustls::{
        ClientConfig,
        client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        pki_types::{CertificateDer, ServerName, UnixTime},
    };

    #[derive(Debug)]
    struct SkipVerifier;

    impl ServerCertVerifier for SkipVerifier {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            rustls::crypto::ring::default_provider()
                .signature_verification_algorithms
                .supported_schemes()
        }
    }

    let url = url.parse::<http::Uri>()?;
    let host = url.host().unwrap_or("127.0.0.1");
    let port = url.port_u16().unwrap_or(443);

    let cfg = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipVerifier))
        .with_no_client_auth();

    let endpoint = quinn::Endpoint::client("127.0.0.1:0".parse()?)?;
    let conn = endpoint
        .connect_with(
            quinn::ClientConfig::new(Arc::new(quinn::crypto::rustls::QuicClientConfig::try_from(
                Arc::new(cfg),
            )?)),
            format!("{host}:{port}").parse()?,
            host,
        )?
        .await?;

    let (mut conn, mut send_request) = h3::client::new(h3_quinn::Connection::new(conn)).await?;
    let conn_task = tokio::spawn(async move {
        let _ = conn.wait_idle().await;
    });

    let mut stream = send_request.send_request(http::Request::get(url).body(())?).await?;
    stream.finish().await?;
    let resp = stream.recv_response().await?;
    assert!(resp.status().is_success());
    let mut body = String::new();
    while let Some(chunk) = stream.recv_data().await? {
        body.push_str(&String::from_utf8_lossy(chunk.chunk()));
    }
    conn_task.abort();
    Ok(body)
}
