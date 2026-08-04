//! Public IP discovery used when `TLS_HOSTNAME` is unset.
//!
//! Sarca prefers to always serve HTTPS (TCP + HTTP/3), so an operator who never
//! configured a domain still gets a certificate identity: the machine's public
//! IP. Let's Encrypt issues short-lived certificates for IP SANs, so an IP
//! identity is enough for a real, browser-trusted HTTP/3 endpoint.

use std::{net::IpAddr, time::Duration};

/// Plain-text "what is my IP" endpoints, tried in order.
const LOOKUP_ENDPOINTS: [&str; 3] =
    ["https://api.ipify.org", "https://icanhazip.com", "https://ifconfig.me/ip"];

/// Per-endpoint budget. Startup must not hang behind a dead network.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(3);

/// Best-effort public IP: external lookup first, then the local route source
/// address. `None` when the host is offline.
pub async fn detect_public_ip() -> Option<IpAddr> {
    if let Some(ip) = lookup_public_ip().await {
        return Some(ip);
    }
    tracing::warn!("public IP lookup failed; falling back to the local outbound interface address");
    local_outbound_ip()
}

/// Query the public-IP services until one answers with a parsable address.
async fn lookup_public_ip() -> Option<IpAddr> {
    let client = reqwest::Client::builder().timeout(LOOKUP_TIMEOUT).build().ok()?;
    for url in LOOKUP_ENDPOINTS {
        match client.get(url).send().await {
            Ok(resp) => {
                match resp.text().await {
                    Ok(body) => {
                        if let Some(ip) = parse_ip_body(&body) {
                            tracing::info!("detected public IP {ip} via {url}");
                            return Some(ip);
                        }
                        tracing::debug!("{url} returned an unparsable body");
                    },
                    Err(e) => tracing::debug!("{url} body read failed: {e}"),
                }
            },
            Err(e) => tracing::debug!("{url} request failed: {e}"),
        }
    }
    None
}

/// Source address the kernel would use for outbound traffic. UDP `connect` only
/// sets the socket's peer, so nothing is actually sent.
fn local_outbound_ip() -> Option<IpAddr> {
    use std::net::UdpSocket;

    let probe = |bind: &str, peer: &str| -> Option<IpAddr> {
        let sock = UdpSocket::bind(bind).ok()?;
        sock.connect(peer).ok()?;
        let ip = sock.local_addr().ok()?.ip();
        (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
    };

    probe("0.0.0.0:0", "1.1.1.1:80").or_else(|| probe("[::]:0", "[2606:4700:4700::1111]:80"))
}

/// Trim the response and accept it only if the whole body is one IP address.
fn parse_ip_body(body: &str) -> Option<IpAddr> {
    body.trim().parse::<IpAddr>().ok()
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;

    #[test]
    fn parses_ipv4_with_trailing_newline() {
        assert_eq!(parse_ip_body("203.0.113.7\n"), Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7))));
    }

    #[test]
    fn parses_ipv6() {
        assert_eq!(
            parse_ip_body("  2001:db8::1  "),
            Some(IpAddr::V6(Ipv6Addr::new(0x2001, 0x0DB8, 0, 0, 0, 0, 0, 1)))
        );
    }

    #[test]
    fn rejects_html_error_pages() {
        assert_eq!(parse_ip_body("<html>rate limited</html>"), None);
        assert_eq!(parse_ip_body(""), None);
    }
}
