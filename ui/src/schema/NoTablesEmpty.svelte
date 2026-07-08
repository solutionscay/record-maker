<script lang="ts">
  // The single "no tables yet" empty state, shared by all three Manage Database
  // tabs (Tables / Fields / Relationships) so they read identically before any
  // table exists. Rendered by SchemaApp directly into the body (a flex column),
  // so `margin: auto` centers it in the pane. The one action is always "New
  // table" — nothing else can happen until a table exists.
  import Icon from '../lib/Icon.svelte';

  let { onnew }: { onnew: () => void } = $props();
</script>

<div class="nt-empty">
  <p class="sc-empty-title">No tables yet</p>
  <p class="sc-hint">Create a table to start defining fields and relationships.</p>
  <button type="button" class="sc-btn sc-btn--primary nt-btn" onclick={onnew}>
    <Icon name="plus" />New table
  </button>
</div>

<style>
  .nt-empty {
    margin: auto;
    padding: 3rem;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
  }
  .nt-btn {
    margin-top: 6px;
  }
</style>
