//! End-to-end tests for `mpd-client` against an in-process [`mpd_mock::MockMpd`].
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use mpd_client::{MpdClient, MpdConfig, MpdEvent, PlayState};
use mpd_mock::{MockMpd, MockState};
use tokio::sync::broadcast;
use tokio::time::{Instant, timeout};

fn fields(pairs: &[(&str, &str)]) -> mpd_mock::FieldList {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn config_for(port: u16) -> MpdConfig {
    MpdConfig::new("127.0.0.1", port, None)
}

/// Wait (up to 5s) for the next event satisfying `pred`, skipping the rest.
async fn wait_for_event<F>(rx: &mut broadcast::Receiver<MpdEvent>, pred: F) -> Option<MpdEvent>
where
    F: Fn(&MpdEvent) -> bool,
{
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        match timeout(deadline - now, rx.recv()).await {
            Ok(Ok(ev)) if pred(&ev) => return Some(ev),
            Ok(Ok(_)) => continue,
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => return None,
            Err(_) => return None,
        }
    }
}

/// Capabilities are populated asynchronously at connect; poll until present.
async fn wait_for_caps(client: &MpdClient) -> mpd_client::Capabilities {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let caps = client.capabilities().await;
        if !caps.commands.is_empty() || Instant::now() >= deadline {
            return caps;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn status_and_currentsong_roundtrip() {
    let server = MockMpd::start(MockState {
        // Realistic MPD 0.23.x `status` response (see MPD source
        // src/command/PlayerCommands.cxx): `time` is `<elapsed>:<total>`,
        // playlist length is `playlistlength`, crossfade is `xfade`.
        status: fields(&[
            ("volume", "70"),
            ("repeat", "1"),
            ("random", "0"),
            ("single", "0"),
            ("consume", "0"),
            ("partition", "default"),
            ("playlist", "7"),
            ("playlistlength", "3"),
            ("mixrampdb", "0"),
            ("state", "play"),
            ("xfade", "3"),
            ("song", "0"),
            ("songid", "0"),
            ("time", "42:214"),
            ("elapsed", "42.5"),
            ("bitrate", "320"),
        ]),
        currentsong: Some(fields(&[
            ("file", "Album/01 - Song.flac"),
            ("Id", "0"),
            ("Title", "Song One"),
            ("Artist", "Band A"),
            ("Album", "Album"),
            ("AlbumArtist", "Band A"),
            ("Track", "01"),
            ("Time", "214"),
        ])),
        ..Default::default()
    })
    .await;

    let client = MpdClient::connect(config_for(server.port())).await;

    let st = client.status().await.expect("status");
    assert_eq!(st.state, PlayState::Play);
    assert_eq!(st.time, 214, "time must be the total after the colon");
    assert!((st.elapsed - 42.5).abs() < 0.01);
    assert_eq!(st.volume, 70);
    assert!(st.repeat);
    assert!(!st.random);
    assert_eq!(st.playlist_version, 7);
    assert_eq!(st.songs, 3, "playlistlength must map to songs");
    assert_eq!(st.crossfade, 3, "xfade must map to crossfade");

    let song = client.currentsong().await.expect("currentsong");
    let song = song.expect("a current song");
    assert_eq!(song.file, "Album/01 - Song.flac");
    assert_eq!(song.title.as_deref(), Some("Song One"));
    assert_eq!(song.artist.as_deref(), Some("Band A"));
    assert_eq!(song.albumartist.as_deref(), Some("Band A"));
    assert_eq!(song.id, Some(0));
    assert_eq!(song.time, Some(214));
    assert_eq!(song.display_artist(), "Band A");
    assert_eq!(song.display_title(), "Song One");
}

#[tokio::test]
async fn playlist_roundtrip() {
    let server = MockMpd::start(MockState {
        playlist: vec![
            fields(&[
                ("file", "a/one.flac"),
                ("Id", "0"),
                ("Title", "One"),
                ("Artist", "A"),
            ]),
            fields(&[
                ("file", "a/two.flac"),
                ("Id", "1"),
                ("Title", "Two"),
                ("Artist", "A"),
            ]),
            fields(&[
                ("file", "b/three.flac"),
                ("Id", "2"),
                ("Title", "Three"),
                ("Artist", "B"),
            ]),
        ],
        ..Default::default()
    })
    .await;

    let client = MpdClient::connect(config_for(server.port())).await;
    let songs = client.playlist().await.expect("playlist");
    assert_eq!(songs.len(), 3);
    assert_eq!(songs[0].file, "a/one.flac");
    assert_eq!(songs[0].id, Some(0));
    assert_eq!(songs[2].artist.as_deref(), Some("B"));
    assert_eq!(songs[2].display_artist(), "B");
}

#[tokio::test]
async fn lsinfo_browse_roundtrip() {
    let server = MockMpd::start(MockState {
        lsinfo: fields(&[
            ("directory", "/Music/Band"),
            ("songcount", "10"),
            ("playtime", "2400"),
            ("directory", "/Music/Other"),
            ("songcount", "5"),
            ("playtime", "1200"),
            ("file", "/Music/loose.flac"),
            ("Title", "Loose"),
            ("Artist", "X"),
            ("Time", "60"),
        ]),
        ..Default::default()
    })
    .await;

    let client = MpdClient::connect(config_for(server.port())).await;
    let b = client.lsinfo("/").await.expect("lsinfo");
    assert_eq!(b.directories.len(), 2);
    assert_eq!(b.directories[0].path, "/Music/Band");
    assert!(b.directories[0].is_dir);
    assert_eq!(b.directories[0].songcount, Some(10));
    assert_eq!(b.directories[0].playtime, Some(2400));
    assert_eq!(b.directories[1].path, "/Music/Other");
    assert_eq!(b.files.len(), 1);
    assert_eq!(b.files[0].path, "/Music/loose.flac");
    assert!(!b.files[0].is_dir);
    assert_eq!(
        b.files[0]
            .song
            .as_ref()
            .expect("file song")
            .title
            .as_deref(),
        Some("Loose")
    );
    assert_eq!(b.songcount, Some(16));
    assert_eq!(b.playtime, Some(3660));
}

#[tokio::test]
async fn lsinfo_browse_real_mpd_no_per_dir_counts() {
    // MPD 0.23.x `lsinfo` emits only `directory:` + `Last-Modified:` for
    // directories (no songcount/playtime). The listing totals must be None.
    let server = MockMpd::start(MockState {
        lsinfo: fields(&[
            ("directory", "Band"),
            ("Last-Modified", "1700000000"),
            ("directory", "Other"),
            ("Last-Modified", "1700000001"),
            ("file", "loose.flac"),
            ("Title", "Loose"),
            ("Artist", "X"),
            ("Time", "60"),
        ]),
        ..Default::default()
    })
    .await;

    let client = MpdClient::connect(config_for(server.port())).await;
    let b = client.lsinfo("/").await.expect("lsinfo");
    assert_eq!(b.directories.len(), 2);
    assert_eq!(b.directories[0].path, "Band");
    assert_eq!(b.directories[0].songcount, None);
    assert_eq!(b.directories[0].playtime, None);
    assert_eq!(b.files.len(), 1);
    assert_eq!(b.songcount, None);
    assert_eq!(b.playtime, None);
}

#[tokio::test]
async fn search_list_count_roundtrip() {
    let server = MockMpd::start(MockState {
        search: vec![
            fields(&[("file", "a/one.flac"), ("Id", "0"), ("Artist", "A")]),
            fields(&[("file", "a/two.flac"), ("Id", "1"), ("Artist", "A")]),
        ],
        // Real MPD `list artist` replies with the capitalized `Artist:` key.
        list: fields(&[("Artist", "A"), ("Artist", "B"), ("Artist", "C")]),
        ..Default::default()
    })
    .await;

    let client = MpdClient::connect(config_for(server.port())).await;

    let artists = client.list("artist", None, None).await.expect("list");
    assert_eq!(
        artists,
        vec!["A".to_string(), "B".to_string(), "C".to_string()]
    );

    let (songs, _playtime) = client.count(&[("Artist", "A")], "==").await.expect("count");
    assert_eq!(songs, 2); // the mock counts the matching search results
}

#[tokio::test]
async fn read_picture_returns_bytes_and_declared_mime() {
    // PNG magic bytes, but declare a different mime to prove declared wins.
    let png_magic = [0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 0];
    let server = MockMpd::start(MockState {
        art: Some((png_magic.to_vec(), "image/webp".to_string())),
        ..Default::default()
    })
    .await;

    let client = MpdClient::connect(config_for(server.port())).await;
    let (bytes, mime) = client
        .read_picture("Album/01 - Song.flac")
        .await
        .expect("art present");
    assert_eq!(bytes, png_magic);
    assert_eq!(mime, "image/webp");
}

#[tokio::test]
async fn read_picture_absent_returns_none() {
    let server = MockMpd::start(MockState::default()).await;
    let client = MpdClient::connect(config_for(server.port())).await;
    assert!(client.read_picture("missing.flac").await.is_none());
}

#[tokio::test]
async fn idle_player_change_emits_snapshot() {
    let server = MockMpd::start(MockState {
        status: fields(&[("state", "play"), ("volume", "55"), ("playlist", "1")]),
        currentsong: Some(fields(&[
            ("file", "Album/01.flac"),
            ("Id", "0"),
            ("Title", "S"),
        ])),
        ..Default::default()
    })
    .await;

    let client = MpdClient::connect(config_for(server.port())).await;
    let mut rx = client.events();

    // The command session primes exactly one snapshot at connect; drain it so
    // the next snapshot is unambiguously the idle-triggered one.
    let _ = wait_for_event(&mut rx, |e| matches!(e, MpdEvent::Reconnected)).await;
    let _ = wait_for_event(&mut rx, |e| matches!(e, MpdEvent::Snapshot(_))).await;

    server.set_next_idle_change("player").await;
    let ev = wait_for_event(&mut rx, |e| matches!(e, MpdEvent::Snapshot(_))).await;
    let MpdEvent::Snapshot(snap) = ev.expect("idle snapshot event") else {
        panic!("expected a snapshot");
    };
    assert_eq!(snap.status.volume, 55);
    assert!(snap.song.is_some());
}

#[tokio::test]
async fn idle_database_change_emits_event() {
    let server = MockMpd::start(MockState::default()).await;
    let client = MpdClient::connect(config_for(server.port())).await;
    let mut rx = client.events();

    // Give the idle connection a moment to attach and enter its loop.
    tokio::time::sleep(Duration::from_millis(150)).await;

    server.set_next_idle_change("database").await;
    let ev = wait_for_event(&mut rx, |e| matches!(e, MpdEvent::DatabaseChanged)).await;
    assert!(matches!(ev, Some(MpdEvent::DatabaseChanged)));
}

#[tokio::test]
async fn capabilities_from_commands() {
    let server = MockMpd::start(MockState {
        commands: vec!["play".into(), "status".into(), "seek".into()],
        not_commands: vec!["setvol".into()],
        ..Default::default()
    })
    .await;

    let client = MpdClient::connect(config_for(server.port())).await;
    let caps = wait_for_caps(&client).await;
    assert_eq!(caps.version, "0.23.0");
    assert!(caps.has("seek"));
    assert!(!caps.has("stop"));
    assert!(caps.not_commands.iter().any(|c| c == "setvol"));
}

#[tokio::test]
async fn password_authentication() {
    let server = MockMpd::start(MockState {
        password: Some("secret".into()),
        status: fields(&[("state", "play"), ("volume", "42")]),
        ..Default::default()
    })
    .await;

    let config = MpdConfig::new("127.0.0.1", server.port(), Some("secret".into()));
    let client = MpdClient::connect(config).await;
    let st = client.status().await.expect("status with auth");
    assert_eq!(st.volume, 42);
}

#[tokio::test]
async fn playback_commands_succeed() {
    let server = MockMpd::start(MockState::default()).await;
    let client = MpdClient::connect(config_for(server.port())).await;

    client.play(Some(0)).await.expect("play 0");
    client.play(None).await.expect("play");
    client.pause(Some(true)).await.expect("pause 1");
    client.pause(None).await.expect("pause");
    client.stop().await.expect("stop");
    client.next().await.expect("next");
    client.previous().await.expect("previous");
    client.seek(12.0).await.expect("seekcur");
    client.set_volume(80).await.expect("setvol");
    client
        .set_options(Some(true), Some(true), Some(false), Some(true))
        .await
        .expect("options");
}

#[tokio::test]
async fn command_keepalive_survives_server_connection_timeout() {
    // Emulates MPD's `connection_timeout` (default 60s): the server closes
    // connections that send no data. The command connection only transmits
    // on `noop` keepalives, so a keepalive shorter than the server window
    // must keep it alive across several windows.
    let server = MockMpd::start(MockState {
        connection_timeout: Some(Duration::from_millis(200)),
        status: fields(&[("state", "play"), ("volume", "42")]),
        ..Default::default()
    })
    .await;

    let mut config = config_for(server.port());
    config.keepalive = Duration::from_millis(50);
    let client = MpdClient::connect(config).await;

    // Four full timeout windows of client silence (broken only by noops).
    tokio::time::sleep(Duration::from_millis(900)).await;
    let st = client.status().await.expect("status after quiet period");
    assert_eq!(st.volume, 42);
}

#[tokio::test]
async fn command_connection_without_keepalive_is_reaped_and_recovers() {
    // Default keepalive (30s) >> server window: the session dies on the
    // next command, then reconnects transparently.
    let server = MockMpd::start(MockState {
        connection_timeout: Some(Duration::from_millis(100)),
        status: fields(&[("state", "play"), ("volume", "42")]),
        ..Default::default()
    })
    .await;

    let client = MpdClient::connect(config_for(server.port())).await;
    let mut rx = client.events();

    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        client.status().await.is_err(),
        "command on the reaped connection must fail"
    );
    let _ = wait_for_event(&mut rx, |e| matches!(e, MpdEvent::Disconnected)).await;

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if client.status().await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        client
            .status()
            .await
            .expect("reconnected session serves status")
            .volume
            == 42
    );
}

