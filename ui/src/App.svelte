<script lang="ts">
  // Placeholder island for Layout Mode. The real design canvas (drag/resize via
  // moveable + selecto, plus the document/history store) lands in #15 / #46 —
  // this only proves the Svelte 5 (runes) + Vite toolchain mounts inside the
  // existing shell chrome and can read the layout id handed in by the page.
  let { layoutId = '' }: { layoutId?: string } = $props();

  // A trivial rune so the runes compiler path is exercised by the scaffold.
  let mounted = $state(false);
  $effect(() => {
    mounted = true;
  });
</script>

<section class="layout-editor">
  <h2>Layout Mode — layout {layoutId}</h2>
  <p class="status">
    Svelte&nbsp;5 island mounted{mounted ? '.' : '…'} The design canvas arrives in
    a later issue (#15 / #46).
  </p>
</section>

<style>
  .layout-editor {
    border: 1px dashed #cbd5e1;
    border-radius: 0.5rem;
    padding: 1rem 1.25rem;
    background: #f7f9fc;
    color: #1b1b1f;
  }
  .layout-editor h2 {
    margin: 0 0 0.4rem;
    font-size: 1.05rem;
  }
  .layout-editor .status {
    margin: 0;
    color: #555;
  }
</style>
