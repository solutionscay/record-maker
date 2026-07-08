<script lang="ts">
  // Custom-layout editor (#149/#151). The pencil on a custom row opens this drawer
  // to rename the layout and — the one place it lives now — delete it. Mirrors the
  // default-layout Edit-views drawer so both row kinds edit through the same
  // pencil → drawer affordance. Rename commits on blur/Done; delete confirms then
  // removes. Self-contained drawer chrome for the cross-entry-CSS reason
  // NewLayoutDrawer.svelte documents.
  import Icon from '../lib/Icon.svelte';
  import { confirmDanger } from './confirm';
  import { deleteLayout, renameLayout, type LayoutManagerView } from './persist';

  let {
    layout,
    onclose,
    onrenamed,
    ondeleted,
  }: {
    layout: LayoutManagerView;
    onclose: () => void;
    onrenamed: (updated: LayoutManagerView) => void;
    ondeleted: (id: number) => void;
  } = $props();

  let name = $state('');
  let error = $state('');
  let busy = $state(false);

  // Seed (and re-sync) the field from the layout; the drawer stays open on the
  // same layout for its lifetime, so this only fires on open and after a commit.
  $effect(() => {
    name = layout.name;
  });

  function viewLabel(view: string): string {
    if (view === 'form') return 'Form';
    if (view === 'list') return 'List';
    return 'Table';
  }

  async function commitRename() {
    if (busy) return; // a commit is already in flight (e.g. from the field's blur)
    const n = name.trim();
    if (!n || n === layout.name) {
      name = layout.name;
      return;
    }
    busy = true;
    error = '';
    try {
      const updated = await renameLayout(layout.id, n);
      onrenamed(updated);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      name = layout.name;
    } finally {
      busy = false;
    }
  }

  // The Done button: commit any pending rename, then close. Clicking Done blurs
  // the field first (which starts commitRename), so this second call is usually a
  // guarded no-op — it's here so Done also commits when the field never blurred.
  async function finish() {
    await commitRename();
    onclose();
  }

  async function remove() {
    const ok = await confirmDanger(`Delete layout "${layout.name}"? This cannot be undone.`, 'Delete layout');
    if (!ok) return;
    busy = true;
    error = '';
    try {
      await deleteLayout(layout.id);
      ondeleted(layout.id);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      busy = false;
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
    <span class="nld-title">Edit layout</span>
    <button type="button" class="sc-btn sc-btn--icon sc-btn--ghost" title="Close" onclick={onclose}>
      <Icon name="minus" />
    </button>
  </header>

  <div class="nld-body">
    <label class="sc-micro nld-label" for="cl-name">Layout name</label>
    <!-- svelte-ignore a11y_autofocus -->
    <input
      id="cl-name"
      class="sc-input"
      bind:value={name}
      disabled={busy}
      onblur={commitRename}
      onkeydown={(e) => {
        if (e.key === 'Enter') (e.currentTarget as HTMLInputElement).blur();
        if (e.key === 'Escape') {
          name = layout.name;
          onclose();
        }
      }}
      autofocus
    />

    <div class="cl-meta">
      <div class="cl-meta-row">
        <span class="sc-micro">View</span>
        <span class="cl-meta-val">{viewLabel(layout.view)}</span>
      </div>
      <div class="cl-meta-row">
        <span class="sc-micro">Associated table</span>
        <span class="cl-meta-val">{layout.tableName}</span>
      </div>
    </div>

    {#if error}
      <p class="sc-hint nld-error">{error}</p>
    {/if}
  </div>

  <footer class="nld-foot">
    <button type="button" class="cl-delete" onclick={remove} disabled={busy}>
      <Icon name="delete" />Delete layout
    </button>
    <span class="nld-spacer"></span>
    <!-- Not disabled on busy: Done only closes (the rename commits in the
         background), and a busy-disabled Done would swallow the click that
         follows the field's own blur-commit. -->
    <button type="button" class="sc-btn sc-btn--primary" onclick={finish}>Done</button>
  </footer>
</aside>

<style>
  /* Drawer shell — identical recipe to NewLayoutDrawer.svelte (local copy for the
     cross-entry CSS constraint documented there). */
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
  .nld-label {
    display: block;
    margin: 0 0 6px;
  }
  .nld-spacer {
    flex: 1 1 auto;
  }
  .nld-error {
    margin: 14px 0 0;
    color: var(--rm-danger);
  }

  /* Read-only view/table facts. */
  .cl-meta {
    margin-top: 18px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .cl-meta-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }
  .cl-meta-val {
    font-size: 13px;
    color: var(--rm-text);
  }

  /* Delete action — red text, reddens on hover; the one delete affordance. */
  .cl-delete {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font: inherit;
    font-size: 12px;
    font-weight: 500;
    color: var(--rm-danger);
    padding: 6px 11px;
    border: 0.5px solid var(--rm-border);
    border-radius: var(--rm-radius);
    background: var(--rm-control-bg);
    box-shadow: var(--rm-elev-1);
    cursor: pointer;
    transition:
      background 0.12s ease,
      color 0.12s ease,
      border-color 0.12s ease;
  }
  .cl-delete:hover:not(:disabled) {
    background: var(--rm-danger);
    border-color: transparent;
    color: #fff;
  }
  .cl-delete:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
