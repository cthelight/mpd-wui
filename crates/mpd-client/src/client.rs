//! Async MPD client: a serialized command connection, an idle connection that
//! pushes events, and ephemeral connections for binary album-art reads.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc, oneshot, watch, RwLock};

use crate::protocol::{
    build_search_filters, parse_art, parse_currentsong, parse_list, parse_lsinfo, parse_song_list,
    parse_status, parse_text, quote_arg, sniff_mime, Response,
};
use crate::types::{Browse, Capabilities, MpdEvent, Snapshot, Song, Status};

/// Connection settings for an MPD server.
#[derive(Debug, Clone)]
pub struct MpdConfig {
    pub host: String,
    pub port: u16,
    pub password: Option<String>,
}

impl MpdConfig {
    pub fn new(host: impl Into<String>, port: u16, password: Option<String>) -> Self {
        Self {
            host: host.into(),
            port,
            password,
        }
    }
}

/// A single request routed through the serialized command connection.
struct Request {
    /// The exact protocol command text to write (always a single line).
    /// A trailing newline is appended by the sender.
    cmd: String,
    reply: oneshot::Sender<Result<String, String>>,
}

/// Handle to a running MPD client. Cheap to clone the pieces out of; the
/// background tasks live for the process lifetime.
#[derive(Clone)]
pub struct MpdClient {
    config: MpdConfig,
    cmd_tx: mpsc::Sender<Request>,
    snap_rx: watch::Receiver<Snapshot>,
    events: broadcast::Sender<MpdEvent>,
    caps: Arc<RwLock<Capabilities>>,
}

impl MpdClient {
    /// Connect and spawn the command + idle background tasks.
    pub async fn connect(config: MpdConfig) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(256);
        let (snap_tx, snap_rx) = watch::channel(Snapshot::default());
        let events = broadcast::channel(128).0;
        let caps = Arc::new(RwLock::new(Capabilities::default()));

        let c1 = config.clone();
        let e1 = events.clone();
        let s1 = snap_tx.clone();
        let caps1 = caps.clone();
        tokio::spawn(command_loop(c1, cmd_rx, e1, s1, caps1));

        let c2 = config.clone();
        let e2 = events.clone();
        let s2 = snap_tx.clone();
        tokio::spawn(idle_loop(c2, cmd_tx.clone(), e2, s2));