#[tokio::test]
async fn queue_commands_succeed() {
    let server = MockMpd::start(MockState {
        search: vec![
            fields(&[("file", "s/a.flac"), ("Id", "900"), ("Artist", "AC/DC")]),
            fields(&[("file", "s/b.flac"), ("Id", "901"), ("Artist", "AC/DC")]),
        ],
        ..Default::default()
    })
    .await;
    let client = MpdClient::connect(config_for(server.port())).await;

    assert_eq!(client.playlist_count().await.expect("empty count"), 0);

    client.add("Album/01 - Song.flac").await.expect("add");
    assert_eq!(client.playlist_count().await.expect("count after add"), 1);

    client
        .searchadd(&[("Artist", "AC/DC")], "==")
        .await
        .expect("searchadd");
    assert_eq!(
        client
            .playlist_count()
            .await
            .expect("count after searchadd"),
        3
    );

    // Wipe the first two entries (the added file plus one search hit).
    client.delete_range(0, 1).await.expect("delete range");
    assert_eq!(
        client
            .playlist_count()
            .await
            .expect("count after delete range"),
        1
    );

    client.clear().await.expect("clear");
    assert_eq!(client.playlist_count().await.expect("count after clear"), 0);

    client.add("Album/01 - Song.flac").await.expect("add again");
    client
        .add("Album/02 - Song.flac")
        .await
        .expect("add again 2");
    client.move_id(1, 0).await.expect("moveid");
    // Positions shift after each delete, so remove from the back.
    client.delete_ids(&[1, 0]).await.expect("deleteid list");
    assert_eq!(
        client.playlist_count().await.expect("count after deleteid"),
        0
    );
    client.shuffle().await.expect("shuffle");
}

