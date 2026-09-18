import { get, post } from "./api.js";
import { icon } from "./icons.js";
import { formatTime } from "./nowplaying.js";
import { escapeHtml, toast } from "./util.js";

const COLLECTION_TYPES = [
  { key: "artist", label: "Artists" },
  { key: "albumartist", label: "Album Artists" },
  { key: "album", label: "Albums" },
  { key: "genre", label: "Genres" },
  { key: "date", label: "Years" },
];

function labelFor(key) {
  return COLLECTION_TYPES.find((item) => item.key === key)?.label ?? key;
}

function targetForValue(key, value) {
  if (key === "artist") return { artist: value };
  if (key === "albumartist") return { albumartist: value };
  if (key === "album") return { album: value };
  if (key === "genre") return { genre: value };
  if (key === "date") return { date: value };
  return { path: value };
}

function songData(song) {
  const title = song.title || song.file.split("/").pop() || song.file || "Unknown track";
  const subtitle = [song.artist, song.album].filter(Boolean).join(" — ");
  return {
    kind: "song",
    target: { path: song.file },
    title,
    subtitle,
    time: song.time,
    drill: false,
  };
}

function dirData(entry) {
  // MPD's `lsinfo` often omits per-directory `songcount`/`playtime`; only show
  // the subtitle for the values that are actually known (avoid "0 tracks · 0:00").
  const parts = [];
  if (entry.songcount != null) parts.push(`${entry.songcount} tracks`);
  if (entry.playtime != null) parts.push(formatTime(entry.playtime));
  return {
    kind: "dir",
    target: { path: entry.path },
    title: entry.path.split("/").pop() || entry.path,
    subtitle: parts.join(" · "),
    drill: true,
    path: entry.path,
  };
}

function fileData(entry) {
  const song = entry.song;
  const title = song
    ? song.title || entry.path.split("/").pop() || entry.path
    : entry.path.split("/").pop() || entry.path;
  const subtitle = song ? [song.artist, song.album].filter(Boolean).join(" — ") : "";
  return {
    kind: "song",
    target: { path: entry.path },
    title,
    subtitle,
    time: song?.time,
    drill: false,
  };
}

function hitData(hit) {
  if (hit.kind === "artist") {
    return {
      kind: "hit",
      icon: "artist",
      title: hit.name,
      subtitle: `${hit.count} tracks`,
      drill: true,
      nav: { type: "artist", key: hit.key, name: hit.name },
    };
  }
  if (hit.kind === "album") {
    return {
      kind: "hit",
      icon: "album",
      title: hit.name,
      subtitle: [hit.artist, `${hit.count} tracks`].filter(Boolean).join(" — "),
      drill: true,
      nav: { type: "album", album: hit.name, albumartist: hit.artist ?? null },
    };
  }
  const row = songData(hit.song);
  row.kind = "hit";
  row.icon = "music";
  return row;
}

function valueData(targetKey, value, subtitle, drill) {
  return {
    kind: "value",
    target: targetForValue(targetKey, value),
    title: value,
    subtitle,
    drill,
    targetKey,
  };
}

function rowHtml(data, index) {
  const iconName =
    data.icon ?? (data.kind === "dir" ? "folder" : data.kind === "value" ? "music" : "file");
  const focusable = data.drill ? ' tabindex="0" role="button"' : "";
  return `
    <li class="row ${data.kind}${data.drill ? " drill" : ""}" data-index="${index}"${focusable}>
      <span class="row-icon">${icon(iconName, 18)}</span>
      <div class="row-meta">
        <span class="row-title" title="${escapeHtml(data.title)}">${escapeHtml(data.title)}</span>
        ${data.subtitle ? `<span class="row-artist" title="${escapeHtml(data.subtitle)}">${escapeHtml(data.subtitle)}</span>` : ""}
      </div>
      <span class="row-time">${data.time ? formatTime(data.time) : ""}</span>
      <div class="row-actions">
        <button class="row-btn" data-row-act="add" title="Add">${icon("plus", 16)}</button>
        <button class="row-btn primary" data-row-act="play" title="Play">${icon("play", 16)}</button>
      </div>
    </li>
  `;
}

function setRows(list, store, rows) {
  store.rows = rows;
  list.innerHTML = rows.length ? rows.map(rowHtml).join("") : `<div class="empty">No results</div>`;
}

function setMessage(list, store, message, className = "empty") {
  store.rows = [];
  list.innerHTML = `<div class="${className}">${escapeHtml(message)}</div>`;
}

async function queueTarget(target, play, button) {
  const original = button.innerHTML;
  button.disabled = true;
  try {
    await post("/queue/add", { targets: [target], play });
  } catch (err) {
    toast(err?.message || String(err));
  } finally {
    button.disabled = false;
    button.innerHTML = original;
  }
}

