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

/** Custom MIME type for a PORTAL-COLUMN drag (#168): related fields dragged out
 * of a portal's inspector Columns picker onto the canvas, to become columns of
 * THAT portal. Distinct from FIELD_DRAG_MIME so the same drag gesture routes to
 * the parent-aware create (parentObjectId = portal, route-relative binding)
 * instead of a top-level base-field placement — the field ids alone can't tell
 * the two apart. Carries the owning portal id + declared route alongside them. */
export const PORTAL_COLUMN_DRAG_MIME = 'application/x-rm-portal-column';

/** JSON payload carried by {@link PORTAL_COLUMN_DRAG_MIME}: the related field ids
 * to add as columns, plus the portal they belong to and its declared route path
 * (so the drop handler can POST a route-relative column create). */
export interface PortalColumnDrag {
  portalId: number;
  route: string;
  fieldIds: number[];
}
