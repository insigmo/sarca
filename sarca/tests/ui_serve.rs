//! Integration: static UI serving without a database.

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use std::fs;
use tower::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};

fn temp_ui() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("sarca-it-ui-{}", uuid::Uuid::new_v4()));
    let ui = root.join("ui");
    fs::create_dir_all(ui.join("assets")).unwrap();
    fs::write(
        ui.join("index.html"),
        r#"<!doctype html><html><script src="/assets/app.js"></script></html>"#,
    )
    .unwrap();
    fs::write(ui.join("assets/app.js"), "console.log('ok')").unwrap();
    ui
}

fn ui_router(ui: &std::path::Path) -> Router {
    Router::new()
        .nest_service("/assets", ServeDir::new(ui.join("assets")))
        .fallback_service(ServeFile::new(ui.join("index.html")))
}

#[tokio::test]
async fn root_returns_index_html() {
    let ui = temp_ui();
    let app = ui_router(&ui);
    let res = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = hyper::body::to_bytes(res.into_body()).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("<!doctype html>") || text.contains("<html"));
    let _ = fs::remove_dir_all(ui.parent().unwrap());
}

#[tokio::test]
async fn spa_fallback_serves_index() {
    let ui = temp_ui();
    let app = ui_router(&ui);
    let res = app
        .oneshot(
            Request::builder()
                .uri("/storages/abc")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = hyper::body::to_bytes(res.into_body()).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("app.js"));
    let _ = fs::remove_dir_all(ui.parent().unwrap());
}

#[tokio::test]
async fn assets_are_served() {
    let ui = temp_ui();
    let app = ui_router(&ui);
    let res = app
        .oneshot(
            Request::builder()
                .uri("/assets/app.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = hyper::body::to_bytes(res.into_body()).await.unwrap();
    assert_eq!(&body[..], b"console.log('ok')");
    let _ = fs::remove_dir_all(ui.parent().unwrap());
}
