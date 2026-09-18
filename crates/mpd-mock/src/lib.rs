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
        let addr = listener.local_addr().unwrap();
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
        self.state.lock().await.playlist = items;
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

    let _ = w.write_all(b"OK MPD 0.23.0\n").await;
    let _ = w.flush().await;

    let mut line = String::new();
    loop {
        line.clear();
        let n = match reader.read_line(&mut line).await {
            Ok(n) => n,
            Err(_) => break,
        };
        if n == 0 {
            break;
        }
        let cmd = line.trim().to_string();
        if cmd.is_empty() {
            continue;
        }
        if handle_cmd(&cmd, &mut w, &state).await.is_err() {
            return;
        }
    }
}

async fn handle_cmd(
    cmd: &str,
    w: &mut tokio::net::tcp::OwnedWriteHalf,
    state: &Arc<Mutex<MockState>>,
) -> std::io::Result<()> {
    let (name, rest) = match cmd.split_once(' ') {
        Some((n, r)) => (n, r.trim()),
        None => (cmd, ""),
    };

    match name {
        "password" => {
            let st = state.lock().await;
            if st.password.as_deref() == Some(unquote(rest)) {
                w.write_all(b"OK\n").await?;
            } else {
                w.write_all(b"ACK [50@1] password not set\n").await?;
            }
        }
        "status" => {
            let st = state.lock().await;
            write_fields(w, &st.status).await?;
            w.write_all(b"OK\n").await?;
        }
        "currentsong" => {
            let st = state.lock().await;
            if let Some(song) = &st.currentsong {
                write_fields(w, song).await?;
            }
            w.write_all(b"OK\n").await?;
        }
        "playlistinfo" => {
            let st = state.lock().await;
            for item in &st.playlist {
                write_fields(w, item).await?;
            }
            w.write_all(b"OK\n").await?;
        }
        "lsinfo" => {
            let st = state.lock().await;
            write_fields(w, &st.lsinfo).await?;
            w.write_all(b"OK\n").await?;
        }
        "list" => {
            let st = state.lock().await;
            write_fields(w, &st.list).await?;
            w.write_all(b"OK\n").await?;
        }
        "search" => {
            let st = state.lock().await;
            for item in &st.search {
                write_fields(w, item).await?;
            }
            w.write_all(b"OK\n").await?;
        }
        "count" => {
            w.write_all(b"songs: 0\nplaytime: 0\nOK\n").await?;
        }
        "commands" => {
            let st = state.lock().await;
            for c in &st.commands {
                w.write_all(format!("command: {c}\n").as_bytes()).await?;
            }
            w.write_all(b"OK\n").await?;
        }
        "notcommands" => {
            let st = state.lock().await;
            for c in &st.not_commands {
                w.write_all(format!("command: {c}\n").as_bytes()).await?;
            }
            w.write_all(b"OK\n").await?;
        }
        "idle" => {
            let kind = {
                let mut st = state.lock().await;
                if st.next_idle_change.is_empty() {
                    None
                } else {
                    Some(std::mem::take(&mut st.next_idle_change))
                }
            };
            if let Some(k) = kind {
                w.write_all(format!("changed: {k}\n").as_bytes()).await?;
            }
            w.write_all(b"OK\n").await?;
            // Be gentle: avoid a hot idle loop in tests.
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        "readpicture" | "albumart" => {
            let st = state.lock().await;
            match &st.art {
                Some((bytes, mime)) => {
                    let n = bytes.len();
                    w.write_all(format!("size: {n}\ntype: {mime}\nbinary: {n}\n").as_bytes())
                        .await?;
                    w.write_all(bytes).await?;
                    w.write_all(b"\nOK\n").await?;
                }
                None => {
                    w.write_all(b"ACK [34@1] No such file or directory\n")
                        .await?;
                }
            }
        }
        // MPD 0.22 removed the command_list_* commands; a real server
        // answers them with an "unknown command" ACK.
        "command_list_start" | "command_list_end" => {
            w.write_all(
                format!("ACK [5@0] {{}} unknown command \"{name}\"\n").as_bytes(),
            )
            .await?;
        }
        // play, pause, stop, next, previous, seekcur, setvol, random, repeat,
        // single, consume, add, searchadd, findadd, clear, deleteid, moveid,
        // shuffle, and anything else: accept.
        _ => {
            let _ = rest;
            w.write_all(b"OK\n").await?;
        }
    }
    let _ = w.flush().await;
    Ok(())
}

async fn write_fields(
    w: &mut tokio::net::tcp::OwnedWriteHalf,
    fields: &FieldList,
) -> std::io::Result<()> {
    for (k, v) in fields {
        w.write_all(format!("{k}: {v}\n").as_bytes()).await?;
    }
    Ok(())
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
