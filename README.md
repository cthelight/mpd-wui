# mpd-wui

A single-binary web UI for MPD. The Rust binary serves an embedded vanilla
HTML/CSS/JS frontend, a JSON API, and a WebSocket status stream on one port,
while bridging to an existing MPD server over TCP.

No runtime internet content is used: there are no CDNs, web fonts, or external
art lookups. All media, metadata, and album art come from the configured MPD
server.

## Features

- Now Playing view with art, transport controls, seek, volume, and playback
  modes
- Persistent mini-player
- Queue with play (replaces the queue with the played song), remove, clear,
  shuffle, and drag-to-reorder
- Library with debounced search (filterable by songs, artists, albums),
  folder browsing, and collections by artist, album artist, album, genre, and
  year
- Add or play songs, albums, artists, genres, years, and folders into the
  queue
- Live status over WebSocket with locally interpolated progress
- Browser back/forward support: every view change (tab, files level,
  collection drill, search) is a history entry, and any view reloads or
  shares as a URL (`#/queue`, `#/library/artist/Beethoven`, …)
- Dark, minimal UI with no frontend dependencies or build step

## Quickstart

```sh
export MPD_HOST=127.0.0.1
export MPD_PORT=6600
# export MPD_PASSWORD=secret   # optional

cargo run -p mpd-wui
```

Open <http://localhost:8080>.

For a release build:

```sh
cargo build --release --locked -p mpd-wui
./target/release/mpd-wui
```

## Configuration

| Variable       | Default   | Description                          |
| -------------- | --------- | ------------------------------------ |
| `MPD_HOST`     | `127.0.0.1` | Hostname or IP of the MPD server   |
| `MPD_PORT`     | `6600`    | MPD port                             |
| `MPD_PASSWORD` | _(empty)_ | Optional MPD password                |
| `BIND_ADDR`    | `0.0.0.0` | Address for the web server to bind   |
| `PORT`         | `8080`    | Port for the web server              |
| `CACHE_TTL`    | `300`     | Library snapshot TTL in seconds      |
| `APP_TITLE`    | `MPD: <host>` | Display name: wordmark and tab title |
| `RUST_LOG`     | `info`    | Logging filter                       |

`CACHE_TTL` controls how often the in-process library snapshot (used by
`/api/search`) is rebuilt; a stale snapshot is served immediately and
refreshed in the background until it is younger than the TTL. Album art is
cached separately for a fixed hour.

The web server performs no authentication of its own and binds `0.0.0.0` by
default. Anyone who can reach the port can control playback and the queue, so
restrict access with a firewall (or set `BIND_ADDR=127.0.0.1` for local-only
use) and rely on `MPD_PASSWORD` for the MPD bridge itself.

## API

| Method | Path                                        | Purpose                         |
| ------ | ------------------------------------------- | ------------------------------- |
| GET    | `/api/status`                               | Status and current song         |
| GET    | `/api/playlist`                             | Full playlist                   |
| GET    | `/api/browse?path=`                         | Browse a directory              |
| GET    | `/api/list?type=&artist=&albumartist=`      | List collection values          |
| GET    | `/api/search?q=&artist=&album=...&kinds=&limit=` | Fuzzy search: artists, albums, tracks; `kinds` is a comma list of which kinds to return (all by default) |
| GET    | `/api/capabilities`                         | MPD capabilities                |
| GET    | `/api/albumart?uri=`                        | Album art proxy                 |
| POST   | `/api/play`, `/api/pause`, `/api/stop`      | Transport controls (`play` accepts `{position?}` / `{id?}`, `clear: true` keeps only that song) |
| POST   | `/api/next`, `/api/previous`                | Track navigation                |
| POST   | `/api/seek`, `/api/volume`                  | Seek and volume                 |
| POST   | `/api/options`                              | Random/repeat/single/consume    |
| POST   | `/api/queue/add`, `/api/queue/remove`       | Modify the queue (`add` accepts `position: "after_current" \| "end"`) |
| POST   | `/api/queue/clear`, `/api/queue/move`       | Clear or reorder the queue      |
| POST   | `/api/queue/shuffle`                        | Shuffle the queue               |
| WS     | `/ws`                                       | Status and change events        |

## Keyboard shortcuts

| Key         | Action                  |
| ----------- | ----------------------- |
| `Space`     | Play/pause              |
| `←` / `→`   | Seek 10 seconds         |
| `N` / `P`   | Next / previous track   |
| `↑` / `↓`   | Volume up / down        |
| `S`         | Stop                    |
| `R`         | Toggle repeat           |

Shortcuts are ignored while typing in form fields.

## Docker

Build and run against an MPD server on the Docker host:

```sh
MPD_HOST=host.docker.internal MPD_PORT=6600 docker compose up --build
```

The compose file maps the container port to `${PORT:-8080}` and forwards
`MPD_HOST`, `MPD_PORT`, `MPD_PASSWORD`, `CACHE_TTL`, and `RUST_LOG`.

The image is multi-stage and ships only the release binary in a non-root
`debian:stable-slim` container.

## Development

```sh
make help       # list targets
make fmt        # format Rust and check JS syntax
make test       # run JS syntax checks and Rust tests
make clippy     # run Clippy with warnings denied
make run        # run the debug binary
```

Debug builds serve the frontend from `web/` on disk, so frontend edits only
need a browser refresh. Release builds embed the frontend in the binary.
