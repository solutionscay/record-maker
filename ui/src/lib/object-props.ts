import type { ObjectDoc } from './doc.svelte';

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