        Self {
            config,
            cmd_tx,
            snap_rx,
            events,
            caps,
        }
    }

    // -- low-level ----------------------------------------------------------

    async fn send_raw(&self, command: &str) -> anyhow::Result<String> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(Request {
                cmd: command.to_string(),
                reply: tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("mpd command channel closed"))?;
        rx.await
            .map_err(|_| anyhow::anyhow!("mpd command dropped"))?
            .map_err(|e| anyhow::anyhow!(e))
    }

    async fn cmd(&self, command: &str) -> anyhow::Result<Response> {
        let raw = self.send_raw(command).await?;
        let resp = parse_text(&raw);
        resp.check().map_err(|a| anyhow::anyhow!("{a}"))?;
        Ok(resp)
    }

    // -- queries ------------------------------------------------------------

    pub async fn status(&self) -> anyhow::Result<Status> {
        Ok(parse_status(&self.cmd("status").await?))
    }

    pub async fn currentsong(&self) -> anyhow::Result<Option<Song>> {
        Ok(parse_currentsong(&self.cmd("currentsong").await?))
    }

    pub async fn playlist(
        &self,
        start: Option<u32>,
        end: Option<u32>,
    ) -> anyhow::Result<Vec<Song>> {
        let cmd = match (start, end) {
            (Some(s), Some(e)) => format!("playlistinfo {s} {e}"),
            (Some(s), None) => format!("playlistinfo {s}"),
            _ => "playlistinfo".to_string(),
        };
        Ok(parse_song_list(&self.cmd(&cmd).await?))
    }

    pub async fn lsinfo(&self, path: &str) -> anyhow::Result<Browse> {
        let cmd = format!("lsinfo {}", quote_arg(path));
        Ok(parse_lsinfo(&self.cmd(&cmd).await?))
    }

    /// `list type [artist] [albumartist]` → values for `type`.
    pub async fn list(
        &self,
        tag: &str,
        artist: Option<&str>,
        albumartist: Option<&str>,
    ) -> anyhow::Result<Vec<String>> {
        let mut cmd = format!("list {tag}");
        if let Some(a) = artist.filter(|a| !a.is_empty()) {
            cmd.push_str(&format!(" {}", quote_arg(a)));
        }
        if let Some(aa) = albumartist.filter(|a| !a.is_empty()) {
            cmd.push_str(&format!(" {}", quote_arg(aa)));
        }
        let resp = self.cmd(&cmd).await?;
        Ok(parse_list(&resp, tag))
    }

    /// Filter-based `search` (MPD >= 0.21 filter syntax).
    pub async fn search(&self, pairs: &[(&str, &str)], op: &str) -> anyhow::Result<Vec<Song>> {
        let filters = build_search_filters(pairs, op);
        if filters.is_empty() {
            return Ok(Vec::new());
        }
        let cmd = format!("search {}", quote_arg(&filters));
        Ok(parse_song_list(&self.cmd(&cmd).await?))
    }

    /// Dump the entire music database — every song with its full metadata — in
    /// a single round-trip, for local (in-process) search.
    ///
    /// MPD's filter grammar cannot express "match any tag" (there is no `OR`),
    /// so broad search is done by pulling the whole library and filtering in
    /// this process. `(File contains '')` matches every song.
    pub async fn library(&self) -> anyhow::Result<Vec<Song>> {
        let filters = "(File contains '')";
        let cmd = format!("search {}", quote_arg(filters));
        Ok(parse_song_list(&self.cmd(&cmd).await?))
    }

    pub async fn count(&self, pairs: &[(&str, &str)], op: &str) -> anyhow::Result<(u32, u32)> {
        let filters = build_search_filters(pairs, op);
        if filters.is_empty() {
            return Ok((0, 0));
        }
        let cmd = format!("count {}", quote_arg(&filters));
        let resp = self.cmd(&cmd).await?;
        let songs = resp.get("songs").and_then(|v| v.parse().ok()).unwrap_or(0);
        let playtime = resp
            .get("playtime")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        Ok((songs, playtime))
    }

    // -- playback -----------------------------------------------------------

    pub async fn play(&self, pos: Option<u32>) -> anyhow::Result<()> {
        match pos {
            Some(p) => self.cmd(&format!("play {p}")).await,
            None => self.cmd("play").await,
        }
        .map(|_| ())
    }

    pub async fn play_id(&self, id: u32) -> anyhow::Result<()> {
        self.cmd(&format!("playid {id}")).await.map(|_| ())
    }

    pub async fn pause(&self, state: Option<bool>) -> anyhow::Result<()> {
        let cmd = match state {
            Some(true) => "pause 1",
            Some(false) => "pause 0",
            None => "pause",
        };
        self.cmd(cmd).await.map(|_| ())
    }

    pub async fn stop(&self) -> anyhow::Result<()> {
        self.cmd("stop").await.map(|_| ())
    }

    pub async fn next(&self) -> anyhow::Result<()> {
        self.cmd("next").await.map(|_| ())
    }

    pub async fn previous(&self) -> anyhow::Result<()> {
        self.cmd("previous").await.map(|_| ())
    }

    pub async fn seek(&self, time: f32) -> anyhow::Result<()> {
        self.cmd(&format!("seekcur {time}")).await.map(|_| ())
    }

    pub async fn set_volume(&self, vol: u32) -> anyhow::Result<()> {
        self.cmd(&format!("setvol {vol}")).await.map(|_| ())
    }

    pub async fn set_options(
        &self,
        random: Option<bool>,
        repeat: Option<bool>,
        single: Option<bool>,
        consume: Option<bool>,
    ) -> anyhow::Result<()> {
        let mut cmds = Vec::new();
        if let Some(v) = random {
            cmds.push(format!("random {}", u32::from(v)));
        }
        if let Some(v) = repeat {
            cmds.push(format!("repeat {}", u32::from(v)));
        }
        if let Some(v) = single {
            cmds.push(format!("single {}", u32::from(v)));
        }
        if let Some(v) = consume {
            cmds.push(format!("consume {}", u32::from(v)));
        }
        // Sent as individual commands: `command_list_*` was removed in
        // MPD 0.22, and a desynchronized reply stream corrupts the
        // cached snapshot (e.g. "Nothing playing" while a song plays).
        if let Some(v) = random {
            self.cmd(&format!("random {}", u32::from(v))).await?;
        }
        if let Some(v) = repeat {
            self.cmd(&format!("repeat {}", u32::from(v))).await?;
        }
        if let Some(v) = single {
            self.cmd(&format!("single {}", u32::from(v))).await?;
        }
        if let Some(v) = consume {
            self.cmd(&format!("consume {}", u32::from(v))).await?;
        }
        Ok(())
    }

    // -- queue --------------------------------------------------------------

    pub async fn add(&self, file: &str) -> anyhow::Result<()> {
        self.cmd(&format!("add {}", quote_arg(file)))
            .await
            .map(|_| ())
    }

    pub async fn searchadd(&self, pairs: &[(&str, &str)], op: &str) -> anyhow::Result<()> {
        let filters = build_search_filters(pairs, op);
        if filters.is_empty() {
            return Ok(());
        }
        self.cmd(&format!("searchadd {}", quote_arg(&filters)))
            .await
            .map(|_| ())
    }

    pub async fn findadd(&self, tag: &str, value: &str) -> anyhow::Result<()> {
        self.cmd(&format!("findadd {tag} {}", quote_arg(value)))
            .await
            .map(|_| ())
    }

    pub async fn clear(&self) -> anyhow::Result<()> {
        self.cmd("clear").await.map(|_| ())
    }

    pub async fn delete_ids(&self, ids: &[u32]) -> anyhow::Result<()> {
        // Individual `deleteid` commands: `command_list_*` was removed in
        // MPD 0.22 (see `set_options`).
        for id in ids {
            self.cmd(&format!("deleteid {id}")).await?;
        }
        Ok(())
    }

    pub async fn move_id(&self, id: u32, to: u32) -> anyhow::Result<()> {
        self.cmd(&format!("moveid {id} {to}")).await.map(|_| ())
    }

    pub async fn shuffle(&self) -> anyhow::Result<()> {
        self.cmd("shuffle").await.map(|_| ())
    }

    // -- album art (ephemeral connection) -----------------------------------

    /// Fetch album art for a song: embedded picture (`readpicture`) first, then
    /// folder cover (`albumart`). Returns `(bytes, mime)` or None if absent.
    pub async fn read_picture(&self, uri: &str) -> Option<(Vec<u8>, String)> {
        if let Some(a) = self.read_art("readpicture", uri).await {
            return Some(a);
        }
        self.read_art("albumart", uri).await
    }

    async fn read_art(&self, kind: &str, uri: &str) -> Option<(Vec<u8>, String)> {
        let stream = TcpStream::connect((self.config.host.as_str(), self.config.port))
            .await
            .ok()?;
        let _ = stream.set_nodelay(true);
        let (r, mut w) = stream.into_split();
        let mut reader = BufReader::new(r);

        let mut greeting = String::new();
        reader.read_line(&mut greeting).await.ok()?;
        if let Some(pw) = &self.config.password {
            let line = format!("password {}\n", quote_arg(pw));
            if w.write_all(line.as_bytes()).await.is_err() {
                return None;
            }
            if w.flush().await.is_err() {
                return None;
            }
            let mut resp = String::new();
            reader.read_line(&mut resp).await.ok()?;
            if !resp.trim().starts_with("OK") {
                return None;
            }
        }

        let mut data: Vec<u8> = Vec::new();
        let mut offset: u32 = 0;
        let mut declared: Option<String> = None;
        loop {
            let cmd = format!("{kind} {} {offset}", quote_arg(uri));
            if w.write_all(cmd.as_bytes()).await.is_err() {
                return None;
            }
            if w.write_all(b"\n").await.is_err() {
                return None;
            }
            if w.flush().await.is_err() {
                return None;
            }
            let raw = match read_response(&mut reader).await {
                Ok(b) => b,
                Err(_) => return None,
            };
            let art = match parse_art(&raw) {
                Some(a) => a,
                None => return None,
            };
            if art.data.is_empty() {
                return None;
            }
            if declared.is_none() {
                declared = art.mime;
            }
            data.extend_from_slice(&art.data);
            offset += art.data.len() as u32;
            if art.total == 0 || data.len() as u32 >= art.total {
                break;
            }
        }
        let mime = declared.unwrap_or_else(|| art_mime(&data));
        Some((data, mime))
    }

    // -- observers ----------------------------------------------------------

    pub fn snapshot(&self) -> Snapshot {
        self.snap_rx.borrow().clone()
    }

    /// A fresh snapshot fetched over the command connection, as opposed to
    /// `snapshot()` which returns the last cached value (stale between
    /// `changed:` events, e.g. `elapsed` during continuous playback).
    pub async fn fresh_snapshot(&self) -> anyhow::Result<Snapshot> {
        let status = self.status().await?;
        let song = self.currentsong().await?;
        Ok(Snapshot { status, song })
    }

    /// Fetch a fresh snapshot and broadcast it to event subscribers.
    ///
    /// MPD only delivers `changed:` idle notifications to an idle connection
    /// that is awaiting an `idle` response at the moment the change happens.
    /// The idle connection is not idle while it fetches the snapshot for a
    /// previous change, so a change made in that window is never announced
    /// (and the earlier, stale snapshot may even overwrite the UI afterwards).
    /// Mutating commands therefore push their own post-command snapshot so
    /// subscribers (the WebSocket) always observe the new state.
    pub async fn refresh(&self) -> anyhow::Result<()> {
        let snapshot = self.fresh_snapshot().await?;
        let _ = self.events.send(MpdEvent::Snapshot(Box::new(snapshot)));
        Ok(())
    }

    pub fn events(&self) -> broadcast::Receiver<MpdEvent> {
        self.events.subscribe()
    }

    pub fn config(&self) -> &MpdConfig {
        &self.config
    }

    pub async fn capabilities(&self) -> Capabilities {
        let caps = self.caps.read().await.clone();
        if !caps.commands.is_empty() {
            return caps;
        }
        // Force the serialized command connection to complete its startup
        // handshake (which fetches `commands`) before reading again.
        let _ = self.cmd("noop").await;
        self.caps.read().await.clone()
    }
}

