<script lang="ts">
  // Relationship connector drawer (#174). The schema Relationships graph is
  // otherwise strictly read-only; this drawer is its SOLE editing affordance and
  // edits ONLY the two portal permission flags — allow-create and allow-delete —
  // on an existing relationship. It never edits the relationship's structure
  // (name/tables/fields come from the field reference in the Fields tab) and
  // never creates or deletes a relationship. Toggling a flag persists
  // immediately through the store (like a graph box position), so there is no
  // Save/Cancel — just Close.
  import type { SchemaStore } from './store.svelte';
  import type { RelationshipView } from './types';
  import SchemaDrawer from './SchemaDrawer.svelte';

  let {
    store,
    relationship,
    onclose,
  }: {
    store: SchemaStore;
    relationship: RelationshipView | null;
    onclose: () => void;
  } = $props();

  const fromTable = $derived(relationship == null ? null : store.tableById(relationship.fromTable));
  const toTable = $derived(relationship == null ? null : store.tableById(relationship.toTable));
  const fromField = $derived(
    relationship == null ? null : store.fieldById(relationship.fromTable, relationship.fromField),
  );
  const toField = $derived(
    relationship == null ? null : store.fieldById(relationship.toTable, relationship.toField),
  );

  function toggleCreate(next: boolean) {
    if (relationship) void store.setRelationshipReferential(relationship.id, next, relationship.allowDelete);
  }
  function toggleDelete(next: boolean) {
    if (relationship) void store.setRelationshipReferential(relationship.id, relationship.allowCreate, next);
  }
  function commitName(next: string) {
    if (!relationship) return;
    const name = next.trim();
    if (!name || name === relationship.name) return;
    void store.renameRelationship(relationship.id, name);
  }
</script>

<SchemaDrawer title="Relationship" {onclose}>
  {#snippet footer()}
    <span class="rd-spacer"></span>
    <button type="button" class="sc-btn sc-btn--primary" onclick={onclose}>Close</button>
  {/snippet}

  {#if relationship}
    <div class="rd-context">
      <input
        class="rd-name-input"
        value={relationship.name}
        aria-label="Relationship name"
        title="Rename this relationship — updates every portal bound to it"
        spellcheck="false"
        autocomplete="off"
        onchange={(e) => commitName(e.currentTarget.value)}
        onkeydown={(e) => {
          if (e.key === 'Enter') e.currentTarget.blur();
          if (e.key === 'Escape') {
            e.currentTarget.value = relationship?.name ?? '';
            e.currentTarget.blur();
          }
        }}
      />
      <div class="rd-path">
        <span class="rd-endpoint">{fromTable?.name ?? 'Missing table'}<span class="rd-field">.{fromField?.name ?? '?'}</span></span>
        <span class="rd-arrow" aria-hidden="true">&rarr;</span>
        <span class="rd-endpoint">{toTable?.name ?? 'Missing table'}<span class="rd-field">.{toField?.name ?? '?'}</span></span>
      </div>
      <span class="sc-hint rd-card">
        Each {fromTable?.name ?? 'record'} references {relationship.forwardCardinality} {toTable?.name ?? 'record'}; each
        {toTable?.name ?? 'record'} has {relationship.reverseCardinality} {fromTable?.name ?? 'record'}.
      </span>
    </div>

    <section class="rd-section" aria-labelledby="rd-perms">
      <span id="rd-perms" class="sc-micro rd-label">Portal permissions</span>
      <label class="rd-switch">
        <span class="rd-switch-text">
          Allow create
          <span class="sc-hint rd-switch-sub">New related records can be added through a portal.</span>
        </span>
        <span class="rd-toggle">
          <input
            type="checkbox"
            checked={relationship.allowCreate}
            onchange={(e) => toggleCreate(e.currentTarget.checked)}
          />
          <span class="rd-track"><span class="rd-knob"></span></span>
        </span>
      </label>
      <label class="rd-switch">
        <span class="rd-switch-text">
          Allow delete
          <span class="sc-hint rd-switch-sub">Related records can be removed through a portal.</span>
        </span>
        <span class="rd-toggle">
          <input
            type="checkbox"
            checked={relationship.allowDelete}
            onchange={(e) => toggleDelete(e.currentTarget.checked)}
          />
          <span class="rd-track"><span class="rd-knob"></span></span>
        </span>
      </label>
    </section>

    <p class="sc-hint rd-note">Renaming and permission changes are saved immediately, and update every portal bound to this relationship. Its fields are defined by the field reference in the Fields tab.</p>
  {:else}
    <p class="sc-hint">This relationship no longer exists.</p>
  {/if}
</SchemaDrawer>

<style>
  .rd-context {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 13px;
    border: 0.5px solid var(--rm-border);
    border-radius: 7px;
    background: var(--rm-control-bg);
    box-shadow: var(--sc-shadow);
  }
  .rd-name-input {
    font: inherit;
    font-size: 14px;
    font-weight: 700;
    color: var(--rm-text);
    width: 100%;
    box-sizing: border-box;
    padding: 4px 6px;
    margin: -4px -6px 0;
    border: 1px solid transparent;
    border-radius: 5px;
    background: transparent;
    transition: border-color 0.12s ease, background 0.12s ease;
  }
  .rd-name-input:hover {
    border-color: var(--rm-border);
  }
  .rd-name-input:focus {
    outline: none;
    border-color: var(--rm-accent);
    background: var(--rm-control-bg);
  }
  .rd-path {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    font-size: 12px;
    color: var(--rm-text);
  }
  .rd-endpoint {
    font-weight: 600;
  }
  .rd-field {
    font-weight: 400;
    color: var(--rm-text-dim);
  }
  .rd-arrow {
    color: var(--rm-text-dim);
  }
  .rd-card {
    line-height: 1.45;
  }
  .rd-section {
    margin-top: 18px;
  }
  .rd-label {
    display: block;
    margin: 0 0 8px;
  }
  /* iOS-style switch — same geometry as the field drawer's switches. */
  .rd-switch {
    min-height: 40px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 6px 0;
    font-size: 13px;
    color: var(--rm-text);
    cursor: pointer;
  }
  .rd-switch-text {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .rd-switch-sub {
    line-height: 1.35;
  }
  .rd-toggle {
    position: relative;
    display: inline-flex;
    flex: none;
  }
  .rd-toggle input {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
  }
  .rd-track {
    width: 36px;
    height: 21px;
    border-radius: 21px;
    background: var(--rm-segment-track);
    transition: background 0.15s ease;
  }
  .rd-knob {
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
  .rd-toggle input:checked + .rd-track {
    background: var(--rm-accent);
  }
  .rd-toggle input:checked + .rd-track .rd-knob {
    left: 17px;
  }
  .rd-note {
    margin: 16px 0 0;
    line-height: 1.45;
  }
  .rd-spacer {
    flex: 1 1 auto;
  }
</style>
