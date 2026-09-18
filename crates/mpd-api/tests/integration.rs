use std::time::Duration;

use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use mpd_api::{router, spawn_cache_invalidation, AppState};
use mpd_client::{MpdClient, MpdConfig};
use mpd_mock::{MockMpd, MockState};
use serde_json::{json, Value};
use tower::ServiceExt;

type FieldList = Vec<(String, String)>;

fn fields(pairs: &[(&str, &str)]) -> FieldList {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

async fn app_with(initial: MockState) -> (Router, AppState, MockMpd) {
    let mock = MockMpd::start(initial).await;
    let (host, port) = mock.host_port();
    let client = MpdClient::connect(MpdConfig::new(host, port, None)).await;
    let state = AppState::new(client, Duration::from_secs(60));
    let router = router(state.clone());
    (router, state, mock)
}

async fn app() -> (Router, AppState, MockMpd) {
    app_with(MockState::default()).await
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header(header::HOST, "localhost")
        .body(Body::empty())
        .expect("valid request")
}

fn get_with(uri: &str, header_name: axum::http::HeaderName, value: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header(header::HOST, "localhost")
        .header(header_name, value)
        .body(Body::empty())
        .expect("valid request")
}

fn post_json(uri: &str, value: &Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(value.to_string()))
        .expect("valid request")
}

fn post_raw(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("valid request")
}

async fn call(router: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, axum::body::Bytes) {
    let resp = router.clone().oneshot(req).await.expect("router responds");
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    (status, headers, bytes)
}

fn as_json(bytes: &axum::body::Bytes) -> Value {
    serde_json::from_slice(bytes).expect("response is JSON")
}

