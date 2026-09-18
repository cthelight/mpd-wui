// Runtime configuration for the mpd-wui frontend.
// All endpoints are same-origin; no external hosts are ever contacted.
// The display name (wordmark + tab title) is injected by the server into the
// `app-title` meta tag; it is configurable and defaults to "MPD: <host>".
const appTitle =
  document.querySelector('meta[name="app-title"]')?.content || "mpd-wui";

export const config = {
  apiBase: "/api",
  appTitle,
  wsUrl: (proto, host) =>
    `${proto === "https:" ? "wss" : "ws"}://${host}/ws`,
};
