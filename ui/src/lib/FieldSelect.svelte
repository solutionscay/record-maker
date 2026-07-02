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

  let {
    fields,
    value,
    values = [],
    onselect,
    onselectMany,
    multi = false,
    disabled = false,
    placeholder = 'Select field…',
    title = 'Field',
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
    multi?: boolean;
    disabled?: boolean;
    placeholder?: string;
    title?: string;
  } = $props();

  // Each FieldKind::as_str value → the sprite symbol drawn beside the name.
  const KIND_ICON: Record<string, string> = {
    text: 'type-text',
    number: 'type-number',
    date: 'type-date',
    time: 'type-time',
    timestamp: 'type-timestamp',
    bool: 'type-bool',
  };
  function iconFor(kind: string): string {
    return KIND_ICON[kind] ?? 'type-text';
  }

  let open = $state(false);
  let query = $state('');
  let highlight = $state(0);
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
  }
  function close(): void {
    open = false;
  }
  function commit(f: FieldChoice | undefined): void {
    if (!f) return;
    onselect(f.id);
    close();
  }
  function toggle(f: FieldChoice | undefined): void {
    if (!f) return;
    const next = new Set(values);
    if (next.has(f.id)) next.delete(f.id);
    else next.add(f.id);
    onselectMany?.([...next]);
  }
  function wantsMultiToggle(e: KeyboardEvent | MouseEvent): boolean {
    return multi && (e.shiftKey || e.ctrlKey || e.metaKey);
  }
  function choose(f: FieldChoice | undefined, e: KeyboardEvent | MouseEvent): void {
    if (wantsMultiToggle(e)) toggle(f);
    else commit(f);
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
      {#if multi && selectedFields.length > 1}
        <span class="fs-count">{selectedFields.length}</span>
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
            onpointerenter={() => (highlight = i)}
            onclick={(e) => choose(f, e)}
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
    border-radius: 7px;
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
  .fs-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fs-count {
    flex: 0 0 auto;
    min-width: 18px;
    padding: 1px 5px;
    border-radius: 999px;
    background: var(--rm-accent);
    color: #fff;
    font-size: 11px;
    line-height: 1.3;
    text-align: center;
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
    border-radius: 6px;
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
    border-radius: 6px;
    font-size: 13px;
    color: var(--rm-text);
    cursor: pointer;
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
    justify-content: flex-end;
    padding-top: 5px;
    margin-top: 5px;
    border-top: 0.5px solid var(--rm-border);
  }
  .fs-actions button {
    font: inherit;
    font-size: 12px;
    padding: 4px 8px;
    border: 0.5px solid var(--rm-border);
    border-radius: 6px;
    background: var(--rm-control-bg);
    color: var(--rm-text);
    cursor: pointer;
  }
</style>