#[tokio::test]
async fn play_id_selects_track() {
    let server = MockMpd::start(MockState::default()).await;
    let client = MpdClient::connect(config_for(server.port())).await;

    client.add("Album/01 - One.flac").await.expect("add one");
    client.add("Album/01 - Two.flac").await.expect("add two");
    let playlist = client.playlist().await.expect("playlist");
    let id = playlist[1].id.expect("second entry carries an id");

    client.play_id(id).await.expect("playid");
    let song = client
        .currentsong()
        .await
        .expect("currentsong")
        .expect("a song is current");
    assert_eq!(song.file, "Album/01 - Two.flac");
}

#[tokio::test]
async fn read_picture_survives_chunked_delivery() {
    // Every reply (greeting, status, the `binary: N` header, and the payload
    // itself) is fragmented into 7-byte chunks: the client must reassemble
    // the text lines and the exact binary length across segment boundaries.
    let mut art: Vec<u8> = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10];
    art.extend(0u8..=55);
    let server = MockMpd::start(MockState {
        art: Some((art.clone(), "image/png".to_string())),
        write_chunk_size: 7,
        status: fields(&[("state", "play"), ("volume", "42")]),
        ..Default::default()
    })
    .await;

    let client = MpdClient::connect(config_for(server.port())).await;
    assert_eq!(client.status().await.expect("status").volume, 42);

    let (bytes, mime) = client
        .read_picture("Album/01 - Song.flac")
        .await
        .expect("art present");
    assert_eq!(bytes, art);
    assert_eq!(mime, "image/png");
}

