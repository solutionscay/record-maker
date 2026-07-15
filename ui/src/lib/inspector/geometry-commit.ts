// Shared live + committed geometry path for the Inspector's Position and Size
// sections (#187/#188). Both surfaces persist full x/y/w/h snapshots, so they
// must share a per-object queue or a rapid Position -> Size edit could let an
// older request arrive last and overwrite newer geometry.

import type { EditorDoc, Geometry } from '../doc.svelte';
import { llog } from '../log';
import { linePropsForBox, lineShapeStyle, parseProps } from '../object-props';
import { setObjectGeometry as persistGeometry } from '../persist';
import { persistObjectPropsAndRefresh, reportPersistError } from './persist-ops';

const persistQueues = new WeakMap<EditorDoc, Map<number, Promise<void>>>();

/** Apply inspector geometry immediately. Lines also derive their visible stroke
 * from the new box, mirroring the canvas handle-resize path. */
export function applyLiveObjectGeometry(doc: EditorDoc, id: number, geometry: Geometry): void {
  const current = doc.getObject(id);
  if (!current) return;
  doc.setObjectGeometry(id, geometry);

  const resized = doc.getObject(id);
  if (!resized || resized.kind !== 'line') return;
  const nextProps = linePropsForBox(resized, parseProps(resized.props));
  doc.setObjectProps(id, JSON.stringify(nextProps));

  // Shape styles are ordinarily server-derived. During live input, derive the
  // same line-only CSS locally, then refresh from the server after commit.
  const resolved = doc.getResolved(id);
  if (resolved) {
    doc.setObjectStyles(id, {
      objectStyle: resolved.objectStyle,
      textStyle: resolved.textStyle,
      shapeStyle: lineShapeStyle(nextProps),
    });
  }
}

/** Seal current live geometry as one undo step and enqueue its full snapshot for
 * persistence. The queue is shared by Position and Size controls per object. */
export function commitObjectGeometry(
  doc: EditorDoc,
  layoutId: string,
  id: number,
  operation: 'position' | 'size',
  control: string,
): void {
  commitObjectGeometries(doc, layoutId, [id], operation, control);
}

/** Seal and persist the current snapshots for one or many objects. Applying all
 * live edits before this call lets one mark capture the entire multi-selection
 * as a single undo step. Persistence remains queued per object, preserving the
 * Position -> Size ordering guarantee when edits overlap. */
export function commitObjectGeometries(
  doc: EditorDoc,
  layoutId: string,
  ids: readonly number[],
  operation: 'position' | 'size',
  control: string,
): void {
  const commits = ids.flatMap((id) => {
    const committed = doc.getObject(id);
    if (!committed) return [];
    return [{
      id,
      geometry: { x: committed.x, y: committed.y, w: committed.w, h: committed.h },
      lineProps: committed.kind === 'line' ? parseProps(committed.props) : null,
    }];
  });
  if (commits.length === 0) return;
  doc.mark();
  llog('persist', `inspector: set object ${operation}${commits.length > 1 ? ' (many)' : ''}`, {
    ids: commits.map(({ id }) => id),
    control,
    geometries: commits.map(({ id, geometry }) => ({ id, ...geometry })),
  });

  for (const { id, geometry, lineProps } of commits) enqueueGeometryPersistence(
    doc,
    layoutId,
    id,
    geometry,
    lineProps,
    operation,
  );
}

function enqueueGeometryPersistence(
  doc: EditorDoc,
  layoutId: string,
  id: number,
  geometry: { x: number; y: number; w: number; h: number },
  lineProps: Record<string, unknown> | null,
  operation: 'position' | 'size',
): void {
  let byId = persistQueues.get(doc);
  if (!byId) {
    byId = new Map();
    persistQueues.set(doc, byId);
  }
  const previous = byId.get(id) ?? Promise.resolve();
  const queued = previous.then(async () => {
    try {
      await persistGeometry(layoutId, id, geometry);
      if (lineProps) {
        await persistObjectPropsAndRefresh(doc, layoutId, id, lineProps, `set line ${operation}`);
      }
    } catch (e) {
      reportPersistError(doc, `set object ${operation}`, e);
    }
  });
  byId.set(id, queued);
  void queued.finally(() => {
    if (byId?.get(id) === queued) byId.delete(id);
  });
}
