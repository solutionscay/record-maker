import { mount } from 'svelte';
import './layout-manager.css';
import LayoutManagerApp from './LayoutManagerApp.svelte';

// Layout Manager island entry (#149). A single mounted island (not an SPA
// page): the axum server renders the shell in `layouts` mode with one
// `#layouts-root` node, and this bundle owns the whole surface — the flat
// layout list, create/rename/delete, and drag-to-reorder — talking to the
// `/layouts/*` endpoints.
const root = document.getElementById('layouts-root');
if (root) {
  mount(LayoutManagerApp, { target: root });
}
