// mpd-wui app entry: mounts views, wires the WebSocket status stream.
// Fleshed out in the frontend commits (shell + Now Playing, then Queue + Library).
import "./icons.js";

document.addEventListener("DOMContentLoaded", () => {
  document.querySelector(".conn")?.classList.add("up");
});
