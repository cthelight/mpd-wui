// mpd-wui app entry: mounts views, wires the WebSocket status stream.
import { get, post } from "./api.js";
import { config } from "./config.js";
import { mountNowPlaying, renderMiniPlayer } from "./nowplaying.js";
import { mountQueue } from "./queue.js";
import { mountLibrary } from "./library.js";
import { defaultLibraryRoute, parseRoute, routeToHash } from "./router.js";
import { toast } from "./util.js";

function fail(err) {
  toast(err?.message || String(err));
}

const state = {
  // Starts null (not "nowplaying") so the initial applyRoute always counts as a
  // switch and mounts the default view's pane. A pre-seeded "nowplaying" would
  // make the first apply a no-op, leaving the Now Playing page empty until the
  // user navigated away and back.
  view: null,
  snapshot: null,
  lastSync: 0,
};

const viewEl = document.getElementById("view");
const miniEl = document.getElementById("minibar");
const connEl = document.querySelector(".conn");
const bannerEl = document.getElementById("conn-banner");
const bannerMsg = document.querySelector("[data-banner-msg]");

// MPD reachability is distinct from the WebSocket link being open: the server
// can push a stale snapshot while MPD is down. Track the two separately and
// surface the MPD state in the dot + banner.
let mpdUp = false;

function setMpd(up, message) {
  mpdUp = up;
  connEl?.classList.toggle("up", up);
  if (!bannerEl) return;
  if (up) {
    bannerEl.hidden = true;
  } else {
    bannerMsg.textContent = message || "MPD is unreachable";
    bannerEl.hidden = false;
  }
}

// `/status` queues behind a dead command connection for up to the server's
// 60s command timeout, so bound the probe client-side to keep the banner
// responsive. A successful probe doubles as a state refresh.
async function probeMpd() {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 8000);
  try {
    setSnapshot(await get("/status", undefined, controller.signal));
    setMpd(true);
  } catch (err) {
    const message =
      err?.name === "AbortError" ? "MPD may be unreachable (timed out)" : err?.message || String(err);
    setMpd(false, message);
  } finally {
    clearTimeout(timer);
  }
}

// While a command is in flight, disable the transport and mode buttons so a
// slow/hung request cannot be queued behind a flood of identical clicks.
let pending = 0;
function setPending(delta) {
  pending = Math.max(0, pending + delta);
  const busy = pending > 0;
  document.querySelectorAll(".ctl, .mode").forEach((button) => (button.disabled = busy));
}

function send(path, body) {
  setPending(1);
  post(path, body)
    .catch(fail)
    .finally(() => setPending(-1));
}

const actions = {
  playPause() {
    const status = state.snapshot?.status;
    if (!status) return;
    if (status.state === "play") send("/pause", { state: true });
    else send("/play", {});
  },
  next() {
    send("/next", {});
  },
  previous() {
    send("/previous", {});
  },
  stop() {
    send("/stop", {});
  },
  seek(time) {
    send("/seek", { time });
    if (state.snapshot) {
      state.snapshot.status.elapsed = time;
      state.lastSync = performance.now();
      mountedViews.forEach((view) => view.progress(time));
    }
  },
  volume(value) {
    send("/volume", { value });
    if (state.snapshot) state.snapshot.status.volume = value;
  },
  toggleOption(key) {
    if (!state.snapshot) return;
    const next = !state.snapshot.status[key];
    state.snapshot.status[key] = next;
    renderSnapshot();
    setPending(1);
    post("/options", { [key]: next })
      .catch((err) => {
        // The command failed: re-sync with the server instead of keeping
        // a state MPD never applied.
        fail(err);
        get("/status").then(setSnapshot).catch(() => {});
      })
      .finally(() => setPending(-1));
  },
};

let mountedViews = [];
let miniView = renderMiniPlayer(miniEl, actions, navigate);
mountedViews.push(miniView);

function renderSnapshot() {
  if (!state.snapshot) return;
  mountedViews.forEach((view) => view.update(state.snapshot));
}

function setSnapshot(snapshot) {
  state.snapshot = snapshot;
  state.lastSync = performance.now();
  renderSnapshot();
  const song = snapshot?.song;
  const name = song?.title || (song?.file ? song.file.split("/").pop() : "");
  document.title = name ? `${name} · ${config.appTitle}` : config.appTitle;
}

function interpolatedElapsed() {
  const status = state.snapshot?.status;
  if (!status) return 0;
  if (status.state !== "play") return status.elapsed || 0;
  const delta = (performance.now() - state.lastSync) / 1000;
  const elapsed = (status.elapsed || 0) + delta;
  return status.time ? Math.min(elapsed, status.time) : elapsed;
}

// Each main view is mounted once (lazily) into its own pane and kept alive, so
// switching tabs preserves scroll position and in-flight state (browse path,
// collection stack, search query) instead of tearing the view down.
const viewPanes = {};

function ensureView(name) {
  if (viewPanes[name]) return;
  const pane = document.createElement("div");
  pane.className = "view-pane";
  pane.hidden = true;
  viewEl.appendChild(pane);
  viewPanes[name] = pane;

  if (name === "nowplaying") {
    const view = mountNowPlaying(pane, actions);
    if (state.snapshot) view.update(state.snapshot);
    mountedViews.push(view);
  } else if (name === "queue") {
    mountedViews.push(mountQueue(pane, state));
  } else if (name === "library") {
    // The pane is only mounted while it is the active view, so the current
    // route is a library route; passing it in makes a deep link apply its
    // full state instead of loading the default tab first.
    libraryView = mountLibrary(pane, navigate, currentRoute);
    mountedViews.push(libraryView);
  }
}

function renderView() {
  ensureView(state.view);
  for (const name in viewPanes) viewPanes[name].hidden = name !== state.view;
}

