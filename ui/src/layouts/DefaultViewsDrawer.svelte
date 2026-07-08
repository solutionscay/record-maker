<script lang="ts">
  // Default-layout "view as" editor (#149/#151). A table's default layout is the
  // Form/List/Table trio (#57); this drawer edits WHICH of the three are enabled,
  // one iOS-style switch per view. Toggles commit immediately via setLayoutEnabled
  // (no draft/save), like the row chips it replaces, and the parent patches its
  // `layouts` so the group prop re-derives. Self-contained drawer chrome for the
  // same cross-entry-CSS reason NewLayoutDrawer.svelte spells out.
  import Icon from '../lib/Icon.svelte';
  import { setLayoutEnabled, type LayoutManagerView } from './persist';

  let {
    tableName,
    views,
    onclose,
    onupdated,
  }: {
    tableName: string;
    views: LayoutManagerView[];
    onclose: () => void;
    onupdated: (updated: LayoutManagerView) => void;
  } = $props();

  let error = $state('');
  let busyId = $state<number | null>(null);

  // At least one view per table must stay on (server-guarded). Mirror that here so
  // the last enabled switch is disabled with a hint rather than bouncing back.
  const enabledCount = $derived(views.filter((v) => v.enabled).length);

  function viewLabel(view: string): string {
    if (view === 'form') return 'Form';
    if (view === 'list') return 'List';
    return 'Table';
  }
  function viewHint(view: string): string {
    if (view === 'form') return 'One record at a time';
    if (view === 'list') return 'Records stacked as rows';
    return 'A spreadsheet-style grid';
  }

  async function toggle(v: LayoutManagerView) {
    if (busyId !== null) return;
    if (v.enabled && enabledCount <= 1) return; // keep at least one on
    error = '';
    busyId = v.id;
    try {
      const updated = await setLayoutEnabled(v.id, !v.enabled);
      onupdated(updated);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busyId = null;
    }
  }

  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onclose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });
</script>

<aside class="nld">
  <header class="nld-head">
    <span class="nld-title">Edit views · {tableName}</span>
    <button type="button" class="sc-btn sc-btn--icon sc-btn--ghost" title="Close" onclick={onclose}>
      <Icon name="minus" />
    </button>
  </header>

  <div class="nld-body">
    <p class="sc-hint nld-intro">Choose which views this table's default layout offers. At least one stays on.</p>
    <ul class="dv-list">
      {#each views as v (v.id)}
        {@const lastOn = v.enabled && enabledCount <= 1}
        <li class="dv-item">
          <span class="dv-info">
            <span class="dv-name">{viewLabel(v.view)}</span>
            <span class="dv-sub">{viewHint(v.view)}</span>
          </span>
          <label class="dv-toggle" class:disabled={lastOn} title={lastOn ? 'At least one view must stay on' : ''}>
            <input
              type="checkbox"
              checked={v.enabled}
              disabled={busyId !== null || lastOn}
              onchange={() => toggle(v)}
            />
            <span class="dv-track"><span class="dv-knob"></span></span>
          </label>
        </li>
      {/each}
    </ul>

    {#if error}
      <p class="sc-hint nld-error">{error}</p>
    {/if}
  </div>

  <footer class="nld-foot">
    <span class="nld-spacer"></span>
    <button type="button" class="sc-btn sc-btn--primary" onclick={onclose}>Done</button>
  </footer>
</aside>

<style>
  /* Drawer shell — identical recipe to NewLayoutDrawer.svelte (kept a local copy
     for the cross-entry CSS constraint documented there). */
  .nld {
    position: absolute;
    top: 0;
    right: 0;
    z-index: 20;
    max-width: 100%;
    width: 360px;
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-left: 0.5px solid var(--rm-border);
    background: var(--rm-inspector-bg);
    box-shadow: -12px 0 32px rgba(0, 0, 0, 0.14);
    animation: nld-slide 0.16s ease-out;
  }
  @keyframes nld-slide {
    from {
      transform: translateX(14px);
      opacity: 0.4;
    }
    to {
      transform: translateX(0);
      opacity: 1;
    }
  }
  .nld-head,
  .nld-foot {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 18px;
  }
  .nld-head {
    justify-content: space-between;
    padding-right: 12px;
    border-bottom: 0.5px solid var(--rm-border);
  }
  .nld-foot {
    border-top: 0.5px solid var(--rm-border);
  }
  .nld-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--rm-text);
  }
  .nld-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
    padding: 18px;
  }
  .nld-intro {
    margin: 0 0 14px;
  }
  .nld-spacer {
    flex: 1 1 auto;
  }
  .nld-error {
    margin: 14px 0 0;
    color: var(--rm-danger);
  }

  /* View switches. */
  .dv-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .dv-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 11px 13px;
    border: 0.5px solid var(--rm-border);
    border-radius: var(--rm-radius-lg);
    background: var(--rm-card-bg);
    box-shadow: var(--rm-elev-1);
  }
  .dv-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .dv-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--rm-text);
  }
  .dv-sub {
    font-size: 11px;
    color: var(--rm-text-dim);
  }
  /* iOS-style toggle — same geometry as the inspector's .toggle. */
  .dv-toggle {
    position: relative;
    display: inline-flex;
    flex: none;
    cursor: pointer;
  }
  .dv-toggle.disabled {
    cursor: not-allowed;
  }
  .dv-toggle input {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
  }
  .dv-track {
    width: 36px;
    height: 21px;
    border-radius: 21px;
    background: var(--rm-segment-track);
    transition: background 0.15s ease;
  }
  .dv-knob {
    position: absolute;
    width: 17px;
    height: 17px;
    border-radius: 50%;
    background: #fff;
    top: 2px;
    left: 2px;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
    transition: left 0.15s ease;
  }
  .dv-toggle input:checked + .dv-track {
    background: var(--rm-accent);
  }
  .dv-toggle input:checked + .dv-track .dv-knob {
    left: 17px;
  }
  .dv-toggle.disabled .dv-track {
    opacity: 0.6;
  }
</style>
