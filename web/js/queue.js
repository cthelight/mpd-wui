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
    <li class="queue-row${isCurrent ? " current" : ""}" data-id="${song.id ?? ""}" data-index="${index}">
      <button class="queue-handle" type="button" title="Drag to reorder">${icon("grip", 16)}</button>
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

  const section = container.querySelector(".queue-view");
  const list = container.querySelector("[data-list]");
  const count = container.querySelector("[data-count]");
  const dropSlot = document.createElement("li");
  dropSlot.className = "queue-drop-slot";
  dropSlot.hidden = true;
  list.appendChild(dropSlot);

  let songs = [];
  let version = null;
  let refreshToken = 0;
  let dragging = null;
  let dropTarget = null;

  function currentId() {
    return state.snapshot?.song?.id ?? null;
  }

  function render() {
    list.innerHTML = songs.map((song, i) => rowHtml(song, i, currentId())).join("");
    // render() wipes the list; re-home the drop slot and drop any pending target.
    // A live drag re-places the slot on the next dragover.
    list.appendChild(dropSlot);
    dropSlot.hidden = true;
    dropTarget = null;
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
    if (event.target.closest(".queue-handle")) return;
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

  // Reordering is initiated from the grip handle only; the row is made
  // draggable for the duration of that press so clicks and text selection
  // on the rest of the row keep working as before. Any drag is preceded by a
  // mousedown on the list, so clearing here keeps the attribute honest.
  const clearDraggable = () =>
    list.querySelectorAll(".queue-row[draggable]").forEach((row) => row.removeAttribute("draggable"));

  list.addEventListener("mousedown", (event) => {
    clearDraggable();
    const handle = event.target.closest(".queue-handle");
    if (handle) handle.closest(".queue-row")?.setAttribute("draggable", "true");
  });

  list.addEventListener("mouseup", clearDraggable);

  list.addEventListener("dragstart", (event) => {
    const row = event.target.closest(".queue-row");
    if (!row) return;
    dragging = { id: Number(row.dataset.id), index: Number(row.dataset.index) };
    row.classList.add("dragging");
    section.classList.add("is-dragging");
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", String(dragging.id));
  });

  function clearDragState() {
    dragging = null;
    dropTarget = null;
    dropSlot.hidden = true;
    section.classList.remove("is-dragging");
    list.querySelectorAll(".queue-row.dragging").forEach((row) => row.classList.remove("dragging"));
    clearDraggable();
  }

  list.addEventListener("dragend", clearDragState);

  // Row whose center is closest to the pointer; also resolves the gaps
  // between rows, the header and the area below the last row.
  function nearestRow(y) {
    let best = null;
    let bestDist = Infinity;
    for (const row of list.querySelectorAll(".queue-row")) {
      const rect = row.getBoundingClientRect();
      const dist = Math.abs(y - (rect.top + rect.height / 2));
      if (dist < bestDist) {
        bestDist = dist;
        best = row;
      }
    }
    return best;
  }

  function setDropTarget(row, after) {
    if (dropTarget && dropTarget.row === row && dropTarget.after === after) return;
    dropTarget = { row, after };
    dropSlot.hidden = false;
    // insertBefore(node, node) is a no-op, so this is safe when the slot
    // already sits in the requested position.
    list.insertBefore(dropSlot, after ? row.nextSibling : row);
  }

  section.addEventListener("dragover", (event) => {
    if (!dragging) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    const row = nearestRow(event.clientY);
    if (!row) return;
    const rect = row.getBoundingClientRect();
    setDropTarget(row, event.clientY > rect.top + rect.height / 2);
  });

  section.addEventListener("drop", (event) => {
    event.preventDefault();
    if (!dragging || !dropTarget) return;
    const targetIndex = Number(dropTarget.row.dataset.index);
    let to = dropTarget.after ? targetIndex + 1 : targetIndex;
    if (dragging.index < to) to -= 1;
    if (to !== dragging.index) act("move", { id: dragging.id, to });
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
