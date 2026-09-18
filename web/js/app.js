// mpd-wui app entry: mounts views, wires the WebSocket status stream.
import { get, post } from "./api.js";
import { config } from "./config.js";
import { mountNowPlaying, renderMiniPlayer } from "./nowplaying.js";
import { mountQueue } from "./queue.js";
import { mountLibrary } from "./library.js";

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
    if (status.state === "play") post("/pause", { state: true });
    else post("/play", {});
  },
  next() {
    post("/next", {});
  },
  previous() {
    post("/previous", {});
  },
  stop() {
    post("/stop", {});
  },
  seek(time) {
    post("/seek", { time }).catch(() => {});
    if (state.snapshot) {
      state.snapshot.status.elapsed = time;
      state.lastSync = performance.now();
      mountedViews.forEach((view) => view.progress(time));
    }
  },
  volume(value) {
    post("/volume", { value }).catch(() => {});
    if (state.snapshot) state.snapshot.status.volume = value;
  },
  toggleOption(key) {
    if (!state.snapshot) return;
    const next = !state.snapshot.status[key];
    post("/options", { [key]: next }).catch(() => {});
    state.snapshot.status[key] = next;
  },
};

let mountedViews = [];
let miniView = renderMiniPlayer(miniEl, actions);
mountedViews.push(miniView);

function setConn(up) {
  state.connected = up;
  connEl?.classList.toggle("up", up);
}

function setSnapshot(snapshot) {
  state.snapshot = snapshot;
  state.lastSync = performance.now();
  mountedViews.forEach((view) => view.update(snapshot));
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
  mountedViews = mountedViews.filter((view) => view !== miniView);
  viewEl.innerHTML = "";
  if (state.view === "nowplaying") {
    const view = mountNowPlaying(viewEl, actions);
    mountedViews.push(view);
    if (state.snapshot) view.update(state.snapshot);
  } else if (state.view === "queue") {
    mountedViews.push(mountQueue(viewEl, state, actions));
  } else if (state.view === "library") {
    mountedViews.push(mountLibrary(viewEl, state, actions));
  }
}

function setView(view) {
  state.view = view;
  document.querySelectorAll(".tab").forEach((button) => {
    button.classList.toggle("active", button.dataset.view === view);
  });
  renderView();
}

function connect() {
  const url = config.wsUrl(location.protocol, location.host);
  const ws = new WebSocket(url);
  let closed = false;

  ws.onopen = () => {
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
      // Queue/library refresh hooks will use this in later phases.
    }
  };
  ws.onclose = () => {
    closed = true;
    setConn(false);
    setTimeout(connect, 750);
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

setView("nowplaying");
loadInitial();
connect();

setInterval(() => {
  if (!state.snapshot) return;
  const elapsed = interpolatedElapsed();
  mountedViews.forEach((view) => view.progress(elapsed));
}, 250);
