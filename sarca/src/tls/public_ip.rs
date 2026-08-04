//! Public IP discovery used when `TLS_HOSTNAME` is unset.
//!
//! Sarca prefers to always serve HTTPS (TCP + HTTP/3), so an operator who never
//! configured a domain still gets a certificate identity: the machine's public
//! IP. Let's Encrypt issues short-lived certificates for IP SANs, so an IP
//! identity is enough for a real, browser-trusted HTTP/3 endpoint.

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    time::Duration,
};

use super::{SharedIdentity, TlsIdentity, TlsRuntime};

/// Plain-text "what is my IP" endpoints, tried in order.
const LOOKUP_ENDPOINTS: [&str; 3] =
    ["https://ifconfig.me/ip", "https://api.ipify.org", "https://icanhazip.com"];

/// Per-endpoint budget. Startup must not hang behind a dead network.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

/// The externally visible IP, or `None` when it cannot be determined.
///
/// Only an external lookup counts. A local interface address is useless here:
/// inside Docker it is the bridge address (`172.19.0.2`), and Let's Encrypt
/// rejects reserved ranges outright, so an identity built from it would fail
/// issuance and break HTTP/3 rather than enable it.
pub async fn detect_public_ip() -> Option<IpAddr> {
    let ip = lookup_public_ip().await;
    if ip.is_none() {
        tracing::warn!("no public IP could be determined from {LOOKUP_ENDPOINTS:?}");
    }
    ip
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

/// How often the watcher re-checks the public address.
const WATCH_INTERVAL: Duration = Duration::from_mins(15);

/// Track the public address and re-issue the certificate when it moves.
///
/// Only spawned when `TLS_HOSTNAME` is unset, so the certificate identity is
/// the detected IP: when the machine changes address, the existing certificate
/// no longer matches and both HTTP/3 and TCP TLS break until a new one is
/// issued for the new address.
pub fn spawn_public_ip_watch(identity: SharedIdentity, runtime: TlsRuntime) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(WATCH_INTERVAL).await;

            let Some(ip) = detect_public_ip().await else {
                tracing::debug!("public IP re-check failed; keeping the current TLS identity");
                continue;
            };

            let current = identity.read().expect("identity lock").clone();
            if matches!(current, TlsIdentity::Ip(cur) if cur == ip) {
                continue;
            }

            tracing::warn!("public IP changed ({current:?} -> {ip}); re-issuing certificate");
            *identity.write().expect("identity lock") = TlsIdentity::Ip(ip);
            runtime.request_renewal("public IP changed");
        }
    });
}

/// Trim the response and accept it only if the whole body is one globally
/// routable IP address.
fn parse_ip_body(body: &str) -> Option<IpAddr> {
    let ip = body.trim().parse::<IpAddr>().ok()?;
    is_globally_routable(ip).then_some(ip)
}

/// Whether Let's Encrypt would consider this address issuable.
///
/// It refuses every reserved block ("IP address is in a reserved address
/// block"), so filtering here turns a doomed ACME order into a clean fallback.
fn is_globally_routable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_global_v4(v4),
        IpAddr::V6(v6) => is_global_v6(v6),
    }
}

fn is_global_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || a == 0
        || a >= 240
        // 100.64.0.0/10 carrier-grade NAT
        || (a == 100 && (64..128).contains(&b))
        // 192.0.0.0/24 IETF protocol assignments
        || (a == 192 && b == 0 && ip.octets()[2] == 0)
        // 198.18.0.0/15 benchmarking
        || (a == 198 && (b == 18 || b == 19)))
}

fn is_global_v6(ip: Ipv6Addr) -> bool {
    let head = ip.segments()[0];
    !(ip.is_loopback()
        || ip.is_unspecified()
        // fc00::/7 unique local
        || head & 0xFE00 == 0xFC00
        // fe80::/10 link local
        || head & 0xFFC0 == 0xFE80
        // 2001:db8::/32 documentation
        || (head == 0x2001 && ip.segments()[1] == 0x0DB8))
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;

    #[test]
    fn parses_ipv4_with_trailing_newline() {
        assert_eq!(
            parse_ip_body("93.184.216.34\n"),
            Some(IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)))
        );
    }

    #[test]
    fn parses_ipv6() {
        assert_eq!(
            parse_ip_body("  2606:4700::1111  "),
            Some(IpAddr::V6(Ipv6Addr::new(0x2606, 0x4700, 0, 0, 0, 0, 0, 0x1111)))
        );
    }

    #[test]
    fn rejects_html_error_pages() {
        assert_eq!(parse_ip_body("<html>rate limited</html>"), None);
        assert_eq!(parse_ip_body(""), None);
    }

    /// Let's Encrypt rejects these, so they must never become a TLS identity.
    #[test]
    fn rejects_reserved_addresses() {
        for body in [
            "172.19.0.2",   // docker bridge, RFC1918
            "10.0.0.5",     //
            "192.168.1.10", //
            "127.0.0.1",    //
            "169.254.1.1",  // link local
            "100.100.0.1",  // CGNAT
            "198.18.0.1",   // benchmarking
            "fd00::1",      // unique local
            "fe80::1",      // link local
            "2001:db8::1",  // documentation
        ] {
            assert_eq!(parse_ip_body(body), None, "{body} must be rejected");
        }
    }

    #[test]
    fn accepts_routable_addresses() {
        assert!(parse_ip_body("8.8.8.8").is_some());
    }
}
