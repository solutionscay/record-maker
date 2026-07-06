<script lang="ts">
  // A searchable field picker (#79) — the icon-aware replacement for the native
  // `<select>`s that both the rail's Field tool and the inspector's Binding row
  // used to draw over `doc.fields`. Field lists get long and native `<option>`s
  // can't render a glyph, so this pairs a text filter + popover list with a
  // per-kind type icon (Icon.svelte / the #72 sprite). It commits a field id
  // exactly like the select it replaces: `onselect(id)` fires on Enter/click.
  //
  // Styling follows the surrounding "modern Mac" controls (the same --rm-* tokens
  // the `.ctl-select`/`.le-select` rules use), so it drops into either panel.
  import type { FieldChoice } from './model';
  import Icon from './Icon.svelte';
  import { FIELD_DRAG_MIME } from './dnd';
  import { kindIcon } from '../shared/field-kinds';

  let {
    fields,
    value,
    values = [],
    onselect,
    onselectMany,
    onclear,
    multi = false,
    disabled = false,
    placeholder = 'Select field…',
    title = 'Field',
    dragToPlace = false,
  }: {
    fields: readonly FieldChoice[];
    /** Currently-bound field id, or null when unset (e.g. an unresolved binding). */
    value: number | null;
    /** Currently selected field ids for multi-select placement. */
    values?: readonly number[];
    /** Commit the chosen field id — the moral equivalent of the old `onchange`. */
    onselect: (id: number) => void;
    /** Commit the chosen field ids for multi-select placement. */
    onselectMany?: (ids: number[]) => void;
    /** Deselect everything (multi mode's Clear button). Distinct from
     * `onselectMany([])` so callers have one obvious place to hang extra
     * cleanup (e.g. syncing a tool's armed state) on an explicit clear. */
    onclear?: () => void;
    multi?: boolean;
    disabled?: boolean;
    placeholder?: string;
    title?: string;
    /** Let rows be dragged out of the list (e.g. onto the layout canvas) instead
     * of only clicked. Off by default: a single-select instance like the
     * inspector's Binding row rebinds an EXISTING object and dragging out of it
     * would be a confusing affordance there, so only the placement picker
     * (RailTools' "Field to place") opts in. */
    dragToPlace?: boolean;
  } = $props();

  // Each FieldKind::as_str value → the sprite symbol drawn beside the name
  // (the shared kind table, #132); an unknown kind falls back to the text glyph.
  function iconFor(kind: string): string {
    return kindIcon(kind, 'type-text');
  }

  let open = $state(false);
  let query = $state('');
  let highlight = $state(0);
  let rangeAnchorId = $state<number | null>(null);
  let root = $state<HTMLDivElement | null>(null);
  let input = $state<HTMLInputElement | null>(null);
  let listEl = $state<HTMLUListElement | null>(null);

  let isEmpty = $derived(fields.length === 0);
  let selectedField = $derived(fields.find((f) => f.id === value) ?? null);
  let selectedSet = $derived(new Set(values));
  let selectedFields = $derived(fields.filter((f) => selectedSet.has(f.id)));
  let hasTriggerSelection = $derived(multi ? selectedFields.length > 0 : selectedField !== null);
  let triggerLabel = $derived(
    multi
      ? selectedFields.length === 0
        ? isEmpty
          ? 'No fields'
          : placeholder
        : selectedFields.length === 1
          ? selectedFields[0].name
          : `${selectedFields.length} fields`
      : selectedField?.name ?? (isEmpty ? 'No fields' : placeholder),
  );
  let triggerIcon = $derived(multi ? selectedFields[0]?.kind : selectedField?.kind);
  let filtered = $derived(
    query.trim() === ''
      ? fields.slice()
      : fields.filter((f) => f.name.toLowerCase().includes(query.trim().toLowerCase())),
  );

  function openPopover(): void {
    if (disabled || isEmpty) return;
    query = '';
    open = true;
    // Highlight the current selection when it survives the (empty) filter.
    const cur = value === null ? -1 : filtered.findIndex((f) => f.id === value);
    highlight = cur >= 0 ? cur : 0;
    rangeAnchorId = value;
  }
  function close(): void {
    open = false;
  }
  function commit(f: FieldChoice | undefined): void {
    if (!f) return;
    rangeAnchorId = f.id;
    onselect(f.id);
    close();
  }
  function toggle(f: FieldChoice | undefined): void {
    if (!f) return;
    const next = new Set(values);
    if (next.has(f.id)) next.delete(f.id);
    else next.add(f.id);
    rangeAnchorId = f.id;
    onselectMany?.([...next]);
  }
  function addRange(f: FieldChoice | undefined): void {
    if (!f) return;
    const anchorId = rangeAnchorId ?? f.id;
    const anchorIndex = filtered.findIndex((field) => field.id === anchorId);
    const targetIndex = filtered.findIndex((field) => field.id === f.id);
    if (anchorIndex < 0 || targetIndex < 0) {
      toggle(f);
      return;
    }
    const [from, to] = anchorIndex <= targetIndex ? [anchorIndex, targetIndex] : [targetIndex, anchorIndex];
    const next = new Set(values);
    for (const field of filtered.slice(from, to + 1)) next.add(field.id);
    rangeAnchorId = f.id;
    onselectMany?.([...next]);
  }
  function choose(f: FieldChoice | undefined, e: KeyboardEvent | MouseEvent): void {
    if (multi && e.shiftKey) addRange(f);
    else if (multi && (e.ctrlKey || e.metaKey)) toggle(f);
    else commit(f);
  }

  // Drag a row out to place it (or the whole current multi-selection) on the
  // canvas. Native HTML5 DnD, not a pointer gesture: it crosses into the
  // canvas island's own event handling for free, and — the actual point of
  // this — `draggable="true"` is what stops the browser from treating the
  // press-drag as a text selection (the "just selects text" symptom this
  // replaces).
  function handleDragStart(e: DragEvent, f: FieldChoice | undefined): void {
    if (!f || !e.dataTransfer) return;
    // Dragging a row that's already part of the multi-selection drags the
    // WHOLE selection (the common "grab any selected item" list convention).
    // Dragging an unselected row drags just that row, and it becomes the
    // selection — same as a plain click would, so the picker stays consistent
    // with what just left it.
    const dragging = multi && selectedSet.has(f.id) ? [...values] : [f.id];
    if (!(multi && selectedSet.has(f.id))) {
      rangeAnchorId = f.id;
      onselectMany?.(dragging);
    }
    e.dataTransfer.effectAllowed = 'copy';
    e.dataTransfer.setData(FIELD_DRAG_MIME, JSON.stringify(dragging));
  }

  // Keep the highlight within bounds as the filter shrinks the list.
  $effect(() => {
    if (highlight >= filtered.length) highlight = Math.max(0, filtered.length - 1);
  });

  // Focus the filter input and scroll the highlight into view while open.
  $effect(() => {
    if (open) input?.focus();
  });
  $effect(() => {
    if (!open || !listEl) return;
    const el = listEl.children[highlight] as HTMLElement | undefined;
    el?.scrollIntoView({ block: 'nearest' });
  });

  // Dismiss on an outside pointer press (the islands live in the server document).
  $effect(() => {
    if (!open) return;
    function onDown(e: PointerEvent): void {
      if (root && !root.contains(e.target as Node)) close();
    }
    document.addEventListener('pointerdown', onDown, true);
    return () => document.removeEventListener('pointerdown', onDown, true);
  });

  function onTriggerKeydown(e: KeyboardEvent): void {
    if (e.key === 'ArrowDown' || e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      openPopover();
    }
  }
  function onInputKeydown(e: KeyboardEvent): void {
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        if (filtered.length) highlight = (highlight + 1) % filtered.length;
        break;
      case 'ArrowUp':
        e.preventDefault();
        if (filtered.length) highlight = (highlight - 1 + filtered.length) % filtered.length;
        break;
      case 'Enter':
        e.preventDefault();
        choose(filtered[highlight], e);
        break;
      case 'Escape':
        e.preventDefault();
        close();
        break;
    }
  }