fn art_mime(data: &[u8]) -> String {
    sniff_mime(data)
}

// ---------------------------------------------------------------------------
// Command connection
// ---------------------------------------------------------------------------

enum SessionOutcome {
    Closed,
    Lost(String),
}

async fn command_loop(
    config: MpdConfig,
    mut rx: mpsc::Receiver<Request>,
    events: broadcast::Sender<MpdEvent>,
    snap: watch::Sender<Snapshot>,
    caps: Arc<RwLock<Capabilities>>,
) {
    let mut backoff = Duration::from_millis(250);
    loop {
        match run_command_session(&config, &mut rx, &events, &snap, &caps).await {
            SessionOutcome::Closed => return,
            SessionOutcome::Lost(e) => {
                tracing::warn!(error = %e, "mpd command connection lost; reconnecting");
                let _ = events.send(MpdEvent::Disconnected);
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(10));
            }
        }
    }
}

async fn run_command_session(
    config: &MpdConfig,
    rx: &mut mpsc::Receiver<Request>,
    events: &broadcast::Sender<MpdEvent>,
    snap: &watch::Sender<Snapshot>,
    caps: &Arc<RwLock<Capabilities>>,
) -> SessionOutcome {
    let stream = match TcpStream::connect((config.host.as_str(), config.port)).await {
        Ok(s) => s,
        Err(e) => return SessionOutcome::Lost(e.to_string()),
    };
    let _ = stream.set_nodelay(true);
    let (r, w) = stream.into_split();
    let mut reader = BufReader::new(r);
    let mut writer = w;

    let mut greeting = String::new();
    if reader.read_line(&mut greeting).await.is_err() {
        return SessionOutcome::Lost("no greeting".into());
    }
    let version = greeting.split_whitespace().last().unwrap_or("").to_string();

    if let Some(pw) = &config.password {
        let line = format!("password {}\n", quote_arg(pw));
        if writer.write_all(line.as_bytes()).await.is_err() {
            return SessionOutcome::Lost("write password".into());
        }
        if writer.flush().await.is_err() {
            return SessionOutcome::Lost("flush".into());
        }
        let mut resp = String::new();
        if reader.read_line(&mut resp).await.is_err() {
            return SessionOutcome::Lost("read auth response".into());
        }
        if !resp.trim().starts_with("OK") {
            return SessionOutcome::Lost(format!("auth failed: {}", resp.trim()));
        }
    }

    {
        caps.write().await.version = version;
        if let Ok(raw) = send_one(&mut reader, &mut writer, "commands").await {
            let r = parse_text(&raw);
            caps.write().await.commands =
                r.get_all("command").into_iter().map(String::from).collect();
        }
        if let Ok(raw) = send_one(&mut reader, &mut writer, "notcommands").await {
            let r = parse_text(&raw);
            caps.write().await.not_commands =
                r.get_all("command").into_iter().map(String::from).collect();
        }
    }

    let _ = events.send(MpdEvent::Reconnected);
    // Prime the snapshot now that we're connected, using this same connection.
    let status_raw = send_one(&mut reader, &mut writer, "status").await.ok();
    let song_raw = send_one(&mut reader, &mut writer, "currentsong").await.ok();
    if let (Some(sr), Some(cr)) = (status_raw, song_raw) {
        let status = parse_status(&parse_text(&sr));
        let song = parse_currentsong(&parse_text(&cr));
        let s = Snapshot { status, song };
        let _ = snap.send(s.clone());
        let _ = events.send(MpdEvent::Snapshot(Box::new(s)));
    }

    while let Some(req) = rx.recv().await {
        match send_one(&mut reader, &mut writer, &req.cmd).await {
            Ok(text) => {
                let _ = req.reply.send(Ok(text));
            }
            Err(e) => {
                let _ = req.reply.send(Err(e.clone()));
                return SessionOutcome::Lost(e);
            }
        }
    }
    SessionOutcome::Closed
}

