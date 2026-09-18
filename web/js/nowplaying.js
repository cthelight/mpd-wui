// Now Playing view and the persistent mini-player.
import { artUrl } from "./api.js";
import { icon } from "./icons.js";

export function formatTime(value) {
  const total = Math.max(0, Math.floor(Number(value) || 0));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

function displayTitle(song) {
  if (!song) return "Nothing playing";
  if (song.title) return song.title;
  return song.file.split("/").pop() || song.file || "Unknown track";
}

function displayArtist(song) {
  if (!song) return "";
  return song.artist || song.albumartist || "";
}

function displayAlbum(song) {
  if (!song) return "";
  return song.album || song.albumartist || "";
}

function setArt(container, song) {
  container.innerHTML = "";
  const url = artUrl(song?.file);
  if (!url) {
    container.classList.add("empty");
    container.innerHTML = icon("music", 42);
    return;
  }
  const img = document.createElement("img");
  img.alt = "";
  img.loading = "lazy";
  img.src = url;
  img.addEventListener("load", () => container.classList.remove("empty"));
  img.addEventListener("error", () => {
    container.classList.add("empty");
    container.innerHTML = icon("music", 42);
  });
  container.appendChild(img);
}

function setPlayButton(button, state) {
  const playing = state === "play";
  button.innerHTML = icon(playing ? "pause" : "play", 22);
  button.title = playing ? "Pause" : "Play";
  button.setAttribute("aria-label", playing ? "Pause" : "Play");
}

function updateProgress(container, elapsed, duration) {
  const cur = container.querySelector("[data-cur]");
  const total = container.querySelector("[data-total]");
  const seek = container.querySelector("[data-seek]");
  if (cur) cur.textContent = formatTime(elapsed);
  if (total) total.textContent = formatTime(duration);
  if (seek) {
    seek.disabled = !duration;
    if (duration > 0) {
      seek.value = String(Math.round((elapsed / duration) * 1000));
    } else {
      seek.value = "0";
    }
  }
}

function bindSeek(container, actions) {
  const seek = container.querySelector("[data-seek]");
  if (!seek) return;
  seek.addEventListener("input", () => {
    const duration = Number(seek.dataset.duration || 0);
    const elapsed = (Number(seek.value) / 1000) * duration;
    updateProgress(container, elapsed, duration);
  });
  seek.addEventListener("change", () => {
    const duration = Number(seek.dataset.duration || 0);
    if (!duration) return;
    const time = (Number(seek.value) / 1000) * duration;
    actions.seek(time);
  });
}

function bindVolume(container, actions) {
  const volume = container.querySelector("[data-volume]");
  if (!volume) return;
  volume.addEventListener("change", () => {
    actions.volume(Number(volume.value));
  });
}

function bindModes(container, actions) {
  container.querySelectorAll("[data-opt]").forEach((button) => {
    button.addEventListener("click", () => {
      const key = button.dataset.opt;
      actions.toggleOption(key);
    });
  });
}

function bindTransport(container, actions) {
  container.querySelectorAll("[data-act]").forEach((button) => {
    button.addEventListener("click", () => {
      const act = button.dataset.act;
      if (act === "playpause") actions.playPause();
      if (act === "next") actions.next();
      if (act === "previous") actions.previous();
      if (act === "stop") actions.stop();
    });
  });
}

function makeUpdate(container) {
  return function update(snapshot) {
    const status = snapshot.status;
    const song = snapshot.song;

    setArt(container.querySelector("[data-art]"), song);
    const title = container.querySelector("[data-title]");
    const artist = container.querySelector("[data-artist]");
    const album = container.querySelector("[data-album]");
    if (title) title.textContent = displayTitle(song);
    if (artist) artist.textContent = displayArtist(song);
    if (album) album.textContent = displayAlbum(song);

    const playButton = container.querySelector("[data-act=playpause]");
    if (playButton) setPlayButton(playButton, status.state);

    const volume = container.querySelector("[data-volume]");
    if (volume) volume.value = String(status.volume || 0);

    container.querySelectorAll("[data-opt]").forEach((button) => {
      const on = Boolean(status[button.dataset.opt]);
      button.classList.toggle("on", on);
      button.setAttribute("aria-pressed", on ? "true" : "false");
    });

    updateProgress(container, status.elapsed || 0, status.time || 0);
    const seek = container.querySelector("[data-seek]");
    if (seek) seek.dataset.duration = String(status.time || 0);
  };
}

function makeProgress(container) {
  return function progress(elapsed) {
    const seek = container.querySelector("[data-seek]");
    const duration = Number(seek?.dataset.duration || 0);
    updateProgress(container, elapsed, duration);
  };
}

export function mountNowPlaying(container, actions) {
  container.innerHTML = `
    <section class="np">
      <div class="np-art" data-art></div>
      <div class="np-meta">
        <h1 class="np-title" data-title>Nothing playing</h1>
        <p class="np-artist" data-artist></p>
        <p class="np-album" data-album></p>
      </div>
      <div class="np-progress">
        <span class="time" data-cur>0:00</span>
        <input type="range" min="0" max="1000" value="0" data-seek aria-label="Seek" />
        <span class="time" data-total>0:00</span>
      </div>
      <div class="np-controls">
        <button class="ctl" data-act="previous" title="Previous">${icon("previous", 22)}</button>
        <button class="ctl primary" data-act="playpause" title="Play">${icon("play", 24)}</button>
        <button class="ctl" data-act="next" title="Next">${icon("next", 22)}</button>
        <button class="ctl" data-act="stop" title="Stop">${icon("stop", 20)}</button>
      </div>
      <div class="np-extra">
        <label class="volume">
          ${icon("volume", 18)}
          <input type="range" min="0" max="100" value="0" data-volume aria-label="Volume" />
        </label>
        <div class="modes">
          <button class="mode" data-opt="random">Random</button>
          <button class="mode" data-opt="repeat">Repeat</button>
          <button class="mode" data-opt="single">Single</button>
          <button class="mode" data-opt="consume">Consume</button>
        </div>
      </div>
    </section>
  `;
  bindTransport(container, actions);
  bindVolume(container, actions);
  bindModes(container, actions);
  bindSeek(container, actions);
  return {
    update: makeUpdate(container),
    progress: makeProgress(container),
  };
}

export function renderMiniPlayer(container, actions) {
  container.innerHTML = `
    <div class="mini">
      <div class="mini-art" data-art></div>
      <div class="mini-meta">
        <span class="mini-title" data-title>Nothing playing</span>
        <span class="mini-artist" data-artist></span>
      </div>
      <div class="mini-progress">
        <span class="time" data-cur>0:00</span>
        <input type="range" min="0" max="1000" value="0" data-seek aria-label="Seek" />
        <span class="time" data-total>0:00</span>
      </div>
      <div class="mini-controls">
        <button class="ctl small" data-act="previous" title="Previous">${icon("previous", 18)}</button>
        <button class="ctl small primary" data-act="playpause" title="Play">${icon("play", 18)}</button>
        <button class="ctl small" data-act="next" title="Next">${icon("next", 18)}</button>
      </div>
    </div>
  `;
  bindTransport(container, actions);
  bindSeek(container, actions);
  return {
    update: makeUpdate(container),
    progress: makeProgress(container),
  };
}
