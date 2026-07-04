// Shared drag-and-drop contract between the field picker (FieldSelect.svelte,
// the drag source) and the canvas interaction layer (interaction.ts, the drop
// target) — two different Svelte islands (#62) that only otherwise share the
// store. Native HTML5 DnD crosses that island boundary for free (it's browser
// events, not component state), which is why this uses `draggable` +
// `dataTransfer` instead of a pointer-tracked custom drag.

/** Custom MIME type carrying a JSON `number[]` of field ids being dragged from
 * the picker onto the canvas. Kept off `text/plain` so it doesn't leak into
 * unrelated drop targets (e.g. a text input) as literal JSON text. */
export const FIELD_DRAG_MIME = 'application/x-rm-field-ids';