/// Fetch a fresh status+song snapshot over a fresh command channel (used by the
/// idle loop and at (re)connect). `snap` is only used to keep the borrow simple.
async fn fetch_snapshot(cmd_tx: &mpsc::Sender<Request>) -> Option<Snapshot> {
    let status_raw = send_cmd(cmd_tx, "status").await.ok()?;
    let status = parse_status(&parse_text(&status_raw));
    let song_raw = send_cmd(cmd_tx, "currentsong").await.ok()?;
    let song = parse_currentsong(&parse_text(&song_raw));
    Some(Snapshot { status, song })
}

async fn send_cmd(cmd_tx: &mpsc::Sender<Request>, cmd: &str) -> Result<String, String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(Request {
            cmd: cmd.to_string(),
            reply: tx,
        })
        .await
        .map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())?
}

async fn send_one<R, W>(reader: &mut R, writer: &mut W, cmd: &str) -> Result<String, String>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    writer
        .write_all(cmd.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    writer.write_all(b"\n").await.map_err(|e| e.to_string())?;
    writer.flush().await.map_err(|e| e.to_string())?;
    let raw = read_response(reader).await.map_err(|e| e.to_string())?;
    String::from_utf8(raw).map_err(|e| e.to_string())
}

/// Read a full response (text, or text + inline binary) up to and including
/// `OK`/`ACK`. Returns the raw bytes.
async fn read_response<R: AsyncBufReadExt + Unpin>(reader: &mut R) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed",
            ));
        }
        buf.extend_from_slice(line.as_bytes());
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "OK" || trimmed.starts_with("ACK") {
            return Ok(buf);
        }
        if let Some(rest) = trimmed.strip_prefix("binary:") {
            let n: usize = rest.trim().parse().unwrap_or(0);
            let mut payload = vec![0u8; n];
            reader.read_exact(&mut payload).await?;
            buf.extend_from_slice(&payload);
            let mut nl = [0u8; 1];
            reader.read_exact(&mut nl).await?;
            buf.push(b'\n');
        }
    }
}

