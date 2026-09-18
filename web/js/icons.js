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
  search:
    '<path d="M15.5 14h-.79l-.28-.27a6.5 6.5 0 1 0-.7.7l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0A4.5 4.5 0 1 1 14 9.5 4.5 4.5 0 0 1 9.5 14z" />',
  folder: '<path d="M10 4H4v16h16V8h-8l-2-4z" />',
  file: '<path d="M6 2h9l5 5v15H6V2zm7 1.5V8h4.5L13 3.5z" />',
  plus: '<path d="M11 5h2v6h6v2h-6v6h-2v-6H5v-2h6V5z" />',
  trash:
    '<path d="M9 3h6l1 2h4v2H4V5h4l1-2zm-3 6h12l-1 12H7L6 9z" />',
  shuffle:
    '<path d="M17 3h4v4h-2V6.4l-4.3 4.3-1.4-1.4L17.6 5H17V3zM3 5h4.6l2.5 2.5-1.4 1.4L6.3 7H3V5zm10.2 5.1 1.4-1.4 1.9 1.9L14.6 12l1.4 1.4-1.9 1.9-1.4-1.4-2.5 2.5H3v-2h4.3l2.5-2.5 1.4 1.4z" />',
};

export function icon(name, size = 18) {
  const body = icons[name] || "";
  return `<svg viewBox="0 0 24 24" width="${size}" height="${size}" aria-hidden="true">${body}</svg>`;
}
