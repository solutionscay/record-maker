import { mount } from 'svelte';
import App from './App.svelte';

// Mount the Layout Mode editor island into the node the design page renders
// (<div id="layout-editor" data-layout="…">). The layout id comes from that
// node's data-layout attribute. The island talks to the engine over the existing
// axum HTTP endpoints (ADR #42), not Tauri IPC.
const target = document.getElementById('layout-editor');

if (target) {
  mount(App, {
    target,
    props: { layoutId: target.dataset.layout ?? '' },
  });
}
