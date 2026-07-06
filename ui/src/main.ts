import { mount } from 'svelte';
import './lib/layout-editor.css';
import App from './App.svelte';
import RailTools from './lib/RailTools.svelte';
import Inspector from './lib/Inspector.svelte';
import { EditorDoc } from './lib/doc.svelte';
import type { DesignModel } from './lib/model';
import { llog, lerror } from './lib/log';

// Layout Mode editor COORDINATOR (#62). It owns the single EditorDoc store and
// mounts three islands that SHARE it: the canvas into `#layout-editor` (the content
// area), the rail tools into `#layout-tools` (the left sidebar), and the
// selection-aware inspector into `#layout-inspector` (the right panel) — the last
// two in design mode only. All read/write the same store, so a tool armed in the
// rail drives placement on the canvas and a selection on the canvas drives the
// inspector's controls. The store is hydrated once here over the existing axum
// model endpoint (ADR #42); the canvas reacts to `doc.hydrated`.
const canvasNode = document.getElementById('layout-editor');

if (canvasNode) {
  const layoutId = canvasNode.dataset.layout ?? '';
  const doc = new EditorDoc();
  llog('init', 'coordinator start', { layoutId });

  // Fetch + hydrate the shared store. Errors surface through the store so the
  // canvas can show them.
  fetch(`/design/${layoutId}/model`)
    .then((r) => {
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      return r.json();
    })
    .then((data: DesignModel) => doc.hydrate(data))
    .catch((e: unknown) => {
      lerror('init', 'model fetch failed', e);
      doc.setError(e instanceof Error ? e.message : String(e));
    });

  mount(App, { target: canvasNode, props: { doc, layoutId } });

  // The rail-tools island only exists in design mode (the server renders the node
  // in the sidebar). Mount it sharing the SAME store.
  const toolsNode = document.getElementById('layout-tools');
  if (toolsNode) {
    mount(RailTools, { target: toolsNode, props: { doc, layoutId } });
  }

  // The inspector island only exists in design mode (the server renders the node
  // in the right panel). Mount it sharing the SAME store.
  const inspectorNode = document.getElementById('layout-inspector');
  if (inspectorNode) {
    mount(Inspector, { target: inspectorNode, props: { doc, layoutId } });
  }
  llog('init', 'islands mounted', { canvas: true, rail: !!toolsNode, inspector: !!inspectorNode });
}
