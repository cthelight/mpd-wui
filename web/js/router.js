// Hash-based routing: the location hash is the single source of truth for
// navigation, so the browser back/forward buttons undo (and redo) every
// navigation step, and any view can be reloaded or shared as a URL. The hash
// (unlike path routes) also works when the frontend is served by any plain
// static server, with no SPA fallback required.
//
// #nowplaying                            Now Playing
// #queue                                 Queue
// #library                               Library, browse at the Music root
// #library/browse/<segment>/…            Library browse, path = segments joined
// #library/collections                   Collections, type picker
// #library/collections/<type>/<value>/…  Collection drill (≤ 3 frames)
// #library/search?q=<query>              Search results

export const COLLECTION_TYPES = [
  { key: "artist", label: "Artists" },
  { key: "albumartist", label: "Album Artists" },
  { key: "album", label: "Albums" },
  { key: "genre", label: "Genres" },
  { key: "date", label: "Years" },
];

export function labelFor(key) {
  return COLLECTION_TYPES.find((item) => item.key === key)?.label ?? key;
}

function decode(segment) {
  try {
    return decodeURIComponent(segment);
  } catch {
    // A hand-edited URL may carry a stray "%"; keep the raw segment rather
    // than throwing out of the popstate handler.
    return segment;
  }
}

// Rebuild a collection drill from its URL form. The frames are
// [type, value?, album?]; the value frame repeats the type key and the third
// frame is always an album, so the keys are derivable.
function collFrom(type, value1, value2) {
  if (!COLLECTION_TYPES.some((item) => item.key === type)) return [];
  const coll = [{ key: type, label: labelFor(type) }];
  if (value1 != null) coll.push({ key: type, value: value1, label: value1 });
  if (value2 != null) coll.push({ key: "album", value: value2, label: value2 });
  return coll;
}

export function parseRoute(hash) {
  let raw = (hash || "").replace(/^#/, "");
  let query = "";
  const question = raw.indexOf("?");
  if (question !== -1) {
    query = raw.slice(question + 1);
    raw = raw.slice(0, question);
  }
  const segments = raw.split("/").filter((segment) => segment !== "");
  switch (segments[0]) {
    case undefined:
    case "nowplaying":
      return { view: "nowplaying" };
    case "queue":
      return { view: "queue" };
    case "library": {
      const [mode = "browse", ...rest] = segments.slice(1);
      if (mode === "browse")
        return {
          view: "library",
          mode: "browse",
          browsePath: rest.map(decode).join("/"),
        };
      if (mode === "collections")
        return {
          view: "library",
          mode: "collections",
          coll: collFrom(
            rest[0],
            rest[1] != null ? decode(rest[1]) : null,
            rest[2] != null ? decode(rest[2]) : null
          ),
        };
      if (mode === "search") {
        const text = new URLSearchParams(query).get("q") || "";
        // An empty search is not a place: fall back to the browse root.
        return text
          ? { view: "library", mode: "search", query: text }
          : { view: "library", mode: "browse", browsePath: "" };
      }
      return { view: "nowplaying" };
    }
    default:
      // Unknown hash (hand-edited): land somewhere sensible.
      return { view: "nowplaying" };
  }
}

export function routeToHash(route) {
  if (route.view === "nowplaying") return "#nowplaying";
  if (route.view === "queue") return "#queue";
  if (route.mode === "search") return `#library/search?q=${encodeURIComponent(route.query)}`;
  if (route.mode === "collections") {
    const [type, value1, value2] = route.coll || [];
    let hash = "#library/collections";
    if (type) hash += `/${type.key}`;
    if (value1) hash += `/${encodeURIComponent(value1.value)}`;
    if (value2) hash += `/${encodeURIComponent(value2.value)}`;
    return hash;
  }
  const segments = (route.browsePath || "")
    .split("/")
    .filter((segment) => segment !== "")
    .map(encodeURIComponent);
  return segments.length ? `#library/browse/${segments.join("/")}` : "#library";
}
