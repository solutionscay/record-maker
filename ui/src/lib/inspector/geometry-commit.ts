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
  const committed = doc.getObject(id);
  if (!committed) return;
  const geometry = { x: committed.x, y: committed.y, w: committed.w, h: committed.h };
  const lineProps = committed.kind === 'line' ? parseProps(committed.props) : null;
  doc.mark();
  llog('persist', `inspector: set object ${operation}`, { id, control, geometry });

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
