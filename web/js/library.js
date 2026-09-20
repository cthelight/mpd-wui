import { get, post } from "./api.js";
import { icon } from "./icons.js";
import { formatTime } from "./nowplaying.js";
import { escapeHtml, toast } from "./util.js";
import { COLLECTION_TYPES, defaultLibraryRoute, labelFor, routeToHash } from "./router.js";

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
      target: { artist: hit.name },
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
      target: { album: hit.name },
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
        <button class="row-btn" data-row-act="enqueue-next" title="Enqueue next">${icon("enqueueNext", 16)}</button>
        <button class="row-btn" data-row-act="enqueue-end" title="Enqueue at end">${icon("enqueueEnd", 16)}</button>
        <button class="row-btn primary" data-row-act="play" title="Play">${icon("play", 16)}</button>
      </div>
    </li>
  `;
}

// Collections can be very large (an artist with thousands of tracks); render
// in pages so the DOM is not flooded with thousands of rows at once.
const PAGE = 200;

function appendShowMore(list, store) {
  list.querySelector(".show-more")?.remove();
  const remaining = store.rows.length - store.shown;
  if (remaining <= 0) return;
  const button = document.createElement("button");
  button.className = "show-more";
  button.type = "button";
  button.textContent = `Show ${Math.min(PAGE, remaining)} more (${remaining} left)`;
  button.addEventListener("click", () => {
    store.shown = Math.min(store.rows.length, store.shown + PAGE);
    const start = store.shown - PAGE;
    const html = store.rows
      .slice(start, store.shown)
      .map((data, i) => rowHtml(data, start + i))
      .join("");
    list.insertAdjacentHTML("beforeend", html);
    appendShowMore(list, store);
  });
  list.appendChild(button);
}

function setRows(list, store, rows) {
  store.rows = rows;
  if (!rows.length) {
    list.innerHTML = `<div class="empty">No results</div>`;
    return;
  }
  store.shown = Math.min(PAGE, rows.length);
  list.innerHTML = rows.slice(0, store.shown).map((data, i) => rowHtml(data, i)).join("");
  appendShowMore(list, store);
}

function setMessage(list, store, message, className = "empty") {
  store.rows = [];
  list.innerHTML = `<div class="${className}">${escapeHtml(message)}</div>`;
}

async function queueTarget(target, play, button, position = "end") {
  if (!target) {
    toast("Nothing to queue");
    return;
  }
  const original = button.innerHTML;
  button.disabled = true;
  try {
    await post("/queue/add", { targets: [target], play, position });
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
      if (action.dataset.rowAct === "enqueue-next")
        queueTarget(data.target, false, action, "after_current");
      if (action.dataset.rowAct === "enqueue-end") queueTarget(data.target, false, action, "end");
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

export function mountLibrary(container, navigate, initialRoute) {
  container.innerHTML = `
    <section class="library">
      <div class="library-search">
        <label class="search-box">
          ${icon("search", 18)}
          <input type="search" data-search placeholder="Search songs, artists, albums" aria-label="Search" />
        </label>
      </div>
      <nav class="library-tabs" data-tabs>
        ${COLLECTION_TYPES.map(
          (item, i) =>
            `<button class="lib-tab${i === 0 ? " active" : ""}" data-mode="${item.key}">${item.label}</button>`
        ).join("")}
        <button class="lib-tab" data-mode="browse">Files</button>
      </nav>
      <div class="library-body">
        <section class="lib-pane" data-pane="browse">
          <div class="breadcrumb" data-browse-crumb></div>
          <div class="list" data-browse-list></div>
        </section>
        <section class="lib-pane" data-pane="collections" hidden>
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

  let activeMode = "artist";
  let browsePath = "";
  let browseToken = 0;
  let collStack = [];
  let collToken = 0;

  // Each tab keeps its own state (files path / collection drill stack), so
  // switching tabs and back resumes each one exactly where it was left.
  const modeStates = {};
  function stateFor(mode) {
    return (
      modeStates[mode] ??
      (modeStates[mode] =
        mode === "browse"
          ? { browsePath: "" }
          : { coll: [{ key: mode, label: labelFor(mode) }] })
    );
  }
  let searchTimer;
  let searchController;
  // The last route this view applied; empty until the first restore so the
  // mount-time initial route always takes effect.
  let appliedHash = "";

  function showMode() {
    browsePane.hidden = activeMode !== "browse";
    collPane.hidden = activeMode === "browse";
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
    navigate({ view: "library", mode: "browse", browsePath: path });
  }

  function renderCollBreadcrumb() {
    collCrumb.innerHTML = "";
    if (!collStack.length) {
      const label = document.createElement("span");
      label.className = "crumb current";
      label.textContent = labelFor(activeMode);
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
        navigate({
          view: "library",
          mode: collStack[0].key,
          coll: collStack.slice(0, targetLength),
        });
      });
      collCrumb.append(crumb);
    });
  }

  async function loadCollection() {
    const token = ++collToken;
    renderCollBreadcrumb();
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

  function selectCollValue(data) {
    navigate({
      view: "library",
      mode: collStack[0].key,
      coll: [...collStack, { key: data.targetKey, value: data.title, label: data.title }],
    });
  }

  // Jump from a search hit straight to the collection that shows the item in
  // full: an artist lists their albums; an album lists its songs.
  function drillIntoSearchHit(nav) {
    let coll;
    if (nav.type === "artist") {
      coll = [
        { key: nav.key, label: labelFor(nav.key) },
        { key: nav.key, value: nav.name, label: nav.name },
      ];
    } else if (nav.albumartist) {
      coll = [
        { key: "albumartist", label: labelFor("albumartist") },
        { key: "albumartist", value: nav.albumartist, label: nav.albumartist },
        { key: "album", value: nav.album, label: nav.album },
      ];
    } else {
      coll = [
        { key: "album", label: labelFor("album") },
        { key: "album", value: nav.album, label: nav.album },
      ];
    }
    navigate({ view: "library", mode: coll[0].key, coll });
  }

  // The route this view is currently showing: the search query when the
  // search pane is up, otherwise the active tab's state (files path or
  // collection stack; each stays alive while searching).
  function preSearchRoute() {
    return activeMode === "browse"
      ? { view: "library", mode: "browse", browsePath }
      : { view: "library", mode: activeMode, coll: collStack };
  }

  function route() {
    const query = searchInput.value.trim();
    return query ? { view: "library", mode: "search", query } : preSearchRoute();
  }

  function showSearch() {
    browsePane.hidden = true;
    collPane.hidden = true;
    searchPane.hidden = false;
    tabs.hidden = true;
  }

  function startSearch(query) {
    clearTimeout(searchTimer);
    searchController?.abort();
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

  function onSearchInput() {
    const query = searchInput.value.trim();
    if (query) {
      // The first keystroke pushes a history entry (so back exits the search
      // in one step, to the view shown before searching); further keystrokes
      // replace that entry instead of stacking one per character.
      const alreadySearching = appliedHash.startsWith("#library/search?");
      // A no-op navigate means the entry already holds this query; re-run the
      // search directly (this is also the refresh() path).
      if (!navigate({ view: "library", mode: "search", query }, alreadySearching))
        startSearch(query);
    } else {
      navigate(preSearchRoute(), true);
    }
  }

  // Apply a route (initial load, tab resume, or back/forward). Comparing
  // hashes makes re-applying the current view a no-op, which keeps the
  // kept-alive state (scroll, "show more" pages) intact.
  function restore(route) {
    const hash = routeToHash(route);
    if (hash === appliedHash) return;
    appliedHash = hash;
    if (route.mode === "search") {
      searchInput.value = route.query;
      showSearch();
      startSearch(route.query);
      return;
    }
    searchInput.value = "";
    activeMode = route.mode;
    showMode();
    const state = stateFor(activeMode);
    if (activeMode === "browse") {
      state.browsePath = route.browsePath || "";
      browsePath = state.browsePath;
      loadBrowse();
    } else {
      state.coll = route.coll?.length ? route.coll : state.coll;
      collStack = state.coll;
      loadCollection();
    }
  }

  bindList(browseList, browseStore, (data) => navigateBrowse(data.path));
  bindList(collList, collStore, selectCollValue);
  bindList(searchList, searchStore, (data) => drillIntoSearchHit(data.nav));

  container.querySelectorAll("[data-mode]").forEach((button) => {
    button.addEventListener("click", () => {
      const mode = button.dataset.mode;
      const state = stateFor(mode);
      navigate(
        mode === "browse"
          ? { view: "library", mode: "browse", browsePath: state.browsePath }
          : { view: "library", mode, coll: state.coll }
      );
    });
  });

  searchInput.addEventListener("input", onSearchInput);

  // The app passes the route the user arrived on (a deep link, or the default
  // Artists tab on a first visit) so the initial load matches the URL.
  restore(initialRoute ?? defaultLibraryRoute());

  function refresh() {
    if (!searchPane.hidden) startSearch(searchInput.value.trim());
    else if (activeMode === "browse") loadBrowse();
    else loadCollection();
  }

  return {
    update() {},
    progress() {},
    refresh,
    restore,
    route,
  };
}
