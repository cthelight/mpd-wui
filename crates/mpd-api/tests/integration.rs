#![allow(clippy::unwrap_used)]

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use http_body_util::BodyExt;
use mpd_api::{AppState, router, spawn_cache_invalidation};
use mpd_client::{MpdClient, MpdConfig, MpdEvent};
use mpd_mock::{MockMpd, MockState};
use serde_json::{Value, json};
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

/// The `file` values of the track hits in a search response, in order.
fn hit_files(value: &Value) -> Vec<String> {
    let Some(hits) = value.as_array() else {
        return Vec::new();
    };
    hits.iter()
        .filter_map(|hit| hit.pointer("/song/file").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn status_returns_snapshot() {
    let (router, _, mock) = app().await;
    // Realistic MPD 0.23.x `status`: `time` is `<elapsed>:<total>`, playlist
    // length is `playlistlength`, crossfade is `xfade`.
    mock.set_status(fields(&[
        ("state", "play"),
        ("volume", "70"),
        ("playlist", "3"),
        ("playlistlength", "4"),
        ("xfade", "2"),
        ("time", "10:220"),
        ("elapsed", "10.0"),
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
    assert_eq!(
        value["status"]["songs"], 4,
        "playlistlength must map to songs"
    );
    assert_eq!(
        value["status"]["crossfade"], 2,
        "xfade must map to crossfade"
    );
    assert_eq!(
        value["status"]["time"], 220,
        "time must be the total after the colon"
    );
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
    // Real MPD `list artist` replies with the capitalized `Artist:` key.
    mock.set_list(fields(&[("Artist", "A"), ("Artist", "B")]))
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
    let hits = value.as_array().expect("array");
    // No artist/album on this song, so the only hit is the track itself.
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["kind"], "track");
    assert_eq!(hits[0]["song"]["file"], "hit.flac");
}

#[tokio::test]
async fn search_exact_returns_songs() {
    let (router, _, mock) = app().await;
    mock.set_search(vec![fields(&[("file", "exact.flac"), ("Artist", "A")])])
        .await;

    let (status, _, body) = call(&router, get("/api/search?artist=A")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    let hits = value.as_array().expect("array");
    // Exact-only (no free text) returns the whole item as track hits.
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["kind"], "track");
    assert_eq!(hits[0]["song"]["file"], "exact.flac");
}

#[tokio::test]
async fn search_filters_locally_across_fields_and_tags() {
    let (router, _, mock) = app().await;
    mock.set_search(vec![
        fields(&[
            ("file", "alpha/one.flac"),
            ("Artist", "Alpha"),
            ("Title", "One"),
            ("Genre", "Jazz"),
        ]),
        fields(&[
            ("file", "beta/two.flac"),
            ("Artist", "Beta"),
            ("Title", "Two"),
            ("Genre", "Rock"),
        ]),
        fields(&[
            ("file", "gamma/blue.flac"),
            ("Artist", "Gamma"),
            ("Title", "Blue Album"),
            ("Genre", "Folk"),
        ]),
    ])
    .await;

    // Free text fuzzy-matches artists, albums and tracks in one list. "beta"
    // surfaces both the artist "Beta" and the matching track.
    let (status, _, body) = call(&router, get("/api/search?q=beta")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    let hits = value.as_array().expect("array");
    assert_eq!(hits.len(), 2);
    assert!(
        hits.iter()
            .any(|h| h["kind"] == "track" && h["song"]["file"] == "beta/two.flac")
    );
    assert!(
        hits.iter()
            .any(|h| h["kind"] == "artist" && h["name"] == "Beta")
    );

    // Multi-tag exact constraints are ANDed (the case MPD's filter grammar broke on).
    let (status, _, body) = call(&router, get("/api/search?genre=rock&title=two")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(as_json(&body).as_array().expect("array").len(), 1);

    // Contradictory exact constraints match nothing.
    let (status, _, body) = call(&router, get("/api/search?genre=rock&title=one")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(as_json(&body).as_array().expect("array").len(), 0);

    // Free text + exact constraints combine (ranked within the constrained pool).
    let (status, _, body) = call(&router, get("/api/search?q=blue&artist=gamma")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    let hits = value.as_array().expect("array");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["kind"], "track");
    assert_eq!(hits[0]["song"]["file"], "gamma/blue.flac");

    // No query at all returns an empty list (and must not error).
    let (status, _, body) = call(&router, get("/api/search")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(as_json(&body).as_array().expect("array").len(), 0);
}

#[tokio::test]
async fn search_serves_stale_snapshot_until_background_refresh_lands() {
    let mock = MockMpd::start(MockState::default()).await;
    let (host, port) = mock.host_port();
    let client = MpdClient::connect(MpdConfig::new(host, port, None)).await;
    let state = AppState::new(client, Duration::from_millis(30));
    let router = router(state);

    // The library dump takes 250ms, well beyond the 30ms snapshot TTL.
    mock.set_search(vec![fields(&[("file", "a/one.flac"), ("Title", "One")])])
        .await;
    mock.set_command_delay("search", 250).await;

    // First load: no stale data, so the request waits for the dump.
    let (status, _, body) = call(&router, get("/api/search?q=one")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(hit_files(&as_json(&body)), vec!["a/one.flac".to_string()]);

    // The server-side library changes, then the TTL expires.
    mock.set_search(vec![fields(&[("file", "a/two.flac"), ("Title", "Two")])])
        .await;
    tokio::time::sleep(Duration::from_millis(60)).await;

    // The stale snapshot is served immediately and a background refresh
    // starts (it is still in flight for the next two requests).
    let (status, _, body) = call(&router, get("/api/search?q=one")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        hit_files(&as_json(&body)),
        vec!["a/one.flac".to_string()],
        "stale snapshot must be served before the refresh lands"
    );

    // A request during the in-flight refresh still gets the stale data.
    let (_, _, body) = call(&router, get("/api/search?q=one")).await;
    assert_eq!(hit_files(&as_json(&body)), vec!["a/one.flac".to_string()]);

    // Once the refresh lands, the new data is visible.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let (status, _, body) = call(&router, get("/api/search?q=two")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(hit_files(&as_json(&body)), vec!["a/two.flac".to_string()]);
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
async fn play_by_id_targets_stable_playlist_id() {
    let (router, _state, _mock) = app_with(MockState {
        playlist: vec![
            fields(&[("file", "a/one.flac"), ("Id", "10")]),
            fields(&[("file", "a/two.flac"), ("Id", "11")]),
        ],
        ..Default::default()
    })
    .await;

    let (status, _, _) = call(&router, post_json("/api/play", &json!({"id": 11}))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, body) = call(&router, get("/api/status")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    assert_eq!(value["song"]["file"], "a/two.flac");

    // Without `clear` the rest of the queue must be untouched.
    let (status, _, body) = call(&router, get("/api/playlist")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    assert_eq!(
        value.as_array().expect("array").len(),
        2,
        "plain play must keep the queue"
    );
}

#[tokio::test]
async fn play_clear_replaces_queue_with_that_song() {
    let initial = MockState {
        playlist: vec![
            fields(&[("file", "a/one.flac"), ("Id", "10")]),
            fields(&[("file", "a/two.flac"), ("Id", "11")]),
            fields(&[("file", "a/three.flac"), ("Id", "12")]),
        ],
        ..Default::default()
    };
    let (router, _state, _mock) = app_with(initial).await;

    let (status, _, _) = call(
        &router,
        post_json("/api/play", &json!({"id": 11, "clear": true})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, body) = call(&router, get("/api/playlist")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    let songs = value.as_array().expect("array");
    assert_eq!(songs.len(), 1, "the queue must hold only the played song");
    assert_eq!(songs[0]["file"], "a/two.flac");

    let (status, _, body) = call(&router, get("/api/status")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(as_json(&body)["song"]["file"], "a/two.flac");
}

#[tokio::test]
async fn play_clear_by_position() {
    let initial = MockState {
        playlist: vec![
            fields(&[("file", "a/one.flac"), ("Id", "10")]),
            fields(&[("file", "a/two.flac"), ("Id", "11")]),
            fields(&[("file", "a/three.flac"), ("Id", "12")]),
        ],
        ..Default::default()
    };
    let (router, _state, _mock) = app_with(initial).await;

    let (status, _, _) = call(
        &router,
        post_json("/api/play", &json!({"position": 2, "clear": true})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, body) = call(&router, get("/api/playlist")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    let songs = value.as_array().expect("array");
    assert_eq!(songs.len(), 1, "the queue must hold only the played song");
    assert_eq!(songs[0]["file"], "a/three.flac");
}

#[tokio::test]
async fn play_clear_unknown_id_rejected_before_wipe() {
    // The id is not in the queue: the request is a client error and the
    // queue must be left untouched.
    let initial = MockState {
        playlist: vec![
            fields(&[("file", "a/one.flac"), ("Id", "10")]),
            fields(&[("file", "a/two.flac"), ("Id", "11")]),
        ],
        ..Default::default()
    };
    let (router, _state, _mock) = app_with(initial).await;

    let (status, _, body) = call(
        &router,
        post_json("/api/play", &json!({"id": 999, "clear": true})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(as_json(&body)["error"], "no such song in queue");

    let (status, _, body) = call(&router, get("/api/playlist")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    let songs = value.as_array().expect("array");
    assert_eq!(songs.len(), 2, "the queue must be untouched");
    assert_eq!(songs[0]["file"], "a/one.flac");
    assert_eq!(songs[1]["file"], "a/two.flac");
}

/// The mock never emits `changed: options` in response to an option command,
/// simulating MPD's idle notification being lost (which happens when the idle
/// connection is busy fetching a snapshot for a previous change). The route
/// must push its own post-command snapshot so the WebSocket still reflects
/// the new option state.
#[tokio::test]
async fn options_change_pushes_snapshot_without_idle_notification() {
    let initial = MockState {
        status: fields(&[("state", "stop"), ("single", "0"), ("volume", "42")]),
        ..Default::default()
    };
    let (router, state, mock) = app_with(initial).await;
    let mut events = state.client.events();

    // Wait until the command connection has primed its snapshot (the
    // distinctive volume marks that), so the stream state is deterministic.
    for _ in 0..200 {
        if state.client.snapshot().status.volume == 42 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        state.client.snapshot().status.volume,
        42,
        "client must connect"
    );

    // Simulate MPD applying `single 1` (a real server applies it before the
    // next `status` read on the serialized command connection).
    mock.set_status(fields(&[
        ("state", "stop"),
        ("single", "1"),
        ("volume", "42"),
    ]))
    .await;

    let (status, _, _) = call(&router, post_json("/api/options", &json!({"single": true}))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let mut found = false;
    for _ in 0..400 {
        match tokio::time::timeout(Duration::from_millis(50), events.recv()).await {
            Ok(Ok(MpdEvent::Snapshot(snapshot))) => {
                if snapshot.status.single {
                    found = true;
                    break;
                }
            }
            Ok(Ok(_)) | Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
            Err(_) | Ok(Err(_)) => break,
        }
    }
    assert!(found, "expected a snapshot carrying single: true");
}

#[tokio::test]
async fn playback_rejects_invalid_json() {
    let (router, _state, _mock) = app().await;
    let (status, _, body) = call(&router, post_raw("/api/play", "not json")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(as_json(&body)["error"].is_string());
}

/// The mock's `searchadd` appends every group in `search` regardless of the
/// filter, so tests give it one group per collection tag they exercise.
fn wipe_state(playlist: Vec<FieldList>, search: Vec<FieldList>) -> MockState {
    MockState {
        playlist,
        search,
        lsinfo: fields(&[("file", "a/one.flac"), ("Title", "One")]),
        ..Default::default()
    }
}

#[tokio::test]
async fn queue_add_and_management_succeed() {
    let initial = wipe_state(
        vec![],
        vec![
            fields(&[("file", "s/a.flac"), ("Id", "900"), ("Artist", "A")]),
            fields(&[("file", "s/b.flac"), ("Id", "901"), ("Album", "B")]),
            fields(&[("file", "s/c.flac"), ("Id", "902"), ("AlbumArtist", "C")]),
            fields(&[("file", "s/d.flac"), ("Id", "903"), ("Genre", "D")]),
        ],
    );
    let (router, _state, _mock) = app_with(initial).await;

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

    // The wipe kept exactly the appended songs: 1 path + 4 search hits.
    let (status, _, body) = call(&router, get("/api/playlist")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    assert_eq!(value.as_array().expect("array").len(), 5);

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
async fn queue_add_play_replaces_queue_with_new_songs() {
    let initial = wipe_state(
        vec![
            fields(&[("file", "old/one.flac"), ("Id", "50")]),
            fields(&[("file", "old/two.flac"), ("Id", "51")]),
        ],
        vec![fields(&[
            ("file", "new/one.flac"),
            ("Id", "900"),
            ("Artist", "X"),
        ])],
    );
    let (router, _state, _mock) = app_with(initial).await;

    let (status, _, _) = call(
        &router,
        post_json(
            "/api/queue/add",
            &json!({"targets": [{"artist": "X"}], "play": true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, body) = call(&router, get("/api/playlist")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    let songs = value.as_array().expect("array");
    assert_eq!(songs.len(), 1, "old queue must be gone");
    assert_eq!(songs[0]["file"], "new/one.flac");
}

#[tokio::test]
async fn queue_add_play_empty_collection_rejected_before_wipe() {
    // No search results: the target would add nothing, so the request is a
    // client error and the queue must be left untouched.
    let initial = wipe_state(
        vec![fields(&[("file", "old/one.flac"), ("Id", "50")])],
        vec![],
    );
    let (router, _state, _mock) = app_with(initial).await;

    let (status, _, body) = call(
        &router,
        post_json(
            "/api/queue/add",
            &json!({"targets": [{"artist": "Nobody"}], "play": true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(as_json(&body)["error"], "no matching tracks");

    let (status, _, body) = call(&router, get("/api/playlist")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    let songs = value.as_array().expect("array");
    assert_eq!(songs.len(), 1);
    assert_eq!(songs[0]["file"], "old/one.flac");
}

#[tokio::test]
async fn queue_add_play_missing_path_rejected_before_wipe() {
    let initial = wipe_state(
        vec![fields(&[("file", "old/one.flac"), ("Id", "50")])],
        vec![],
    );
    let (router, _state, mock) = app_with(initial).await;
    mock.set_fail_commands(vec!["lsinfo".to_string()]).await;

    let (status, _, body) = call(
        &router,
        post_json(
            "/api/queue/add",
            &json!({"targets": [{"path": "missing.flac"}], "play": true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        as_json(&body)["error"]
            .as_str()
            .unwrap_or("")
            .contains("no such path")
    );

    let (status, _, body) = call(&router, get("/api/playlist")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    let songs = value.as_array().expect("array");
    assert_eq!(songs.len(), 1);
    assert_eq!(songs[0]["file"], "old/one.flac");
}

#[tokio::test]
async fn queue_add_play_failure_mid_add_preserves_old_queue() {
    // The first target adds a song, the second fails: the request must roll
    // back only what it appended, leaving the caller's old queue intact.
    let initial = wipe_state(
        vec![
            fields(&[("file", "old/one.flac"), ("Id", "50")]),
            fields(&[("file", "old/two.flac"), ("Id", "51")]),
        ],
        vec![
            fields(&[("file", "new/one.flac"), ("Id", "900"), ("Artist", "X")]),
            fields(&[("file", "new/two.flac"), ("Id", "901"), ("Artist", "X")]),
        ],
    );
    let (router, _state, mock) = app_with(initial).await;
    mock.set_fail_commands(vec!["searchadd".to_string()]).await;

    let (status, _, _) = call(
        &router,
        post_json(
            "/api/queue/add",
            &json!({
                "targets": [{"path": "a/one.flac"}, {"artist": "X"}],
                "play": true
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);

    let (status, _, body) = call(&router, get("/api/playlist")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    let songs = value.as_array().expect("array");
    assert_eq!(songs.len(), 2, "compensation must undo the partial add");
    assert_eq!(songs[0]["file"], "old/one.flac");
    assert_eq!(songs[1]["file"], "old/two.flac");
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

/// `position: after_current` must place the new song right after the playing
/// one, before the rest of the queue.
#[tokio::test]
async fn queue_add_after_current_inserts_after_playing_song() {
    let initial = wipe_state(
        vec![
            fields(&[("file", "a/one.flac"), ("Id", "10")]),
            fields(&[("file", "a/two.flac"), ("Id", "11")]),
            fields(&[("file", "a/three.flac"), ("Id", "12")]),
        ],
        vec![fields(&[
            ("file", "new/one.flac"),
            ("Id", "900"),
            ("Artist", "X"),
        ])],
    );
    let (router, _state, mock) = app_with(initial).await;
    // No pinned `playlistlength`: the mock mirrors the live playlist size,
    // as real MPD does.
    mock.set_status(fields(&[("state", "play"), ("song", "1")]))
        .await;

    let (status, _, _) = call(
        &router,
        post_json(
            "/api/queue/add",
            &json!({"targets": [{"artist": "X"}], "position": "after_current"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, body) = call(&router, get("/api/playlist")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    let songs = value.as_array().expect("array");
    let files: Vec<&str> = songs
        .iter()
        .map(|s| s["file"].as_str().expect("file"))
        .collect();
    assert_eq!(
        files,
        vec!["a/one.flac", "a/two.flac", "new/one.flac", "a/three.flac"],
        "the new song must land right after the playing one"
    );
}

/// Several appended songs move as one block, keeping their order, right
/// after the playing song.
#[tokio::test]
async fn queue_add_after_current_moves_block_of_songs() {
    let initial = wipe_state(
        vec![
            fields(&[("file", "a/one.flac"), ("Id", "10")]),
            fields(&[("file", "a/two.flac"), ("Id", "11")]),
            fields(&[("file", "a/three.flac"), ("Id", "12")]),
        ],
        vec![
            fields(&[("file", "new/one.flac"), ("Id", "900"), ("Artist", "X")]),
            fields(&[("file", "new/two.flac"), ("Id", "901"), ("Artist", "X")]),
        ],
    );
    let (router, _state, mock) = app_with(initial).await;
    mock.set_status(fields(&[("state", "play"), ("song", "1")]))
        .await;

    let (status, _, _) = call(
        &router,
        post_json(
            "/api/queue/add",
            &json!({"targets": [{"artist": "X"}], "position": "after_current"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, body) = call(&router, get("/api/playlist")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    let songs = value.as_array().expect("array");
    let files: Vec<&str> = songs
        .iter()
        .map(|s| s["file"].as_str().expect("file"))
        .collect();
    assert_eq!(
        files,
        vec![
            "a/one.flac",
            "a/two.flac",
            "new/one.flac",
            "new/two.flac",
            "a/three.flac"
        ],
        "all appended songs must land right after the playing one, in order"
    );
}

/// With nothing playing, `after_current` means the front of the queue.
#[tokio::test]
async fn queue_add_after_current_with_nothing_playing_goes_to_front() {
    let initial = wipe_state(
        vec![
            fields(&[("file", "a/one.flac"), ("Id", "10")]),
            fields(&[("file", "a/two.flac"), ("Id", "11")]),
        ],
        vec![fields(&[
            ("file", "new/one.flac"),
            ("Id", "900"),
            ("Artist", "X"),
        ])],
    );
    let (router, _state, mock) = app_with(initial).await;
    mock.set_status(fields(&[("state", "stop")])).await;

    let (status, _, _) = call(
        &router,
        post_json(
            "/api/queue/add",
            &json!({"targets": [{"artist": "X"}], "position": "after_current"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, body) = call(&router, get("/api/playlist")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    let songs = value.as_array().expect("array");
    assert_eq!(songs.len(), 3);
    assert_eq!(
        songs[0]["file"], "new/one.flac",
        "nothing playing: the new song must lead the queue"
    );
    assert_eq!(songs[1]["file"], "a/one.flac");
    assert_eq!(songs[2]["file"], "a/two.flac");
}

/// Without `position` the songs simply append to the end (the old behavior).
#[tokio::test]
async fn queue_add_default_appends_to_end() {
    let initial = wipe_state(
        vec![
            fields(&[("file", "a/one.flac"), ("Id", "10")]),
            fields(&[("file", "a/two.flac"), ("Id", "11")]),
        ],
        vec![fields(&[
            ("file", "new/one.flac"),
            ("Id", "900"),
            ("Artist", "X"),
        ])],
    );
    let (router, _state, mock) = app_with(initial).await;
    mock.set_status(fields(&[("state", "play"), ("song", "0")]))
        .await;

    let (status, _, _) = call(
        &router,
        post_json("/api/queue/add", &json!({"targets": [{"artist": "X"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, body) = call(&router, get("/api/playlist")).await;
    assert_eq!(status, StatusCode::OK);
    let value = as_json(&body);
    let songs = value.as_array().expect("array");
    let files: Vec<&str> = songs
        .iter()
        .map(|s| s["file"].as_str().expect("file"))
        .collect();
    assert_eq!(files, vec!["a/one.flac", "a/two.flac", "new/one.flac"]);
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
