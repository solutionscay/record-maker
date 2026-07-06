<script lang="ts">
  // The right-side drawer shell (#132) shared by the Field / Table / Relationship
  // drawers, which each used to re-implement the same aside + slide-in, header
  // with ghost close, scrollable body, footer, and Escape-to-close effect (and
  // had drifted: only FieldDrawer animated — the slide-in is the intended design,
  // now applied to all three). The drawers keep their own content, footer
  // buttons, and validation; this shell owns only the frame.
  import type { Snippet } from 'svelte';
  import Icon from '../lib/Icon.svelte';

  let {
    title,
    onclose,
    width = 360,
    lead,
    children,
    footer,
  }: {
    title: string;
    /** Close without saving — also wired to Escape and the header ghost button. */
    onclose: () => void;
    /** Drawer width in px (the relationship drawer is a touch wider). */
    width?: number;
    /** Optional non-scrolling strip between the header and the body (the field
     * drawer's name/kind chip). */
    lead?: Snippet;
    /** Scrollable drawer body. */
    children: Snippet;
    /** Footer row — Delete/Cancel/Save live with the owning drawer. */
    footer: Snippet;
  } = $props();

  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onclose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });
</script>

<aside class="sd" style:width="{width}px">
  <header class="sd-head">
    <span class="sd-title">{title}</span>
    <button type="button" class="sc-btn sc-btn--icon sc-btn--ghost" title="Close" onclick={onclose}>
      <Icon name="minus" />
    </button>
  </header>

  {#if lead}{@render lead()}{/if}

  <div class="sd-body">{@render children()}</div>

  <footer class="sd-foot">{@render footer()}</footer>
</aside>

<style>
  .sd {
    position: absolute;
    top: 0;
    right: 0;
    z-index: 20;
    max-width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-left: 0.5px solid var(--rm-border);
    background: var(--rm-inspector-bg);
    box-shadow: -12px 0 32px rgba(0, 0, 0, 0.14);
    animation: sd-slide 0.16s ease-out;
  }
  @keyframes sd-slide {
    from {
      transform: translateX(14px);
      opacity: 0.4;
    }
    to {
      transform: translateX(0);
      opacity: 1;
    }
  }
  .sd-head,
  .sd-foot {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 18px;
  }
  .sd-head {
    justify-content: space-between;
    padding-right: 12px;
    border-bottom: 0.5px solid var(--rm-border);
  }
  .sd-foot {
    border-top: 0.5px solid var(--rm-border);
  }
  .sd-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--rm-text);
  }
  .sd-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
    padding: 18px;
  }
</style>
