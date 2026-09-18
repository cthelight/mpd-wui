// Queue view: paged rows, drag-to-reorder, click-to-play, remove, clear, shuffle.
export function mountQueue(container) {
  container.innerHTML = `
    <section class="placeholder">
      <h1>Queue</h1>
      <p>Queue management lands in the next frontend step.</p>
    </section>
  `;
  return {
    update() {},
    progress() {},
  };
}