function bindList(list, store, onDrill) {
  list.addEventListener("click", (event) => {
    const row = event.target.closest(".row");
    if (!row) return;
    const data = store.rows[Number(row.dataset.index)];
    if (!data) return;
    const action = event.target.closest("[data-row-act]");
    if (action) {
      if (action.dataset.rowAct === "add") queueTarget(data.target, false, action);
      if (action.dataset.rowAct === "play") queueTarget(data.target, true, action);
      return;
    }
    if (data.drill) onDrill(data);
  });

  // Keyboard activation for focusable drill rows (Enter/Space). preventDefault
  // also stops the app-level shortcut handler from firing for the same key.
  list.addEventListener("keydown", (event) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    const row = event.target.closest?.(".row");
    if (!row || event.target !== row) return;
    const data = store.rows[Number(row.dataset.index)];
    if (!data) return;
    event.preventDefault();
    if (data.drill) onDrill(data);
  });
}

export function mountLibrary(container) {
  container.innerHTML = `
    <section class="library">
      <div class="library-search">
        <label class="search-box">
          ${icon("search", 18)}
          <input type="search" data-search placeholder="Search songs, artists, albums" aria-label="Search" />
        </label>
      </div>
      <nav class="library-tabs" data-tabs>
        <button class="lib-tab active" data-mode="browse">Browse</button>
        <button class="lib-tab" data-mode="collections">Collections</button>
      </nav>
      <div class="library-body">
        <section class="lib-pane" data-pane="browse">
          <div class="breadcrumb" data-browse-crumb></div>
          <div class="list" data-browse-list></div>
        </section>
        <section class="lib-pane" data-pane="collections" hidden>
          <div class="coll-types" data-coll-types>
            ${COLLECTION_TYPES.map(
              (item) => `<button class="coll-type" data-coll-type="${item.key}">${item.label}</button>`
            ).join("")}
          </div>
          <div class="breadcrumb" data-coll-crumb></div>
          <div class="list" data-coll-list></div>
        </section>
        <section class="lib-pane" data-pane="search" hidden>
          <div class="breadcrumb" data-search-crumb>Search results</div>
          <div class="list" data-search-list></div>
        </section>
      </div>
    </section>
  `;

  const searchInput = container.querySelector("[data-search]");
  const tabs = container.querySelector("[data-tabs]");
  const browsePane = container.querySelector('[data-pane="browse"]');
  const collPane = container.querySelector('[data-pane="collections"]');
  const searchPane = container.querySelector('[data-pane="search"]');
  const browseCrumb = container.querySelector("[data-browse-crumb]");
  const browseList = container.querySelector("[data-browse-list]");
  const collCrumb = container.querySelector("[data-coll-crumb]");
  const collList = container.querySelector("[data-coll-list]");
  const searchCrumb = container.querySelector("[data-search-crumb]");
  const searchList = container.querySelector("[data-search-list]");

  const browseStore = { rows: [] };
  const collStore = { rows: [] };
  const searchStore = { rows: [] };

  let activeMode = "browse";
  let browsePath = "";
  let browseToken = 0;
  let collStack = [];
  let collToken = 0;
  let searchTimer;
  let searchController;

  function showMode() {
    browsePane.hidden = activeMode !== "browse";
    collPane.hidden = activeMode !== "collections";
    searchPane.hidden = true;
    tabs.hidden = false;
    container.querySelectorAll("[data-mode]").forEach((button) => {
      button.classList.toggle("active", button.dataset.mode === activeMode);
    });
  }

  function renderBrowseBreadcrumb(path) {
    browseCrumb.innerHTML = "";
    const root = document.createElement("button");
    root.className = "crumb" + (path ? "" : " current");
    root.textContent = "Music";
    root.addEventListener("click", () => navigateBrowse(""));
    browseCrumb.append(root);
    if (!path) return;

    const parts = path.split("/");
    let prefix = "";
    parts.forEach((part, index) => {
      const sep = document.createElement("span");
      sep.className = "sep";
      sep.textContent = "/";
      browseCrumb.append(sep);

      prefix = prefix ? `${prefix}/${part}` : part;
      const isLast = index === parts.length - 1;
      const crumb = document.createElement("button");
      crumb.className = "crumb" + (isLast ? " current" : "");
      crumb.textContent = part;
      if (!isLast) crumb.addEventListener("click", () => navigateBrowse(prefix));
      browseCrumb.append(crumb);
    });
  }

  async function loadBrowse() {
    const token = ++browseToken;
    renderBrowseBreadcrumb(browsePath);
    setMessage(browseList, browseStore, "Loading…");
    try {
      const browse = await get("/browse", { path: browsePath || undefined });
      if (token !== browseToken) return;
      const rows = [...browse.directories.map(dirData), ...browse.files.map(fileData)];
      setRows(browseList, browseStore, rows);
    } catch (err) {
      if (token !== browseToken) return;
      setMessage(browseList, browseStore, err.message, "error");
    }
  }

  function navigateBrowse(path) {
    browsePath = path;
    loadBrowse();
  }

  function renderCollBreadcrumb() {
    collCrumb.innerHTML = "";
    if (!collStack.length) {
      const label = document.createElement("span");
      label.className = "crumb current";
      label.textContent = "Collections";
      collCrumb.append(label);
      return;
    }

    collStack.forEach((frame, index) => {
      if (index > 0) {
        const sep = document.createElement("span");
        sep.className = "sep";
        sep.textContent = "/";
        collCrumb.append(sep);
      }
      const crumb = document.createElement("button");
      crumb.className = "crumb" + (index === collStack.length - 1 ? " current" : "");
      crumb.textContent = frame.label;
      crumb.addEventListener("click", () => {
        const targetLength = index + 1;
        if (targetLength === collStack.length) return;
        collStack = collStack.slice(0, targetLength);
        loadCollection();
      });
      collCrumb.append(crumb);
    });
  }

  function updateCollTypeButtons() {
    const active = collStack[0]?.key;
    container.querySelectorAll("[data-coll-type]").forEach((button) => {
      button.classList.toggle("active", button.dataset.collType === active);
    });
  }

  async function loadCollection() {
    const token = ++collToken;
    renderCollBreadcrumb();
    updateCollTypeButtons();
    if (!collStack.length) {
      setMessage(collList, collStore, "Choose a collection");
      return;
    }

    setMessage(collList, collStore, "Loading…");
    const type = collStack[0].key;
    try {
      if (collStack.length === 1) {
        const values = await get("/list", { type });
        if (token !== collToken) return;
        setRows(collList, collStore, values.map((value) => valueData(type, value, "", true)));
      } else if (collStack.length === 2) {
        const value = collStack[1].value;
        if (type === "artist" || type === "albumartist") {
          const albums = await get("/list", { type: "album", [type]: value });
          if (token !== collToken) return;
          const context = type === "artist" ? `Artist: ${value}` : `Album artist: ${value}`;
          setRows(
            collList,
            collStore,
            albums.map((album) => valueData("album", album, context, true))
          );
        } else {
          const hits = await get("/search", { [type]: value });
          if (token !== collToken) return;
          setRows(collList, collStore, hits.map(hitData));
        }
      } else {
        const parentValue = collStack[1].value;
        const album = collStack[2].value;
        const hits = await get("/search", { [type]: parentValue, album });
        if (token !== collToken) return;
        setRows(collList, collStore, hits.map(hitData));
      }
    } catch (err) {
      if (token !== collToken) return;
      setMessage(collList, collStore, err.message, "error");
    }
  }

  function selectCollType(key) {
    activeMode = "collections";
    collStack = [{ key, label: labelFor(key) }];
    showMode();
    loadCollection();
  }

  function selectCollValue(data) {
    collStack.push({ key: data.targetKey, value: data.title, label: data.title });
    loadCollection();
  }

  // Jump from a search hit straight to the collection that shows the item in
  // full: an artist lists their albums; an album lists its songs.
  function drillIntoSearchHit(nav) {
    if (nav.type === "artist") {
      collStack = [
        { key: nav.key, label: labelFor(nav.key) },
        { key: nav.key, value: nav.name, label: nav.name },
      ];
    } else if (nav.albumartist) {
      collStack = [
        { key: "albumartist", label: labelFor("albumartist") },
        { key: "albumartist", value: nav.albumartist, label: nav.albumartist },
        { key: "album", value: nav.album, label: nav.album },
      ];
    } else {
      collStack = [
        { key: "album", label: labelFor("album") },
        { key: "album", value: nav.album, label: nav.album },
      ];
    }
    activeMode = "collections";
    showMode();
    loadCollection();
  }

  function onSearchInput() {
    const query = searchInput.value.trim();
    clearTimeout(searchTimer);
    searchController?.abort();

    if (!query) {
      showMode();
      return;
    }

    searchPane.hidden = false;
    tabs.hidden = true;
    browsePane.hidden = true;
    collPane.hidden = true;
    searchCrumb.textContent = `Search: ${query}`;
    setMessage(searchList, searchStore, "Searching…");

    searchTimer = setTimeout(async () => {
      const controller = new AbortController();
      searchController = controller;
      try {
        const hits = await get("/search", { q: query }, controller.signal);
        if (controller !== searchController) return;
        setRows(searchList, searchStore, hits.map(hitData));
      } catch (err) {
        if (err.name === "AbortError") return;
        setMessage(searchList, searchStore, err.message, "error");
      }
    }, 250);
  }

  bindList(browseList, browseStore, (data) => navigateBrowse(data.path));
  bindList(collList, collStore, selectCollValue);
  bindList(searchList, searchStore, (data) => drillIntoSearchHit(data.nav));

  container.querySelectorAll("[data-mode]").forEach((button) => {
    button.addEventListener("click", () => {
      activeMode = button.dataset.mode;
      showMode();
      if (activeMode === "collections" && !collStack.length) loadCollection();
    });
  });

  container.querySelectorAll("[data-coll-type]").forEach((button) => {
    button.addEventListener("click", () => selectCollType(button.dataset.collType));
  });

  searchInput.addEventListener("input", onSearchInput);

  showMode();
  loadBrowse();

  function refresh() {
    if (!searchPane.hidden) onSearchInput();
    else if (activeMode === "browse") loadBrowse();
    else loadCollection();
  }

  return {
    update() {},
    progress() {},
    refresh,
  };
}
