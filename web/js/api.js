// Thin client for the /api surface. Fleshed out in the frontend commits.
import { config } from "./config.js";

export async function get(path, params) {
  const url = new URL(config.apiBase + path, location.origin);
  for (const [k, v] of Object.entries(params || {})) {
    if (v !== undefined && v !== null) url.searchParams.set(k, v);
  }
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
  return res.json();
}

export async function post(path, body) {
  const res = await fetch(config.apiBase + path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body || {}),
  });
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
  return res.json().catch(() => ({}));
}
