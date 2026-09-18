// mpd-wui app entry: mounts views, wires the WebSocket status stream.
import { get, post } from "./api.js";
import { config } from "./config.js";
import { mountNowPlaying, renderMiniPlayer } from "./nowplaying.js";
import { mountQueue } from "./queue.js";
import { mountLibrary } from "./library.js";
import { toast } from "./util.js";

function fail(err) {
  toast(err?.message || String(err));
}

const state = {
  view: "nowplaying",
  snapshot: null,
  connected: false,
  lastSync: 0,
};

const viewEl = document.getElementById("view");
const miniEl = document.getElementById("minibar");
const connEl = document.querySelector(".conn");

const actions = {
  playPause() {
    const status = state.snapshot?.status;
    if (!status) return;
    if (status.state === "play") post("/pause", { state: true }).catch(fail);
    else post("/play", {}).catch(fail);
  },
  next() {
    post("/next", {}).catch(fail);
  },
  previous() {
    post("/previous", {}).catch(fail);
  },
  stop() {
    post("/stop", {}).catch(fail);
  },
  seek(time) {
    post("/seek", { time }).catch(fail);
    if (state.snapshot) {
      state.snapshot.status.elapsed = time;
      state.lastSync = performance.now();
      mountedViews.forEach((view) => view.progress(time));
    }
  },
  volume(value) {
    post("/volume", { value }).catch(fail);
    if (state.snapshot) state.snapshot.status.volume = value;
  },
  toggleOption(key) {
    if (!state.snapshot) return;
    const next = !state.snapshot.status[key];
    state.snapshot.status[key] = next;
    renderSnapshot();
    post("/options", { [key]: next }).catch((err) => {
      // The command failed: re-sync with the server instead of keeping
      // a state MPD never applied.
      fail(err);
      get("/status").then(setSnapshot).catch(() => {});
    });
  },
};

let mountedViews = [];
let miniView = renderMiniPlayer(miniEl, actions);
mountedViews.push(miniView);

function setConn(up) {
  state.connected = up;
  connEl?.classList.toggle("up", up);
}

function renderSnapshot() {
  if (!state.snapshot) return;
  mountedViews.forEach((view) => view.update(state.snapshot));
}

function setSnapshot(snapshot) {
  state.snapshot = snapshot;
  state.lastSync = performance.now();
  renderSnapshot();
}

function interpolatedElapsed() {
  const status = state.snapshot?.status;
  if (!status) return 0;
  if (status.state !== "play") return status.elapsed || 0;
  const delta = (performance.now() - state.lastSync) / 1000;
  const elapsed = (status.elapsed || 0) + delta;
  return status.time ? Math.min(elapsed, status.time) : elapsed;
}

function renderView() {
  // Keep the persistent mini-player; drop only the previously-mounted main view.
  mountedViews = mountedViews.filter((view) => view === miniView);
  viewEl.innerHTML = "";
  if (state.view === "nowplaying") {
    const view = mountNowPlaying(viewEl, actions);
    mountedViews.push(view);
    if (state.snapshot) view.update(state.snapshot);
  } else if (state.view === "queue") {
    mountedViews.push(mountQueue(viewEl, state));
  } else if (state.view === "library") {
    mountedViews.push(mountLibrary(viewEl));
  }
}

function setView(view) {
  state.view = view;
  document.querySelectorAll(".tab").forEach((button) => {
    button.classList.toggle("active", button.dataset.view === view);
  });
  renderView();
}

let wsAttempts = 0;

function connect() {
  const url = config.wsUrl(location.protocol, location.host);
  const ws = new WebSocket(url);
  let closed = false;

  ws.onopen = () => {
    wsAttempts = 0;
    setConn(true);
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
      setConn(false);
    } else if (message.type === "reconnected") {
      setConn(true);
    } else if (message.type === "database-changed") {
      mountedViews.forEach((view) => view.refresh?.());
    }
  };
  ws.onclose = () => {
    closed = true;
    setConn(false);
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

async function loadInitial() {
  try {
    setSnapshot(await get("/status"));
  } catch {
    setConn(false);
  }
}

document.querySelectorAll(".tab").forEach((button) => {
  button.addEventListener("click", () => setView(button.dataset.view));
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

setView("nowplaying");
loadInitial();
connect();

setInterval(() => {
  if (!state.snapshot) return;
  const elapsed = interpolatedElapsed();
  mountedViews.forEach((view) => view.progress(elapsed));
}, 250);
