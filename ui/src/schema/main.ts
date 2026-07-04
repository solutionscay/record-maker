import { mount } from 'svelte';
import SchemaApp from './SchemaApp.svelte';

// Schema-builder island entry (#113). A single mounted island (not an SPA page):
// the axum server renders the shell in `schema` mode with one `#schema-root`
// node, and this bundle owns the whole surface — table list, field grid, and the
// master-detail field drawer — talking to the #107 `/schema/*` endpoints.
const root = document.getElementById('schema-root');
if (root) {
  mount(SchemaApp, { target: root });
}
