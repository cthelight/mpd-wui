// Settings view: server info, database maintenance, local cache, audio outputs
// and partitions.
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
        <section class="settings-card" aria-labelledby="settings-outputs">
          <h2 id="settings-outputs">${icon("volume", 18)} Outputs</h2>
          <div data-outputs>Loading…</div>
        </section>
        <section class="settings-card" aria-labelledby="settings-partitions">
          <h2 id="settings-partitions">${icon("music", 18)} Partitions</h2>
          <div data-partitions>Loading…</div>
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
  const outputsEl = container.querySelector("[data-outputs]");
  const partitionsEl = container.querySelector("[data-partitions]");
  const buttons = {
    update: container.querySelector('[data-act="update"]'),
    rescan: container.querySelector('[data-act="rescan"]'),
    clearCache: container.querySelector('[data-act="clear-cache"]'),
  };

  let capabilities = null;
  let updating = false;
  let pending = 0;
  let pollTimer = null;

  // System state (outputs + partitions) is scoped to the client's current
  // partition, so we only re-fetch when the partition changes or a load
  // failed. Output ids are not stable across MPD runs, so we never cache them
  // across a reload — every render comes from a fresh `outputs` response.
  let outputs = [];
  let partitions = { current: "", partitions: [] };
  let loadedPartition = null;
  let systemLoaded = false;
  let outputsLoading = false;
  let partitionsLoading = false;

  function rescanSupported() {
    return capabilities?.commands?.includes("rescan") ?? false;
  }

  // Optimistic: if MPD reports no command list we assume support rather than
  // hiding features on older servers that omit `commands` entirely.
  function hasCommand(name) {
    const commands = capabilities?.commands;
    return commands?.length ? commands.includes(name) : true;
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
    // Capability gating affects the output toggles; re-render if we already
    // have output data.
    if (systemLoaded) renderOutputs();
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

  async function loadOutputs() {
    if (outputsLoading) return;
    outputsLoading = true;
    outputsEl.innerHTML = `<div class="empty">Loading…</div>`;
    try {
      outputs = (await get("/outputs")) || [];
      systemLoaded = true;
      renderOutputs();
    } catch (err) {
      outputsEl.innerHTML = `<div class="error">${escapeHtml(err.message)}</div>`;
    } finally {
      outputsLoading = false;
    }
  }

  function renderOutputs() {
    const canControl = hasCommand("enableoutput");
    if (!outputs.length) {
      outputsEl.innerHTML = `<div class="empty">No audio outputs configured.</div>`;
      return;
    }
    const rows = outputs
      .map((output) => `
        <div class="output-row">
          <span class="output-name" title="${escapeHtml(output.plugin ? `${output.name} (${output.plugin})` : output.name)}">
            ${escapeHtml(output.name)}
          </span>
          ${output.plugin ? `<span class="output-plugin">${escapeHtml(output.plugin)}</span>` : ""}
          <input
            class="toggle"
            type="checkbox"
            role="switch"
            data-output-id="${output.id}"
            aria-label="${output.enabled ? "Disable" : "Enable"} ${escapeHtml(output.name)}"
            ${output.enabled ? "checked" : ""}
            ${canControl ? "" : "disabled"}
          />
        </div>
      `)
      .join("");
    outputsEl.innerHTML = `
      <div class="output-list">${rows}</div>
      ${canControl ? "" : '<p class="card-hint">Switching outputs requires admin permission.</p>'}
    `;
  }

  async function loadPartitions() {
    if (partitionsLoading) return;
    partitionsLoading = true;
    partitionsEl.innerHTML = `<div class="empty">Loading…</div>`;
    try {
      partitions = await get("/partitions");
      loadedPartition = partitions.current;
      systemLoaded = true;
      renderPartitions();
    } catch (err) {
      partitionsEl.innerHTML = `<div class="error">${escapeHtml(err.message)}</div>`;
    } finally {
      partitionsLoading = false;
    }
  }

  function renderPartitions() {
    const list = partitions.partitions || [];
    if (list.length <= 1) {
      partitionsEl.innerHTML = `<div class="empty">This server runs a single partition; there is nothing to switch.</div>`;
      return;
    }
    const rows = list
      .map((name) => {
        const active = name === partitions.current;
        return `
        <button
          type="button"
          class="partition-row"
          data-partition-name="${escapeHtml(name)}"
          ${active ? 'aria-current="true" disabled' : ""}
        >
          <span class="partition-name">${escapeHtml(name)}</span>
          ${active ? '<span class="partition-badge">current</span>' : ""}
        </button>
      `;
      })
      .join("");
    partitionsEl.innerHTML = `
      <div class="partition-list">${rows}</div>
      <p class="card-hint">Switching moves the whole UI — queue, player and outputs — to that partition.</p>
    `;
  }

  function refresh() {
    loadCapabilities();
    loadStats();
    loadOutputs();
    loadPartitions();
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
    // A partition change — from this client or another — invalidates both
    // system cards, whose lists are scoped to the client's current partition.
    const partition = snapshot?.status?.partition;
    if (systemLoaded && partition && partition !== loadedPartition) {
      loadOutputs();
      loadPartitions();
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

  // Output toggles: delegate to the (re-rendered) list container.
  outputsEl.addEventListener("change", async (event) => {
    const checkbox = event.target.closest(".toggle");
    if (!checkbox) return;
    const id = Number(checkbox.dataset.outputId);
    const enabled = checkbox.checked;
    const output = outputs.find((item) => item.id === id);
    checkbox.disabled = true;
    try {
      await post("/outputs", { id, enabled });
      if (output) output.enabled = enabled;
      toast(`${enabled ? "Enabled" : "Disabled"} output "${output?.name || id}"`);
      renderOutputs();
    } catch (err) {
      toast(err?.message || String(err));
      checkbox.checked = !enabled;
      checkbox.disabled = false;
    }
  });

  // Partition switching: a successful switch changes what every other card
  // shows (stats, outputs, capabilities are all partition-scoped), so reload
  // the whole settings view.
  partitionsEl.addEventListener("click", async (event) => {
    const button = event.target.closest(".partition-row");
    if (!button || button.disabled) return;
    const name = button.dataset.partitionName;
    button.disabled = true;
    try {
      await post("/partitions", { name });
      toast(`Switched to partition "${name}"`);
      loadedPartition = null;
      systemLoaded = false;
      refresh();
    } catch (err) {
      toast(err?.message || String(err));
      button.disabled = false;
    }
  });

  refresh();

  return {
    update,
    progress,
    refresh,
  };
}
