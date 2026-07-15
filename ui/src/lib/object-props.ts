import type { ObjectDoc } from './doc.svelte';

export const DEFAULT_PORTAL_ROW_COUNT = 5;
export const MAX_PORTAL_ROW_COUNT = 1_000;

export type LineBox = Pick<Readonly<ObjectDoc>, 'x' | 'y' | 'w' | 'h'>;

export function parseProps(raw: string | null | undefined): Record<string, unknown> {
  if (!raw) return {};
  try {
    const parsed = JSON.parse(raw) as unknown;
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? (parsed as Record<string, unknown>) : {};
  } catch {
    return {};
  }
}

export function numberProp(value: unknown, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}

/** #184: normalize the portal's explicit repeated-row setting from its props.
 * Invalid metadata falls back to the five-row default; the ceiling mirrors the
 * server so document-store rendering and fresh server projections cannot drift. */
export function portalRowCount(props: Record<string, unknown>): number {
  const raw = props.rowCount;
  if (typeof raw !== 'number' || !Number.isInteger(raw) || raw < 1) return DEFAULT_PORTAL_ROW_COUNT;
  return Math.min(raw, MAX_PORTAL_ROW_COUNT);
}

/** Full visible height of a portal preview/viewport while its editable geometry
 * remains one row. Non-portals retain their ordinary authored height. */
export function objectFootprintHeight(o: Pick<Readonly<ObjectDoc>, 'kind' | 'h' | 'props'>): number {
  return o.kind === 'portal' ? o.h * portalRowCount(parseProps(o.props)) : o.h;
}

export function normalizeAngle(angle: number): number {
  if (!Number.isFinite(angle)) return 0;
  const normalized = ((angle % 360) + 360) % 360;
  return Math.round(normalized * 100) / 100;
}

export function lineAngle(x1: number, y1: number, x2: number, y2: number): number {
  return normalizeAngle((Math.atan2(y2 - y1, x2 - x1) * 180) / Math.PI);
}

export function lineLength(box: Pick<LineBox, 'w' | 'h'>, props: Record<string, unknown>): number {
  return Math.max(1, numberProp(props.length, Math.hypot(box.w, box.h) || box.w || 1));
}

/** Resolve a line's authored angle/length from its resized bounding box. Canvas
 * handles and Inspector pixel inputs both use this path so the visible stroke
 * stays synchronized with w/h instead of only resizing its outer hit box. */
export function linePropsForBox(box: Pick<LineBox, 'w' | 'h'>, props: Record<string, unknown>): Record<string, unknown> {
  const currentAngle = numberProp(props.angle, 0);
  const radians = (currentAngle * Math.PI) / 180;
  const horizontalish = currentAngle <= 5 || currentAngle >= 355 || Math.abs(currentAngle - 180) <= 5;
  const verticalish = Math.abs(currentAngle - 90) <= 5 || Math.abs(currentAngle - 270) <= 5;
  const w = Math.max(1, box.w);
  const h = horizontalish && box.h <= 2 ? 0 : Math.max(1, box.h);
  const dx = (Math.cos(radians) < 0 ? -1 : 1) * (verticalish && box.w <= 2 ? 0 : w);
  const dy = (Math.sin(radians) < 0 ? -1 : 1) * h;
  return {
    ...props,
    angle: lineAngle(0, 0, dx, dy),
    length: Math.max(1, Math.hypot(dx, dy)),
  };
}

export function lineGeometryForAngle(box: LineBox, angle: number, length: number): LineBox {
  const radians = (angle * Math.PI) / 180;
  const w = Math.max(1, Math.round(Math.abs(Math.cos(radians)) * length));
  const h = Math.max(1, Math.round(Math.abs(Math.sin(radians)) * length));
  const cx = box.x + box.w / 2;
  const cy = box.y + box.h / 2;
  return {
    x: Math.max(0, Math.round(cx - w / 2)),
    y: Math.max(0, Math.round(cy - h / 2)),
    w,
    h,
  };
}

export function lineShapeStyle(props: Record<string, unknown>): string {
  const stroke = typeof props.stroke === 'string' ? props.stroke : '#888';
  const strokeWidth = Math.max(1, Math.round(numberProp(props.strokeWidth, 2)));
  const length = Math.max(1, numberProp(props.length, 1));
  const angle = numberProp(props.angle, 0);
  return `background:${stroke};height:${strokeWidth}px;width:${length}px;left:50%;right:auto;transform:translate(-50%,-50%) rotate(${angle}deg);transform-origin:center center;`;
}