</script>

<div class="fs" bind:this={root}>
  <button
    type="button"
    class="fs-trigger"
    class:fs-placeholder={!hasTriggerSelection}
    {title}
    disabled={disabled || isEmpty}
    aria-haspopup="listbox"
    aria-expanded={open}
    onclick={() => (open ? close() : openPopover())}
    onkeydown={onTriggerKeydown}
  >
    <span class="fs-current">
      {#if hasTriggerSelection}
        <Icon name={iconFor(triggerIcon ?? 'text')} />
        <span class="fs-name">{triggerLabel}</span>
      {:else}
        <span class="fs-name">{triggerLabel}</span>
      {/if}
    </span>
    <svg class="fs-caret" width="10" height="7" viewBox="0 0 10 7" aria-hidden="true"
      ><path d="M1 1.5 5 5.5 9 1.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" /></svg
    >
  </button>

  {#if open}
    <div class="fs-pop">
      <input
        class="fs-input"
        type="text"
        placeholder="Search fields…"
        bind:this={input}
        bind:value={query}
        onkeydown={onInputKeydown}
      />
      <ul class="fs-list" role="listbox" aria-multiselectable={multi || undefined} bind:this={listEl}>
        {#each filtered as f, i (f.id)}
          <!-- Combobox pattern: arrow/Enter/Escape are handled on the filter input
               above; the row click is a pointer affordance for the same commit. -->
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <li
            role="option"
            aria-selected={multi ? selectedSet.has(f.id) : f.id === value}
            class="fs-opt"
            class:fs-active={i === highlight}
            class:fs-selected={multi && selectedSet.has(f.id)}
            draggable={dragToPlace}
            onpointerenter={() => (highlight = i)}
            onclick={(e) => choose(f, e)}
            ondragstart={(e) => handleDragStart(e, f)}
          >
            {#if multi}
              <span class="fs-check">{selectedSet.has(f.id) ? '✓' : ''}</span>
            {/if}
            <Icon name={iconFor(f.kind)} />
            <span class="fs-name">{f.name}</span>
          </li>
        {/each}
        {#if filtered.length === 0}
          <li class="fs-none">No matches</li>
        {/if}
      </ul>
      {#if multi}
        <div class="fs-actions">
          {#if values.length > 0}
            <button type="button" class="fs-clear" onclick={() => onclear?.()}>Clear</button>
          {/if}
          <button type="button" onclick={close}>Done</button>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .fs {
    position: relative;
    width: 100%;
  }
  .fs-trigger {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    font: inherit;
    font-size: 13px;
    color: var(--rm-text);
    text-align: left;
    padding: 7px 10px;
    border: 0.5px solid var(--rm-border);
    border-radius: 0;
    background: var(--rm-control-bg);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.04);
    cursor: pointer;
  }
  .fs-trigger:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .fs-placeholder .fs-name {
    color: var(--rm-text-dim);
  }
  .fs-current {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    flex: 1 1 auto;
  }
  /* Flexbox centers the icon and name by box height, but the sprite's ink sits
     high in its box relative to the font's optical center, reading as
     misaligned; nudge it down to match. */
  .fs-current :global(.icon) {
    margin-top: 2px;
  }
  .fs-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fs-caret {
    flex: 0 0 auto;
    color: #8a8a8e;
  }
  .fs-pop {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    right: 0;
    z-index: 40;
    padding: 5px;
    border: 0.5px solid var(--rm-border);
    border-radius: 8px;
    background: var(--rm-panel-bg, var(--rm-control-bg));
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.18);
  }
  .fs-input {
    width: 100%;
    font: inherit;
    font-size: 13px;
    color: var(--rm-text);
    padding: 6px 9px;
    margin-bottom: 5px;
    border: 0.5px solid var(--rm-border);
    border-radius: 0;
    background: var(--rm-control-bg);
  }
  .fs-input:focus {
    outline: none;
    border-color: var(--rm-accent);
    box-shadow: 0 0 0 2px rgba(10, 132, 255, 0.25);
  }
  .fs-list {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 220px;
    overflow-y: auto;
  }
  .fs-opt {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 6px 8px;
    border-radius: 0;
    font-size: 13px;
    color: var(--rm-text);
    cursor: pointer;
  }
  .fs-opt[draggable='true'] {
    cursor: grab;
  }
  .fs-opt[draggable='true']:active {
    cursor: grabbing;
  }
  .fs-selected:not(.fs-active) {
    background: rgba(10, 132, 255, 0.12);
  }
  .fs-check {
    width: 14px;
    flex: 0 0 14px;
    text-align: center;
    font-size: 12px;
    font-weight: 700;
  }
  .fs-active {
    background: var(--rm-accent);
    color: #fff;
  }
  .fs-none {
    padding: 8px;
    font-size: 12px;
    color: var(--rm-text-dim);
    text-align: center;
  }
  .fs-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
    padding-top: 5px;
    margin-top: 5px;
    border-top: 0.5px solid var(--rm-border);
  }
  .fs-actions button {
    font: inherit;
    font-size: 12px;
    padding: 4px 8px;
    border: 0.5px solid var(--rm-border);
    border-radius: 0;
    background: var(--rm-control-bg);
    color: var(--rm-text);
    cursor: pointer;
  }
  .fs-actions button.fs-clear {
    margin-right: auto;
    border-color: transparent;
    background: transparent;
    color: var(--rm-text-dim);
  }
</style>
