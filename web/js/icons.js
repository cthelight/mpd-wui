// Inline SVG icon set (no external assets). Returns SVG markup strings.
const icons = {
  play: '<svg viewBox="0 0 24 24" width="18" height="18"><path d="M8 5v14l11-7z"/></svg>',
};

export function icon(name) {
  return icons[name] || "";
}
