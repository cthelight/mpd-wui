//! mpd-mock: an in-process MPD server that speaks just enough of the protocol
//! to exercise `mpd-client` and `mpd-api` in tests. Dev-only, never shipped.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

/// A single `key: value` field.
pub type Field = (String, String);
/// An ordered list of fields (one item group, or a flat response).
pub type FieldList = Vec<Field>;

/// Configurable server state shared across all mock connections.
#[derive(Debug, Clone, Default)]
pub struct MockState {
    /// Fields for the `status` command.
    pub status: FieldList,
    /// Fields for the current song (`currentsong`), or None for an empty playlist.
    pub currentsong: Option<FieldList>,
    /// Item groups for `playlistinfo`.
    pub playlist: Vec<FieldList>,
    /// Flat fields for `lsinfo` (directory/file groups).
    pub lsinfo: FieldList,
    /// Flat fields for `list` (e.g. repeated `artist: X` lines).
    pub list: FieldList,
    /// Item groups for `search`.
    pub search: Vec<FieldList>,
    /// Album art `(bytes, mime)` returned by `readpicture`/`albumart`, if any.
    pub art: Option<(Vec<u8>, String)>,
    /// The `changed: <kind>` to emit on the *next* `idle` call (empty = no change).
    pub next_idle_change: String,
    /// Commands reported by `commands`.
    pub commands: Vec<String>,
    /// Commands reported by `notcommands`.
    pub not_commands: Vec<String>,
    /// Expected password, if authentication is required.
    pub password: Option<String>,
    /// Emulate MPD's `connection_timeout`: close connections that send no
    /// data within this window. None disables the behavior.
    pub connection_timeout: Option<std::time::Duration>,
    /// Next song `Id` to assign to items added to the playlist.
    pub next_id: u32,
    /// Commands to answer with an `ACK` failure, to exercise error paths.
    pub fail_commands: Vec<String>,
    /// Commands to never answer (no reply at all), to exercise client
    /// command timeouts.
    pub stall_commands: Vec<String>,
    /// Commands whose reply is delayed by a number of milliseconds, to make
    /// slow round-trips (e.g. stale-while-revalidate) deterministic.
    pub command_delays: std::collections::HashMap<String, std::time::Duration>,
    /// When non-zero, every reply is written in chunks of at most this many
    /// bytes with a short pause between chunks, emulating fragmented TCP
    /// delivery (mid-line and mid-binary splits).
    pub write_chunk_size: usize,
    /// An unexpected line (neither `changed:` nor `OK`) to emit on the *next*
    /// `idle` reply, to verify the client tolerates stray server output.
    pub next_idle_stray: String,
}

/// A running mock MPD server bound to an ephemeral localhost port.
pub struct MockMpd {
    addr: SocketAddr,
    state: Arc<Mutex<MockState>>,
    handle: tokio::task::JoinHandle<()>,
}

impl MockMpd {
    /// Start listening on 127.0.0.1 with an OS-chosen port.
    pub async fn start(state: MockState) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock mpd");
        let addr = listener
            .local_addr()
            .expect("bound listener has an address");
        let state = Arc::new(Mutex::new(state));
        let accept_state = state.clone();
        let handle = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let s = accept_state.clone();
                tokio::spawn(handle_conn(stream, s));
            }
        });
        Self {
            addr,
            state,
            handle,
        }
    }

    /// The `(host, port)` to point a client at.
    pub fn host_port(&self) -> (String, u16) {
        (self.addr.ip().to_string(), self.addr.port())
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    // -- state mutators (await to lock the shared state) --------------------

    pub async fn set_status(&self, fields: FieldList) {
        self.state.lock().await.status = fields;
    }

    pub async fn set_currentsong(&self, fields: Option<FieldList>) {
        self.state.lock().await.currentsong = fields;
    }

    pub async fn set_playlist(&self, items: Vec<FieldList>) {
        let mut st = self.state.lock().await;
        st.playlist = items;
        st.next_id = st
            .playlist
            .iter()
            .flatten()
            .find(|(k, _)| k == "Id")
            .and_then(|(_, v)| v.parse::<u32>().ok())
            .map(|id| id + 1)
            .unwrap_or(0);
    }

    pub async fn set_lsinfo(&self, fields: FieldList) {
        self.state.lock().await.lsinfo = fields;
    }

    pub async fn set_list(&self, fields: FieldList) {
        self.state.lock().await.list = fields;
    }

    pub async fn set_search(&self, items: Vec<FieldList>) {
        self.state.lock().await.search = items;
    }

    pub async fn set_art(&self, bytes: Vec<u8>, mime: &str) {
        self.state.lock().await.art = Some((bytes, mime.to_string()));
    }

    /// Queue a `changed: <kind>` to be emitted on the next `idle`.
    pub async fn set_next_idle_change(&self, kind: &str) {
        self.state.lock().await.next_idle_change = kind.to_string();
    }

    pub async fn set_commands(&self, cmds: Vec<String>) {
        self.state.lock().await.commands = cmds;
    }

    pub async fn set_password(&self, pw: Option<String>) {
        self.state.lock().await.password = pw;
    }

    pub async fn set_fail_commands(&self, cmds: Vec<String>) {
        self.state.lock().await.fail_commands = cmds;
    }

    pub async fn set_stall_commands(&self, cmds: Vec<String>) {
        self.state.lock().await.stall_commands = cmds;
    }

    /// Delay replies to `command` (e.g. `"search"`) by `ms` milliseconds.
    pub async fn set_command_delay(&self, command: &str, ms: u64) {
        self.state
            .lock()
            .await
            .command_delays
            .insert(command.to_string(), std::time::Duration::from_millis(ms));
    }

    /// Queue an unexpected line to be emitted alongside the next `idle` reply.
    pub async fn set_next_idle_stray(&self, line: &str) {
        self.state.lock().await.next_idle_stray = line.to_string();
    }
}