#[tokio::test]
async fn idle_survives_unexpected_line() {
    // A line that is neither `changed:` nor `OK` appears in the idle reply.
    // The client must ignore it, still deliver the batch's change, and keep
    // idling afterwards.
    let server = MockMpd::start(MockState {
        status: fields(&[("state", "play"), ("volume", "55"), ("playlist", "1")]),
        currentsong: Some(fields(&[
            ("file", "Album/01.flac"),
            ("Id", "0"),
            ("Title", "S"),
        ])),
        ..Default::default()
    })
    .await;

    let client = MpdClient::connect(config_for(server.port())).await;
    let mut rx = client.events();

    let _ = wait_for_event(&mut rx, |e| matches!(e, MpdEvent::Reconnected)).await;
    let _ = wait_for_event(&mut rx, |e| matches!(e, MpdEvent::Snapshot(_))).await;

    server
        .set_next_idle_stray("stray: unsolicited server output")
        .await;
    server.set_next_idle_change("player").await;
    let ev = wait_for_event(&mut rx, |e| matches!(e, MpdEvent::Snapshot(_))).await;
    let MpdEvent::Snapshot(snap) = ev.expect("snapshot after stray line") else {
        panic!("expected a snapshot");
    };
    assert_eq!(snap.status.volume, 55);

    // The idle connection must still be alive and delivering changes.
    server.set_next_idle_change("database").await;
    let ev = wait_for_event(&mut rx, |e| matches!(e, MpdEvent::DatabaseChanged)).await;
    assert!(matches!(ev, Some(MpdEvent::DatabaseChanged)));
}