// Navigation is driven by the location hash (see router.js): every view
// change is a history entry, so the browser back/forward buttons undo (and
// redo) navigation steps, and any view reloads and shares as a URL.
let currentHash = "";
let currentRoute = null;
let libraryView = null;

function applyRoute(route) {
  currentRoute = route;
  const switched = route.view !== state.view;
  state.view = route.view;
  // Synced on every apply (not just on a switch) so the first apply — which
  // may re-select an already-active tab — still lands on a consistent state.
  document.querySelectorAll(".tab").forEach((button) => {
    const active = button.dataset.view === route.view;
    button.classList.toggle("active", active);
    button.setAttribute("aria-selected", active ? "true" : "false");
  });
  if (switched) renderView();
  if (route.view === "library") libraryView?.restore(route);
}

function navigate(route, replace = false) {
  const hash = routeToHash(route);
  if (hash === currentHash) return false;
  if (replace) history.replaceState(null, "", hash);
  else history.pushState(null, "", hash);
  currentHash = hash;
  applyRoute(route);
  return true;
}

let wsAttempts = 0;

function connect() {
  const url = config.wsUrl(location.protocol, location.host);
  const ws = new WebSocket(url);
  let closed = false;

  ws.onopen = () => {
    wsAttempts = 0;
    // The link is up, but the server may be pushing a stale snapshot while
    // MPD is down; probe to learn the real state.
    probeMpd();
  };
  ws.onmessage = (event) => {
    let message;
    try {
      message = JSON.parse(event.data);
    } catch {
      return;
    }
    if (message.type === "status" && message.snapshot) {
      setSnapshot(message.snapshot);
    } else if (message.type === "disconnected") {
      // Server→MPD command connection lost; authoritative for MPD health.
      setMpd(false, "MPD connection lost");
    } else if (message.type === "reconnected") {
      setMpd(true);
    } else if (message.type === "database-changed") {
      mountedViews.forEach((view) => view.refresh?.());
    }
  };
  ws.onclose = () => {
    closed = true;
    // The browser→server link dropped; MPD state is unknown until we reach
    // the server again (onopen probes).
    setMpd(false, "Lost connection to mpd-wui");
    // Back off exponentially (750ms, 1.5s, 3s, ...) capped at 10s; a healthy
    // open resets the counter.
    const delay = Math.min(750 * 2 ** wsAttempts, 10000);
    wsAttempts += 1;
    setTimeout(connect, delay);
  };
  ws.onerror = () => {
    if (!closed) ws.close();
  };
}

function loadInitial() {
  probeMpd();
}

// The library tab resumes the library where it was left (active tab, files
// path, collection drill or search), since its pane is kept alive.
function libraryRoute() {
  return libraryView?.route() ?? defaultLibraryRoute();
}

document.querySelectorAll(".tab").forEach((button) => {
  button.addEventListener("click", () => {
    const view = button.dataset.view;
    navigate(view === "library" ? libraryRoute() : { view });
  });
});

// The wordmark is the app's home affordance: clicking it returns to the Now
// Playing view, matching the mini-player's art/track-info shortcut.
document.querySelector(".wordmark")?.addEventListener("click", () => {
  navigate({ view: "nowplaying" });
});

document.querySelector("[data-banner-retry]")?.addEventListener("click", () => {
  probeMpd();
});

function isFormTarget(target) {
  return target?.closest?.("input, textarea, select, [contenteditable='true']");
}

function seekBy(delta) {
  const status = state.snapshot?.status;
  if (!status?.time) return;
  const elapsed = (status.elapsed || 0) + delta;
  actions.seek(Math.min(status.time, Math.max(0, elapsed)));
}

function adjustVolume(delta) {
  const current = state.snapshot?.status?.volume ?? 0;
  actions.volume(Math.min(100, Math.max(0, current + delta)));
}

document.addEventListener("keydown", (event) => {
  if (event.metaKey || event.ctrlKey || event.altKey) return;
  if (isFormTarget(event.target)) return;
  // Let focused rows (library drill rows) handle Enter/Space themselves.
  if (event.defaultPrevented) return;
  switch (event.key) {
    case " ":
      event.preventDefault();
      actions.playPause();
      break;
    case "ArrowLeft":
      event.preventDefault();
      seekBy(-10);
      break;
    case "ArrowRight":
      event.preventDefault();
      seekBy(10);
      break;
    case "ArrowUp":
      event.preventDefault();
      adjustVolume(5);
      break;
    case "ArrowDown":
      event.preventDefault();
      adjustVolume(-5);
      break;
    case "n":
    case "N":
      actions.next();
      break;
    case "p":
    case "P":
      actions.previous();
      break;
    case "s":
    case "S":
      actions.stop();
      break;
    case "r":
    case "R":
      actions.toggleOption("repeat");
      break;
  }
});

function onHistoryChange() {
  // Back/forward (popstate) and a hand-edited hash (hashchange) both land
  // here; the hash guard makes the double-fire of a single traversal a no-op.
  const hash = location.hash;
  if (hash === currentHash) return;
  currentHash = hash;
  applyRoute(parseRoute(hash));
}
window.addEventListener("popstate", onHistoryChange);
window.addEventListener("hashchange", onHistoryChange);

// Initial route: a reloaded or shared deep link applies immediately, and the
// hash is normalized so the bar always reflects the current view.
{
  const route = parseRoute(location.hash);
  currentHash = routeToHash(route);
  if (location.hash !== currentHash) history.replaceState(null, "", currentHash);
  applyRoute(route);
}
loadInitial();
connect();

setInterval(() => {
  if (!state.snapshot) return;
  const elapsed = interpolatedElapsed();
  mountedViews.forEach((view) => view.progress(elapsed));
}, 250);
