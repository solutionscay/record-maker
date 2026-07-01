<script lang="ts">
  // Icon wrapper for the Layout-mode islands (#72). It emits the SAME `<use>`
  // markup the server chrome uses, referencing a `<symbol>` from the inline
  // pixelarticons sprite that shell.html embeds once (crates/server/templates/
  // _icons.svg). Because the islands mount INTO that document, the symbols are
  // already present — there is no per-component SVG and no runtime fetch.
  //
  // Icons are decorative: the sprite is aria-hidden and each control keeps its
  // own title/aria-label. Sizing/colour come from the shared global `.icon` rule
  // in shell.html (`1em` box, `currentColor` fill), so icons inherit the button's
  // colour and active/disabled/hover state.
  let { name }: { name: string } = $props();
</script>

<svg class="icon" aria-hidden="true" focusable="false"><use href={`#icon-${name}`} /></svg>