// ---------------------------------------------------------------------------
// Idle connection
// ---------------------------------------------------------------------------

async fn idle_loop(
    config: MpdConfig,
    cmd_tx: mpsc::Sender<Request>,
    events: broadcast::Sender<MpdEvent>,
    snap: watch::Sender<Snapshot>,
) {
    let mut backoff = Duration::from_millis(250);
    loop {
        let was_connected = idle_session(&config, &cmd_tx, &events, &snap).await;
        if !was_connected {
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(10));
            continue;
        }
        backoff = Duration::from_millis(250);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Run one idle session: connect, handshake, then keep issuing `idle` until the
/// connection dies. Returns true if a connection was established.
async fn idle_session(
    config: &MpdConfig,
    cmd_tx: &mpsc::Sender<Request>,
    events: &broadcast::Sender<MpdEvent>,
    snap: &watch::Sender<Snapshot>,
) -> bool {
    let stream = match TcpStream::connect((config.host.as_str(), config.port)).await {
        Ok(s) => s,
        Err(_) => return false,
    };
    let _ = stream.set_nodelay(true);
    let (r, mut w) = stream.into_split();
    let mut reader = BufReader::new(r);

    let mut greeting = String::new();
    if reader.read_line(&mut greeting).await.is_err() || !greeting.trim().starts_with("OK") {
        return false;
    }
    if let Some(pw) = &config.password {
        let line = format!("password {}\n", quote_arg(pw));
        if w.write_all(line.as_bytes()).await.is_err() || w.flush().await.is_err() {
            return false;
        }
        let mut resp = String::new();
        if reader.read_line(&mut resp).await.is_err() || !resp.trim().starts_with("OK") {
            return false;
        }
    }

    // Connected; keep issuing idle until the connection dies.
    loop {
        let idle_cmd = "idle player mixer options playlist database\n";
        if w.write_all(idle_cmd.as_bytes()).await.is_err() || w.flush().await.is_err() {
            return true;
        }
        let mut line = String::new();
        loop {
            match reader.read_line(&mut line).await {
                Ok(0) => return true,
                Ok(_) => {
                    let t = line.trim();
                    if t == "OK" {
                        break; // batch done; re-issue idle
                    }
                    if let Some(kind) = t.strip_prefix("changed: ") {
                        handle_change(kind, cmd_tx, events, snap).await;
                    }
                }
                Err(_) => return true,
            }
        }
    }
}

async fn handle_change(
    kind: &str,
    cmd_tx: &mpsc::Sender<Request>,
    events: &broadcast::Sender<MpdEvent>,
    snap: &watch::Sender<Snapshot>,
) {
    match kind.trim() {
        "database" => {
            let _ = events.send(MpdEvent::DatabaseChanged);
        }
        "player" | "playlist" | "options" | "mixer" => {
            if let Some(s) = fetch_snapshot(cmd_tx).await {
                let _ = snap.send(s.clone());
                let _ = events.send(MpdEvent::Snapshot(Box::new(s)));
            }
        }
        _ => {}
    }
}
