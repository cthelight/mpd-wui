// Queue view: full list, drag-to-reorder, click-to-play, remove, clear, shuffle.
import { get, post } from "./api.js";
import { icon } from "./icons.js";
import { formatTime } from "./nowplaying.js";

function title(song) {
  return song.title || song.file.split("/").pop() || song.file || "Unknown track";
}

function artist(song) {
  return song.artist || song.albumartist || "";
}

function rowHtml(song, index, currentId) {
  const isCurrent = currentId != null && song.id == currentId;
  return `
    <li class="queue-row${isCurrent ? " current" : ""}" draggable="true" data-id="${song.id ?? ""}" data-index="${index}">
      <span class="queue-index">${index + 1}</span>
      <button class="row-btn" data-row-act="play" title="Play">${icon("play", 16)}</button>
      <div class="row-meta">
        <span class="row-title" title="${escapeHtml(title(song))}">${escapeHtml(title(song))}</span>
        <span class="row-artist" title="${escapeHtml(artist(song))}">${escapeHtml(artist(song))}</span>
      </div>
      <span class="row-time">${formatTime(song.time)}</span>
      <button class="row-btn" data-row-act="remove" title="Remove">${icon("trash", 16)}</button>
    </li>
  `;
}

function escapeHtml(value) {
  return String(value ?? "").replace(/[&<>"']/g, (ch) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  }[ch]));
}

export function mountQueue(container, state) {
  container.innerHTML = `
    <section class="queue-view">
      <header class="queue-header">
        <h1>Queue</h1>
        <div class="queue-actions">
          <button data-queue-act="shuffle" title="Shuffle">${icon("shuffle", 16)} Shuffle</button>
          <button data-queue-act="clear" title="Clear">${icon("trash", 16)} Clear</button>
        </div>
      </header>
      <ol class="queue-list" data-list></ol>
      <div class="queue-footer">
        <span data-count class="queue-count"></span>
      </div>
    </section>
  `;

  const list = container.querySelector("[data-list]");
  const count = container.querySelector("[data-count]");

  let songs = [];
  let version = null;
  let refreshToken = 0;
  let dragging = null;
  let dropAfter = false;

  function currentId() {
    return state.snapshot?.song?.id ?? null;
  }

  function render() {
    list.innerHTML = songs.map((song, i) => rowHtml(song, i, currentId())).join("");
    count.textContent = `${songs.length} tracks`;
  }

  async function refresh() {
    const token = ++refreshToken;
    try {
      const page = await get("/playlist");
      if (token !== refreshToken) return;
      songs = page;
      render();
    } catch {
      // The next snapshot or manual refresh will retry.
    }
  }

  async function act(action, extra = {}) {
    try {
      if (action === "shuffle") await post("/queue/shuffle", {});
      if (action === "clear") await post("/queue/clear", {});
      if (action === "remove") await post("/queue/remove", { ids: [extra.id] });
      if (action === "move") await post("/queue/move", { id: extra.id, to: extra.to });
      await refresh();
    } catch (err) {
      console.warn("queue action failed", err);
    }
  }

  list.addEventListener("click", (event) => {
    const button = event.target.closest("[data-row-act]");
    const row = event.target.closest(".queue-row");
    if (!row) return;
    const id = Number(row.dataset.id);
    const index = Number(row.dataset.index);
    const action = button?.dataset.rowAct;
    if (action === "remove") {
      act("remove", { id });
    } else if (action === "play" || !action) {
      post("/play", { position: index }).catch((err) => console.warn(err));
    }
  });

  container.querySelector("[data-queue-act=shuffle]").addEventListener("click", () => act("shuffle"));
  container.querySelector("[data-queue-act=clear]").addEventListener("click", () => act("clear"));

  // Fetch the whole queue on mount: the app may already hold a snapshot
  // (from /status or WS), but no new snapshot arrives unless MPD changes,
  // so an idle MPD would otherwise leave the list empty.
  version = state.snapshot?.status?.playlist_version ?? null;
  refresh();

  list.addEventListener("dragstart", (event) => {
    const row = event.target.closest(".queue-row");
    if (!row) return;
    dragging = { id: Number(row.dataset.id), index: Number(row.dataset.index) };
    row.classList.add("dragging");
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", String(dragging.id));
  });

  list.addEventListener("dragend", () => {
    dragging = null;
    list.querySelectorAll(".queue-row").forEach((row) => {
      row.classList.remove("dragging", "drop-before", "drop-after");
    });
  });

  list.addEventListener("dragover", (event) => {
    const row = event.target.closest(".queue-row");
    if (!row || !dragging) return;
    event.preventDefault();
    const rect = row.getBoundingClientRect();
    dropAfter = event.clientY > rect.top + rect.height / 2;
    row.classList.toggle("drop-before", !dropAfter);
    row.classList.toggle("drop-after", dropAfter);
  });

  list.addEventListener("drop", (event) => {
    event.preventDefault();
    const row = event.target.closest(".queue-row");
    if (!row || !dragging) return;
    const targetIndex = Number(row.dataset.index);
    if (targetIndex === dragging.index) return;
    let to = dropAfter ? targetIndex + 1 : targetIndex;
    if (dragging.index < to) to -= 1;
    act("move", { id: dragging.id, to });
  });

  return {
    update(snapshot) {
      if (!snapshot?.status) return;
      const newVersion = snapshot.status.playlist_version;
      if (version === null || newVersion !== version) {
        version = newVersion;
        refresh();
      } else {
        render();
      }
    },
    progress() {},
    refresh,
  };
}
