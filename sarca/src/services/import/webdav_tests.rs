use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::Response,
};

use super::*;

/// Shaped like a Nextcloud answer: `d:` prefix, the listed folder first,
/// percent-encoded hrefs, an entity in a name, and a 404 propstat for the
/// properties a folder does not have.
const NEXTCLOUD: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:s="http://sabredav.org/ns" xmlns:oc="http://owncloud.org/ns">
 <d:response>
  <d:href>/remote.php/dav/files/alice/Photos/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype>
   <d:getlastmodified>Tue, 20 Oct 2026 10:00:00 GMT</d:getlastmodified></d:prop>
   <d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  <d:propstat><d:prop><d:getcontentlength/></d:prop>
   <d:status>HTTP/1.1 404 Not Found</d:status></d:propstat>
 </d:response>
 <d:response>
  <d:href>/remote.php/dav/files/alice/Photos/Summer%202026/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop>
   <d:status>HTTP/1.1 200 OK</d:status></d:propstat>
 </d:response>
 <d:response>
  <d:href>/remote.php/dav/files/alice/Photos/Tom%20&amp;%20Jerry.jpg</d:href>
  <d:propstat><d:prop><d:resourcetype/>
   <d:getcontentlength>12345</d:getcontentlength>
   <d:getlastmodified>Wed, 21 Oct 2026 08:30:00 GMT</d:getlastmodified></d:prop>
   <d:status>HTTP/1.1 200 OK</d:status></d:propstat>
 </d:response>
</d:multistatus>"#;

#[test]
fn parses_a_nextcloud_listing() {
    let entries = parse_multistatus(NEXTCLOUD).unwrap();
    assert_eq!(entries.len(), 3);
    assert!(entries[0].is_dir);
    assert_eq!(entries[0].size, 0);
    assert!(entries[1].is_dir);
    assert_eq!(entries[2].href, "/remote.php/dav/files/alice/Photos/Tom%20&%20Jerry.jpg");
    assert!(!entries[2].is_dir);
    assert_eq!(entries[2].size, 12345);
    assert_eq!(
        entries[2].mtime.map(|t| t.to_rfc3339()).as_deref(),
        Some("2026-10-21T08:30:00+00:00")
    );
}

#[test]
fn root_url_is_normalised_and_credentials_in_it_are_refused() {
    assert_eq!(
        parse_root_url(" https://cloud.example.com/remote.php/webdav?x=1 ").unwrap().as_str(),
        "https://cloud.example.com/remote.php/webdav/"
    );
    assert!(parse_root_url("https://alice:secret@cloud.example.com/dav/").is_err());
    assert!(parse_root_url("ftp://cloud.example.com/").is_err());
    assert!(parse_root_url("not a url").is_err());
}

fn status(code: StatusCode) -> Response {
    Response::builder().status(code).body(Body::empty()).unwrap()
}

fn dav_entry(href: &str, dir: bool, size: u64) -> String {
    let props = if dir {
        "<D:resourcetype><D:collection/></D:resourcetype>".to_owned()
    } else {
        format!("<D:resourcetype/><D:getcontentlength>{size}</D:getcontentlength>")
    };
    format!(
        "<D:response><D:href>{href}</D:href><D:propstat><D:prop>{props}</D:prop>\
         <D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"
    )
}

/// A WebDAV server with the folder `Photos/` holding `a b.txt` and the
/// sub-folder `Sub/` with `c.txt`; only `alice`/`pw` gets in.
async fn handle(method: Method, uri: Uri, headers: HeaderMap) -> Response {
    // base64("alice:pw")
    let authorized = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == "Basic YWxpY2U6cHc=");
    if !authorized {
        return status(StatusCode::UNAUTHORIZED);
    }
    let path = uri.path();
    if method.as_str() == "PROPFIND" {
        assert_eq!(headers.get("depth").and_then(|v| v.to_str().ok()), Some("1"));
        let entries = match path {
            "/dav/Photos/" => {
                [
                    dav_entry("/dav/Photos/", true, 0),
                    dav_entry("/dav/Photos/a%20b.txt", false, 5),
                    dav_entry("/dav/Photos/Sub/", true, 0),
                ]
                .concat()
            },
            "/dav/Photos/Sub/" => {
                [
                    dav_entry("/dav/Photos/Sub/", true, 0),
                    dav_entry("/dav/Photos/Sub/c.txt", false, 3),
                ]
                .concat()
            },
            _ => return status(StatusCode::NOT_FOUND),
        };
        let body = format!("<D:multistatus xmlns:D=\"DAV:\">{entries}</D:multistatus>");
        return Response::builder()
            .status(StatusCode::MULTI_STATUS)
            .body(Body::from(body))
            .unwrap();
    }
    let body = match path {
        "/dav/Photos/a%20b.txt" => "hello",
        "/dav/Photos/Sub/c.txt" => "abc",
        _ => return status(StatusCode::NOT_FOUND),
    };
    Response::builder().body(Body::from(body)).unwrap()
}

async fn fake_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, Router::new().fallback(handle)).await.unwrap();
    });
    format!("http://{addr}/dav/Photos")
}

#[tokio::test]
async fn lists_folders_and_downloads_files_from_a_server() {
    let base = fake_server().await;
    let client =
        WebDavClient::new(parse_root_url(&base).unwrap(), "alice".into(), "pw".into()).unwrap();

    let mut root = client.list("").await.unwrap();
    root.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    assert_eq!(
        root.iter().map(|e| (e.rel_path.as_str(), e.is_dir, e.size)).collect::<Vec<_>>(),
        [("Sub", true, 0), ("a b.txt", false, 5)]
    );
    let sub = client.list("Sub").await.unwrap();
    assert_eq!(sub.len(), 1);
    assert_eq!(sub[0].rel_path, "Sub/c.txt");

    assert_eq!(client.get("a b.txt").await.unwrap().text().await.unwrap(), "hello");
    assert_eq!(client.get("Sub/c.txt").await.unwrap().text().await.unwrap(), "abc");
    assert!(client.list("missing").await.is_err());
}

#[tokio::test]
async fn a_wrong_password_is_reported_as_such() {
    let base = fake_server().await;
    let client =
        WebDavClient::new(parse_root_url(&base).unwrap(), "alice".into(), "nope".into()).unwrap();
    let err = client.list("").await.unwrap_err();
    assert!(err.is_auth(), "{err}");
    assert!(!err.to_string().contains("nope"));
}