impl Drop for MockMpd {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

// ---------------------------------------------------------------------------
// Connection handling
// ---------------------------------------------------------------------------

async fn handle_conn(stream: TcpStream, state: Arc<Mutex<MockState>>) {
    let _ = stream.set_nodelay(true);
    let (r, mut w) = stream.into_split();
    let mut reader = BufReader::new(r);
    let st = state.lock().await;
    let idle_timeout = st.connection_timeout;
    let chunk = st.write_chunk_size;
    drop(st);

    let _ = write_all_chunked(&mut w, b"OK MPD 0.23.0\n", chunk).await;
    let _ = w.flush().await;

    let mut line = String::new();
    loop {
        line.clear();
        let read = match idle_timeout {
            Some(d) => tokio::time::timeout(d, reader.read_line(&mut line)).await,
            None => Ok(reader.read_line(&mut line).await),
        };
        let n = match read {
            Ok(Ok(n)) => n,
            // Data arrived too late (idle timeout) or the stream broke:
            // close, like MPD's `connection_timeout`.
            Ok(Err(_)) | Err(_) => break,
        };
        if n == 0 {
            break;
        }
        let cmd = line.trim().to_string();
        if cmd.is_empty() {
            continue;
        }
        let chunk = state.lock().await.write_chunk_size;
        if handle_cmd(&cmd, &mut w, &state, chunk).await.is_err() {
            return;
        }
    }
}

/// Write `bytes`, optionally split into chunks of at most `chunk` bytes with
/// a short pause between chunks (0 disables chunking). The pauses force the
/// client to reassemble responses across TCP segment boundaries.
async fn write_all_chunked(
    w: &mut tokio::net::tcp::OwnedWriteHalf,
    bytes: &[u8],
    chunk: usize,
) -> std::io::Result<()> {
    if chunk == 0 {
        w.write_all(bytes).await?;
        return Ok(());
    }
    for piece in bytes.chunks(chunk) {
        w.write_all(piece).await?;
        w.flush().await?;
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    Ok(())
}

async fn handle_cmd(
    cmd: &str,
    w: &mut tokio::net::tcp::OwnedWriteHalf,
    state: &Arc<Mutex<MockState>>,
    chunk: usize,
) -> std::io::Result<()> {
    let (name, rest) = match cmd.split_once(' ') {
        Some((n, r)) => (n, r.trim()),
        None => (cmd, ""),
    };

    let delay = {
        let st = state.lock().await;
        if st.fail_commands.iter().any(|c| c == name) {
            write_all_chunked(w, b"ACK [5@1] simulated failure\n", chunk).await?;
            let _ = w.flush().await;
            return Ok(());
        }
        if st.stall_commands.iter().any(|c| c == name) {
            return Ok(());
        }
        st.command_delays.get(name).copied()
    };
    // Sleep outside the state lock so other connections stay responsive.
    if let Some(d) = delay {
        tokio::time::sleep(d).await;
    }

    match name {
        "password" => {
            let st = state.lock().await;
            if st.password.as_deref() == Some(unquote(rest)) {
                write_all_chunked(w, b"OK\n", chunk).await?;
            } else {
                write_all_chunked(w, b"ACK [50@1] password not set\n", chunk).await?;
            }
        }
        "status" => {
            let st = state.lock().await;
            write_fields(w, &st.status, chunk).await?;
            write_all_chunked(w, b"OK\n", chunk).await?;
        }
        "currentsong" => {
            let st = state.lock().await;
            if let Some(song) = &st.currentsong {
                write_fields(w, song, chunk).await?;
            }
            write_all_chunked(w, b"OK\n", chunk).await?;
        }
        "playlistinfo" => {
            let st = state.lock().await;
            for item in &st.playlist {
                write_fields(w, item, chunk).await?;
            }
            write_all_chunked(w, b"OK\n", chunk).await?;
        }
        "lsinfo" => {
            let st = state.lock().await;
            write_fields(w, &st.lsinfo, chunk).await?;
            write_all_chunked(w, b"OK\n", chunk).await?;
        }
        "list" => {
            let st = state.lock().await;
            write_fields(w, &st.list, chunk).await?;
            write_all_chunked(w, b"OK\n", chunk).await?;
        }
        "search" => {
            let st = state.lock().await;
            for item in &st.search {
                write_fields(w, item, chunk).await?;
            }
            write_all_chunked(w, b"OK\n", chunk).await?;
        }
        "count" => {
            let st = state.lock().await;
            if rest == "playlist" {
                let n = st.playlist.len();
                write_all_chunked(
                    w,
                    format!("songs: {n}\nplaytime: 0\nplaylist: {n}\n").as_bytes(),
                    chunk,
                )
                .await?;
            } else {
                // Filtered count: report the number of matching search results.
                let n = filter_tag(unquote(rest))
                    .map(|t| count_tagged(&st.search, t))
                    .unwrap_or(st.search.len());
                write_all_chunked(w, format!("songs: {n}\nplaytime: 0\n").as_bytes(), chunk)
                    .await?;
            }
            write_all_chunked(w, b"OK\n", chunk).await?;
        }
        "commands" => {
            let st = state.lock().await;
            for c in &st.commands {
                write_all_chunked(w, format!("command: {c}\n").as_bytes(), chunk).await?;
            }
            write_all_chunked(w, b"OK\n", chunk).await?;
        }
        "notcommands" => {
            let st = state.lock().await;
            for c in &st.not_commands {
                write_all_chunked(w, format!("command: {c}\n").as_bytes(), chunk).await?;
            }
            write_all_chunked(w, b"OK\n", chunk).await?;
        }
        "idle" => {
            let (kind, stray) = {
                let mut st = state.lock().await;
                let kind = if st.next_idle_change.is_empty() {
                    None
                } else {
                    Some(std::mem::take(&mut st.next_idle_change))
                };
                let stray = std::mem::take(&mut st.next_idle_stray);
                (kind, stray)
            };
            if let Some(k) = kind {
                write_all_chunked(w, format!("changed: {k}\n").as_bytes(), chunk).await?;
            }
            if !stray.is_empty() {
                write_all_chunked(w, format!("{stray}\n").as_bytes(), chunk).await?;
            }
            write_all_chunked(w, b"OK\n", chunk).await?;
            // Be gentle: avoid a hot idle loop in tests.
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        "readpicture" | "albumart" => {
            let st = state.lock().await;
            match &st.art {
                Some((bytes, mime)) => {
                    let n = bytes.len();
                    write_all_chunked(
                        w,
                        format!("size: {n}\ntype: {mime}\nbinary: {n}\n").as_bytes(),
                        chunk,
                    )
                    .await?;
                    // The payload itself is chunked too: `binary: N` must
                    // reassemble across segment boundaries.
                    write_all_chunked(w, bytes, chunk).await?;
                    write_all_chunked(w, b"\nOK\n", chunk).await?;
                }
                None => {
                    write_all_chunked(w, b"ACK [34@1] No such file or directory\n", chunk).await?;
                }
            }
        }
        // MPD 0.22 removed the command_list_* commands; a real server
        // answers them with an "unknown command" ACK.
        "command_list_start" | "command_list_end" => {
            write_all_chunked(
                w,
                format!("ACK [5@0] {{}} unknown command \"{name}\"\n").as_bytes(),
                chunk,
            )
            .await?;
        }
        "add" => {
            let mut st = state.lock().await;
            let file = unquote(rest);
            let id = st.next_id;
            st.next_id += 1;
            st.playlist.push(vec![
                ("file".to_string(), file.to_string()),
                ("Id".to_string(), id.to_string()),
            ]);
            write_all_chunked(w, b"OK\n", chunk).await?;
        }
        "searchadd" => {
            let mut st = state.lock().await;
            let tag = filter_tag(unquote(rest));
            let groups = st.search.clone();
            let mut items = Vec::new();
            for group in &groups {
                if tag
                    .map(|t| group.iter().any(|(k, _)| k == t))
                    .unwrap_or(true)
                {
                    let mut item = group.clone();
                    let id = st.next_id;
                    st.next_id += 1;
                    if let Some(pos) = item.iter().position(|(k, _)| k == "Id") {
                        item[pos].1 = id.to_string();
                    } else {
                        item.push(("Id".to_string(), id.to_string()));
                    }
                    items.push(item);
                }
            }
            st.playlist.append(&mut items);
            write_all_chunked(w, b"OK\n", chunk).await?;
        }
        "clear" => {
            state.lock().await.playlist.clear();
            write_all_chunked(w, b"OK\n", chunk).await?;
        }
        "clearid" => {
            let mut st = state.lock().await;
            remove_range(&mut st.playlist, rest);
            write_all_chunked(w, b"OK\n", chunk).await?;
        }
        "deleteid" => {
            let mut st = state.lock().await;
            if let Ok(pos) = rest.parse::<usize>() {
                if pos < st.playlist.len() {
                    st.playlist.remove(pos);
                }
            }
            write_all_chunked(w, b"OK\n", chunk).await?;
        }
        "moveid" => {
            let mut st = state.lock().await;
            if let Some((a, b)) = rest.split_once(' ') {
                if let (Ok(from), Ok(to)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>()) {
                    if from < st.playlist.len() {
                        let item = st.playlist.remove(from);
                        let to = to.min(st.playlist.len());
                        st.playlist.insert(to, item);
                    }
                }
            }
            write_all_chunked(w, b"OK\n", chunk).await?;
        }
        // play, pause, stop, next, previous, seekcur, setvol, random, repeat,
        // single, consume, shuffle, and anything else: accept.
        _ => {
            let _ = rest;
            write_all_chunked(w, b"OK\n", chunk).await?;
        }
    }
    let _ = w.flush().await;
    Ok(())
}

async fn write_fields(
    w: &mut tokio::net::tcp::OwnedWriteHalf,
    fields: &FieldList,
    chunk: usize,
) -> std::io::Result<()> {
    for (k, v) in fields {
        write_all_chunked(w, format!("{k}: {v}\n").as_bytes(), chunk).await?;
    }
    Ok(())
}

/// Parse an MPD positional spec (`N`, `a : b`, `a : ` for "to the end") and
/// remove that inclusive range from the playlist, clamped to its length.
fn remove_range(playlist: &mut Vec<FieldList>, spec: &str) {
    let spec = spec.trim();
    let (start, end) = if let Some((a, b)) = spec.split_once(':') {
        (
            a.trim().parse::<usize>().unwrap_or(0),
            b.trim()
                .parse::<usize>()
                .unwrap_or(playlist.len().saturating_sub(1)),
        )
    } else {
        let i = spec.parse::<usize>().unwrap_or(0);
        (i, i)
    };
    let end = end.min(playlist.len().saturating_sub(1));
    if !playlist.is_empty() && start <= end {
        playlist.drain(start..=end);
    }
}

/// The tag name of a single-clause search filter, e.g. `(Artist == "A")`.
fn filter_tag(rest: &str) -> Option<&str> {
    let inner = rest.strip_prefix('(')?.strip_suffix(')')?;
    inner.split(' ').next()
}

/// Number of item groups carrying a field with the given tag.
fn count_tagged(groups: &[FieldList], tag: &str) -> usize {
    groups
        .iter()
        .filter(|group| group.iter().any(|(k, _)| k == tag))
        .count()
}

/// Strip a matching pair of surrounding double quotes from a protocol argument,
/// mirroring how MPD unquotes command arguments.
fn unquote(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        &s[1..s.len() - 1]
    } else {
        s
    }
}
