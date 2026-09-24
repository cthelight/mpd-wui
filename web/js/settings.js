// Settings view: server info, database maintenance and local cache controls.
import { get, post } from "./api.js";
import { icon } from "./icons.js";
import { escapeHtml, toast } from "./util.js";

function pad(value) {
  return String(value).padStart(2, "0");
}

function formatPlaytime(value) {
  const total = Math.max(0, Math.floor(Number(value) || 0));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  if (hours > 0) return `${hours}:${pad(minutes)}:${pad(seconds)}`;
  return `${minutes}:${pad(seconds)}`;
}

function formatDate(value) {
  const seconds = Number(value) || 0;
  if (!seconds) return "Unknown";
  const date = new Date(seconds * 1000);
  return Number.isNaN(date.getTime()) ? "Unknown" : date.toLocaleString();
}

function statsHtml(stats) {
  const items = [
    { label: "Playtime", value: formatPlaytime(stats.db_playtime) },
    { label: "Songs", value: stats.songs },
    { label: "Albums", value: stats.albums },
    { label: "Artists", value: stats.artists },
    { label: "Last updated", value: formatDate(stats.db_update) },
  ];
  return items
    .map(
      (item) => `
      <div class="stat">
        <span class="stat-label">${item.label}</span>
        <span class="stat-value" title="${escapeHtml(item.value)}">${escapeHtml(item.value)}</span>
      </div>
    `
    )
    .join("");
}

export function mountSettings(container) {
  container.innerHTML = `
    <section class="settings">
      <h1>Settings</h1>
      <div class="settings-grid">
        <section class="settings-card" aria-labelledby="settings-server">
          <h2 id="settings-server">Server</h2>
          <div class="server-status" data-server-status>Loading…</div>
        </section>
        <section class="settings-card" aria-labelledby="settings-database">
          <div class="settings-card-header">
            <h2 id="settings-database">${icon("database", 18)} Database</h2>
            <span class="db-status" data-db-status></span>
          </div>
          <div class="stats-grid" data-stats>Loading…</div>
          <div class="settings-actions">
            <button type="button" data-act="update">${icon("refresh", 16)} Update</button>
            <button type="button" data-act="rescan">${icon("refresh", 16)} Rescan</button>
          </div>
        </section>
        <section class="settings-card" aria-labelledby="settings-cache">
          <h2 id="settings-cache">${icon("trash", 18)} Cache</h2>
          <p>Clear cached album art and the in-process library snapshot.</p>
          <div class="settings-actions">
            <button type="button" data-act="clear-cache">${icon("trash", 16)} Clear cache</button>
          </div>
        </section>
      </div>
    </section>
  `;

  const serverStatus = container.querySelector("[data-server-status]");
  const statsEl = container.querySelector("[data-stats]");
  const dbStatus = container.querySelector("[data-db-status]");
  const buttons = {
    update: container.querySelector('[data-act="update"]'),
    rescan: container.querySelector('[data-act="rescan"]'),
    clearCache: container.querySelector('[data-act="clear-cache"]'),
  };

  let capabilities = null;
  let updating = false;
  let pending = 0;
  let pollTimer = null;

  function rescanSupported() {
    return capabilities?.commands?.includes("rescan") ?? false;
  }

  function refreshButtons() {
    const busy = pending > 0;
    buttons.update.disabled = busy || updating;
    buttons.rescan.disabled = busy || updating || !rescanSupported();
    buttons.clearCache.disabled = busy;
    buttons.rescan.title = rescanSupported()
      ? "Rescan and re-parse metadata"
      : "Rescan is not supported by this MPD server";
    dbStatus.textContent = updating ? "Updating…" : "";
    dbStatus.classList.toggle("active", updating);
  }

  function setPending(delta) {
    pending = Math.max(0, pending + delta);
    refreshButtons();
  }

  async function loadCapabilities() {
    try {
      capabilities = await get("/capabilities");
      const version = capabilities.version || "unknown version";
      serverStatus.innerHTML = `
        <div><strong>MPD ${escapeHtml(version)}</strong></div>
        <div>Rescan: ${rescanSupported() ? "supported" : "not supported"}</div>
      `;
    } catch (err) {
      capabilities = null;
      serverStatus.innerHTML = `<div class="error">${escapeHtml(err.message)}</div>`;
    }
    refreshButtons();
  }

  async function loadStats() {
    statsEl.innerHTML = `<div class="empty">Loading…</div>`;
    try {
      const stats = await get("/database/stats");
      statsEl.innerHTML = statsHtml(stats);
    } catch (err) {
      statsEl.innerHTML = `<div class="error">${escapeHtml(err.message)}</div>`;
    }
  }

  function refresh() {
    loadCapabilities();
    loadStats();
  }

  // The WebSocket only pushes a snapshot when MPD reports a change, and a
  // database update can finish without that transition reaching us (the `idle`
  // race, or the `database` notice not being paired with a snapshot). So while
  // "updating", poll /status to guarantee the indicator clears as soon as MPD
  // stops updating, independent of the push stream.
  function update(snapshot) {
    const was = updating;
    updating = Boolean(snapshot?.status?.updating);
    if (was && !updating) {
      loadStats();
      stopPolling();
    } else if (updating) {
      startPolling();
    }
    refreshButtons();
  }

  function startPolling() {
    if (pollTimer) return;
    pollTimer = setInterval(async () => {
      try {
        update(await get("/status"));
      } catch {
        // MPD momentarily unreachable; keep polling.
      }
    }, 2500);
  }

  function stopPolling() {
    if (!pollTimer) return;
    clearInterval(pollTimer);
    pollTimer = null;
  }

  function progress() {}

  async function run(button, path, success) {
    setPending(1);
    try {
      await post(path, {});
      toast(success);
    } catch (err) {
      toast(err?.message || String(err));
    } finally {
      setPending(-1);
    }
  }

  buttons.update.addEventListener("click", () =>
    run(buttons.update, "/database/update", "Database update started")
  );
  buttons.rescan.addEventListener("click", () =>
    run(buttons.rescan, "/database/rescan", "Database rescan started")
  );
  buttons.clearCache.addEventListener("click", () =>
    run(buttons.clearCache, "/cache/clear", "Cache cleared")
  );

  refresh();

  return {
    update,
    progress,
    refresh,
  };
}