#[tokio::test]
async fn status_returns_snapshot() {
    let (router, _, mock) = app().await;
    mock.set_status(fields(&[
        ("state", "play"),
        ("volume", "70"),
        ("playlist", "3"),
    ]))
    .await;
    mock.set_currentsong(Some(fields(&[
        ("file", "a/one.flac"),
        ("Title", "One"),
        ("Artist", "A"),
    ])))
    .await;

    let (status, _, body) = call(&router, get("/api/status")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    assert_eq!(value["status"]["state"], "play");
    assert_eq!(value["status"]["volume"], 70);
    assert_eq!(value["status"]["playlist_version"], 3);
    assert_eq!(value["song"]["file"], "a/one.flac");
    assert_eq!(value["song"]["artist"], "A");
}

#[tokio::test]
async fn playlist_returns_songs() {
    let (router, _, mock) = app().await;
    mock.set_playlist(vec![
        fields(&[("file", "a/one.flac"), ("Id", "0")]),
        fields(&[("file", "a/two.flac"), ("Id", "1")]),
    ])
    .await;

    let (status, _, body) = call(&router, get("/api/playlist")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    assert_eq!(value.as_array().expect("array").len(), 2);
    assert_eq!(value[0]["file"], "a/one.flac");
}

#[tokio::test]
async fn playlist_rejects_bad_range() {
    let (router, _state, _mock) = app().await;
    let (status, _, body) = call(&router, get("/api/playlist?start=abc")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(as_json(&body)["error"].is_string());
}

#[tokio::test]
async fn browse_returns_directories_and_files() {
    let (router, _, mock) = app().await;
    mock.set_lsinfo(fields(&[
        ("directory", "/Band"),
        ("songcount", "2"),
        ("playtime", "100"),
        ("file", "/loose.flac"),
        ("Title", "Loose"),
        ("Time", "10"),
    ]))
    .await;

    let (status, _, body) = call(&router, get("/api/browse?path=/")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    assert_eq!(value["directories"].as_array().expect("array").len(), 1);
    assert_eq!(value["directories"][0]["path"], "/Band");
    assert_eq!(value["files"].as_array().expect("array").len(), 1);
    assert_eq!(value["songcount"], 3);
}

#[tokio::test]
async fn list_returns_values_and_validates_type() {
    let (router, _, mock) = app().await;
    mock.set_list(fields(&[("artist", "A"), ("artist", "B")]))
        .await;

    let (status, _, body) = call(&router, get("/api/list?type=artist")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    assert_eq!(value.as_array().expect("array").len(), 2);
    assert_eq!(value[0], "A");

    let (status, _, _) = call(&router, get("/api/list")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _, _) = call(&router, get("/api/list?type=bogus")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn search_returns_songs() {
    let (router, _, mock) = app().await;
    mock.set_search(vec![fields(&[("file", "hit.flac"), ("Title", "Hit")])])
        .await;

    let (status, _, body) = call(&router, get("/api/search?q=hit")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    assert_eq!(value.as_array().expect("array").len(), 1);
    assert_eq!(value[0]["file"], "hit.flac");
}

#[tokio::test]
async fn search_exact_returns_songs() {
    let (router, _, mock) = app().await;
    mock.set_search(vec![fields(&[("file", "exact.flac"), ("Artist", "A")])])
        .await;

    let (status, _, body) = call(&router, get("/api/search?exact=1&artist=A")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    assert_eq!(value.as_array().expect("array").len(), 1);
    assert_eq!(value[0]["file"], "exact.flac");
}

#[tokio::test]
async fn capabilities_reports_mock_commands() {
    let initial = MockState {
        commands: vec!["play".to_string(), "pause".to_string()],
        ..Default::default()
    };
    let (router, _state, _mock) = app_with(initial).await;

    let (status, _, body) = call(&router, get("/api/capabilities")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    assert_eq!(value["commands"].as_array().expect("array").len(), 2);
}

#[tokio::test]
async fn playback_endpoints_succeed() {
    let (router, _state, _mock) = app().await;

    for (uri, body) in [
        ("/api/play", json!({"position": 3})),
        ("/api/pause", json!({"state": true})),
        ("/api/stop", json!({})),
        ("/api/next", json!({})),
        ("/api/previous", json!({})),
        ("/api/seek", json!({"time": 12.5})),
        ("/api/volume", json!({"value": 42})),
        ("/api/options", json!({"random": true, "repeat": false})),
    ] {
        let (status, _, _) = call(&router, post_json(uri, &body)).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "expected 204 for {uri}");
    }
}

#[tokio::test]
async fn playback_rejects_invalid_json() {
    let (router, _state, _mock) = app().await;
    let (status, _, body) = call(&router, post_raw("/api/play", "not json")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(as_json(&body)["error"].is_string());
}

#[tokio::test]
async fn queue_add_and_management_succeed() {
    let (router, _state, _mock) = app().await;

    let add = json!({
        "targets": [
            {"path": "a/one.flac"},
            {"artist": "A"},
            {"album": "B"},
            {"albumartist": "C"},
            {"genre": "D"}
        ],
        "play": true
    });
    let (status, _, _) = call(&router, post_json("/api/queue/add", &add)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call(
        &router,
        post_json("/api/queue/remove", &json!({"ids": [0, 1]})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call(&router, post_raw("/api/queue/clear", "")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call(
        &router,
        post_json("/api/queue/move", &json!({"id": 2, "to": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = call(&router, post_raw("/api/queue/shuffle", "")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn queue_add_rejects_unknown_target() {
    let (router, _state, _mock) = app().await;
    let (status, _, body) = call(
        &router,
        post_json("/api/queue/add", &json!({"targets": [{"bogus": "x"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(as_json(&body)["error"].is_string());
}

#[tokio::test]
async fn albumart_proxies_bytes_and_validates_etag() {
    let initial = MockState {
        art: Some((vec![1, 2, 3], "image/png".to_string())),
        ..Default::default()
    };
    let (router, _state, _mock) = app_with(initial).await;
    let uri = "/api/albumart?uri=album%2Fone.flac";

    let (status, headers, body) = call(&router, get(uri)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, axum::body::Bytes::copy_from_slice(&[1, 2, 3]));
    assert_eq!(
        headers.get(header::CONTENT_TYPE).expect("content-type"),
        "image/png"
    );
    let etag = headers
        .get(header::ETAG)
        .expect("etag")
        .to_str()
        .expect("utf8")
        .to_string();

    let (status, _, _) = call(&router, get_with(uri, header::IF_NONE_MATCH, &etag)).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn albumart_missing_returns_404() {
    let (router, _state, _mock) = app().await;
    let (status, _, body) = call(&router, get("/api/albumart?uri=none.flac")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(as_json(&body)["error"].is_string());
}

#[tokio::test]
async fn cache_is_cleared_on_database_change() {
    let initial = MockState {
        art: Some((vec![9, 9], "image/jpeg".to_string())),
        ..Default::default()
    };
    let (router, state, mock) = app_with(initial).await;
    spawn_cache_invalidation(state.clone());
    let uri = "/api/albumart?uri=cover.jpg";

    let (status, _, _) = call(&router, get(uri)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(state.cache.get_art("art:cover.jpg").is_some());

    mock.set_next_idle_change("database").await;
    for _ in 0..100 {
        if state.cache.get_art("art:cover.jpg").is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(state.cache.get_art("art:cover.jpg").is_none());
}
