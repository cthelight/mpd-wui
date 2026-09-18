// Thin client for the /api surface.
import { config } from "./config.js";

async function errorFromResponse(res) {
  let detail = res.statusText;
  try {
    const data = await res.json();
    if (data && typeof data.error === "string") detail = data.error;
  } catch {
    // Non-JSON error body; keep the status text.
  }
  return new Error(`${res.status} ${detail}`.trim());
}

export async function get(path, params, signal) {
  const url = new URL(config.apiBase + path, location.origin);
  for (const [k, v] of Object.entries(params || {})) {
    if (v !== undefined && v !== null && v !== "") url.searchParams.set(k, v);
  }
  const res = await fetch(url, signal ? { signal } : undefined);
  if (!res.ok) throw await errorFromResponse(res);
  return res.json();
}

export async function post(path, body) {
  const res = await fetch(config.apiBase + path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body || {}),
  });
  if (!res.ok) throw await errorFromResponse(res);
  if (res.status === 204 || res.status === 205) return {};
  return res.json().catch(() => ({}));
}

export function artUrl(file) {
  if (!file) return null;
  return `${config.apiBase}/albumart?uri=${encodeURIComponent(file)}`;
}