#[tokio::test]
async fn auth_failure_recovers_when_password_restored() {
    // The operator changes the MPD password to one the client does not know.
    // The session is reaped (keepalive 30s >> server window 300ms) and every
    // reconnect fails auth; commands must fail. Once the password is restored
    // to what the client sends, the session must recover transparently.
    let server = MockMpd::start(MockState {
        password: Some("secret".into()),
        connection_timeout: Some(Duration::from_millis(300)),
        status: fields(&[("state", "play"), ("volume", "42")]),
        ..Default::default()
    })
    .await;

    let mut config = MpdConfig::new("127.0.0.1", server.port(), Some("secret".into()));
    config.command_timeout = Duration::from_millis(250);
    let client = MpdClient::connect(config).await;
    assert_eq!(client.status().await.expect("initial status").volume, 42);

    server.set_password(Some("changed".into())).await;

    // After several reaped/rejected windows, commands must fail promptly.
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(
        client.status().await.is_err(),
        "commands must fail while the server rejects the password"
    );

    server.set_password(Some("secret".into())).await;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut recovered = false;
    while Instant::now() < deadline {
        if let Ok(st) = client.status().await {
            if st.volume == 42 {
                recovered = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        recovered,
        "session must recover once the password is restored"
    );
}

#[tokio::test]
async fn command_times_out_when_server_never_replies() {
    let server = MockMpd::start(MockState {
        stall_commands: vec!["status".to_string()],
        ..Default::default()
    })
    .await;
    let mut config = config_for(server.port());
    config.command_timeout = Duration::from_millis(150);
    let client = MpdClient::connect(config).await;

    let err = client.status().await.expect_err("must time out");
    assert!(err.to_string().contains("timed out"), "{err}");
}
