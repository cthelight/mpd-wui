// Runtime configuration for the mpd-wui frontend.
// All endpoints are same-origin; no external hosts are ever contacted.
export const config = {
  apiBase: "/api",
  wsUrl: (proto, host) =>
    `${proto === "https:" ? "wss" : "ws"}://${host}/ws`,
};
