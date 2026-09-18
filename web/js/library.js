// Library view: debounced search, folder browse, collection drill-down, bulk add/play.
export function mountLibrary(container) {
  container.innerHTML = `
    <section class="placeholder">
      <h1>Library</h1>
      <p>Library browsing and search land in the next frontend step.</p>
    </section>
  `;
  return {
    update() {},
    progress() {},
  };
}
