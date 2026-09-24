// Inline SVG icon set (no external assets). Returns SVG markup strings.
const icons = {
  play: '<path d="M8 5v14l11-7z" />',
  pause: '<path d="M6 5h4v14H6z" /><path d="M14 5h4v14h-4z" />',
  stop: '<path d="M6 6h12v12H6z" />',
  next: '<path d="M6 5l8 7-8 7z" /><path d="M16 5h2v14h-2z" />',
  previous: '<path d="M18 5l-8 7 8 7z" /><path d="M6 5h2v14H6z" />',
  volume:
    '<path d="M4 9v6h4l5 5V4L8 9H4z" /><path d="M16.5 12a3.5 3.5 0 0 0-2-3.15v6.3a3.5 3.5 0 0 0 2-3.15z" />',
  music:
    '<path d="M12 3v10.55A4 4 0 1 0 14 17V7h4V3h-6z" />',
  artist:
    '<path d="M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z" />',
  album:
    '<path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 14.5c-2.49 0-4.5-2.01-4.5-4.5S9.51 7.5 12 7.5s4.5 2.01 4.5 4.5-2.01 4.5-4.5 4.5zm0-5c-.28 0-.5.22-.5.5s.22.5.5.5.5-.22.5-.5-.22-.5-.5-.5z" />',
  search:
    '<path d="M15.5 14h-.79l-.28-.27a6.5 6.5 0 1 0-.7.7l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0A4.5 4.5 0 1 1 14 9.5 4.5 4.5 0 0 1 9.5 14z" />',
  folder: '<path d="M10 4H4v16h16V8h-8l-2-4z" />',
  file: '<path d="M6 2h9l5 5v15H6V2zm7 1.5V8h4.5L13 3.5z" />',
  plus: '<path d="M11 5h2v6h6v2h-6v6h-2v-6H5v-2h6V5z" />',
  trash:
    '<path d="M9 3h6l1 2h4v2H4V5h4l1-2zm-3 6h12l-1 12H7L6 9z" />',
  shuffle:
    '<path d="M17 3h4v4h-2V6.4l-4.3 4.3-1.4-1.4L17.6 5H17V3zM3 5h4.6l2.5 2.5-1.4 1.4L6.3 7H3V5zm10.2 5.1 1.4-1.4 1.9 1.9L14.6 12l1.4 1.4-1.9 1.9-1.4-1.4-2.5 2.5H3v-2h4.3l2.5-2.5 1.4 1.4z" />',
  grip:
    '<circle cx="9" cy="5.5" r="1.6" /><circle cx="15" cy="5.5" r="1.6" /><circle cx="9" cy="12" r="1.6" /><circle cx="15" cy="12" r="1.6" /><circle cx="9" cy="18.5" r="1.6" /><circle cx="15" cy="18.5" r="1.6" />',
  // Queue list with the highlighted insertion row near the top (after the
  // current song) — "enqueue next".
  enqueueNext:
    '<rect x="3" y="2.5" width="3" height="2.5" rx="1.25" /><rect x="8" y="2.5" width="9" height="2.5" rx="1.25" /><rect x="3" y="8" width="3.5" height="5.5" rx="1.75" /><rect x="9" y="8" width="11.5" height="5.5" rx="1.75" /><rect x="3" y="16.5" width="3" height="2.5" rx="1.25" /><rect x="8" y="16.5" width="9" height="2.5" rx="1.25" />',
  // Queue list with the highlighted insertion row at the bottom — "enqueue at end".
  enqueueEnd:
    '<rect x="3" y="2.5" width="3" height="2.5" rx="1.25" /><rect x="8" y="2.5" width="9" height="2.5" rx="1.25" /><rect x="3" y="8" width="3" height="2.5" rx="1.25" /><rect x="8" y="8" width="9" height="2.5" rx="1.25" /><rect x="3" y="16.5" width="3.5" height="5.5" rx="1.75" /><rect x="9" y="16.5" width="11.5" height="5.5" rx="1.75" />',
  up: '<path d="M12 9 6 15l1.4 1.4L12 11.8l4.6 4.6L18 15z" />',
  down: '<path d="M12 15 6 9l1.4-1.4L12 12.2l4.6-4.6L18 9z" />',
  refresh:
    '<path d="M17.65 6.35A7.95 7.95 0 0 0 12 4a8 8 0 1 0 8 8h-2a6 6 0 1 1-1.76-4.24L13 11h7V4l-2.35 2.35z" />',
  database:
    '<ellipse cx="12" cy="5" rx="9" ry="3" /><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5c0 1.66-4 3-9 3S3 6.66 3 5z" />',
  settings:
    '<path d="M19.14 12.94a7.07 7.07 0 0 0 0-1.88l2.03-1.58a.5.5 0 0 0 .12-.64l-1.92-3.32a.5.5 0 0 0-.61-.22l-2.39.96a7.07 7.07 0 0 0-1.63-.94l-.36-2.54A.5.5 0 0 0 12.89 2h-3.78a.5.5 0 0 0-.49.42l-.36 2.54c-.59.24-1.13.56-1.63.94l-2.39-.96a.5.5 0 0 0-.61.22L1.71 8.44a.5.5 0 0 0 .12.64l2.03 1.58a7.07 7.07 0 0 0 0 1.88l-2.03 1.58a.5.5 0 0 0-.12.64l1.92 3.32c.13.23.4.31.61.22l2.39-.96c.5.38 1.04.7 1.63.94l.36 2.54c.04.24.25.42.49.42h3.78c.24 0 .45-.18.49-.42l.36-2.54c.59-.24 1.13-.56 1.63-.94l2.39.96c.21.09.48 0 .61-.22l1.92-3.32a.5.5 0 0 0-.12-.64l-2.03-1.58zM11 15.5a3.5 3.5 0 1 1 0-7 3.5 3.5 0 0 1 0 7z" />',
};

export function icon(name, size = 18) {
  const body = icons[name] || "";
  return `<svg viewBox="0 0 24 24" width="${size}" height="${size}" aria-hidden="true">${body}</svg>`;
}
