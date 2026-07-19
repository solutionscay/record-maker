import type { ObjectDoc, ToolKind } from '../doc.svelte';
import { defaultProps } from '../create';
import {
  lineAngle,
  lineGeometryForAngle,
  lineLength,
  linePropsForBox,
  lineShapeStyle,
  normalizeAngle,
  numberProp,
  parseProps,
} from '../object-props';

type ObjectTool = Exclude<ToolKind, 'pointer'>;

export type PlacementFrame = {
  dragged: boolean;
  box: { x: number; y: number; w: number; h: number };
  partTop: number;
  line: { angle: number; length: number } | null;
};

export type PreviewStyle = {
  left: number;
  top: number;
  width: number;
  height: number;
  transform: string;
};

export type RotationFrame = {
  geometry: { x: number; y: number; w: number; h: number };
  props: Record<string, unknown>;
};

export type DrawGeometryInput = {
  startX: number;
  startY: number;
  endX: number;
  endY: number;
  snap(value: number): number;
};

export type DrawGeometry = {
  x: number;
  yGlobal: number;
  w: number;
  h: number;
  line: { angle: number; length: number } | null;
};

export interface ObjectTypeBehavior {
  defaultContent: string | null;
  rotatable: boolean;
  persistAfterResize: boolean;
  drawGeometry(input: DrawGeometryInput): DrawGeometry;
  previewStyle(frame: PlacementFrame): PreviewStyle | null;
  placementProps(frame: PlacementFrame): Record<string, unknown> | null;
  rotationStart(object: Readonly<ObjectDoc>): { angle: number; measure: number } | null;
  onRotate(object: Readonly<ObjectDoc>, angle: number, measure: number): RotationFrame | null;
  syncGeometry(object: Readonly<ObjectDoc>): Record<string, unknown> | null;
  shapeStyle(props: Record<string, unknown>): string | null;
}

function genericBehavior(kind: string): ObjectTypeBehavior {
  return {
    defaultContent: kind === 'text' ? 'Text' : null,
    rotatable: false,
    persistAfterResize: false,
    drawGeometry: ({ startX, startY, endX, endY }) => ({
      x: Math.min(startX, endX),
      yGlobal: Math.min(startY, endY),
      w: Math.max(8, Math.abs(endX - startX)),
      h: Math.max(8, Math.abs(endY - startY)),
      line: null,
    }),
    previewStyle: () => null,
    placementProps: () => defaultProps(kind as ObjectTool) ?? null,
    rotationStart: () => null,
    onRotate: () => null,
    syncGeometry: () => null,
    shapeStyle: () => null,
  };
}

const LINE_BEHAVIOR: ObjectTypeBehavior = {
  defaultContent: null,
  rotatable: true,
  persistAfterResize: true,
  drawGeometry: ({ startX, startY, endX, endY, snap }) => {
    const sx = snap(startX);
    const sy = snap(startY);
    const ex = snap(endX);
    const ey = snap(endY);
    return {
      x: Math.min(sx, ex),
      yGlobal: Math.min(sy, ey),
      w: Math.max(1, Math.abs(ex - sx)),
      h: Math.max(1, Math.abs(ey - sy)),
      line: {
        angle: lineAngle(sx, sy, ex, ey),
        length: Math.max(1, Math.hypot(ex - sx, ey - sy)),
      },
    };
  },
  previewStyle: (frame) => {
    const line = frame.line ?? { angle: 0, length: Math.max(1, frame.box.w) };
    return {
      left: frame.box.x + frame.box.w / 2 - line.length / 2,
      top: frame.partTop + frame.box.y + frame.box.h / 2 - 1,
      width: line.length,
      height: 2,
      transform: `rotate(${line.angle}deg)`,
    };
  },
  placementProps: (frame) => {
    const base = defaultProps('line');
    const line = frame.dragged && frame.line
      ? frame.line
      : { angle: 0, length: Math.max(1, frame.box.w) };
    return { ...(base ?? {}), angle: line.angle, length: line.length };
  },
  rotationStart: (object) => {
    const props = parseProps(object.props);
    return { angle: numberProp(props.angle, 0), measure: lineLength(object, props) };
  },
  onRotate: (object, angle, measure) => {
    const nextAngle = normalizeAngle(angle);
    const length = measure || lineLength(object, parseProps(object.props));
    return {
      geometry: lineGeometryForAngle(object, nextAngle, length),
      props: { ...parseProps(object.props), angle: nextAngle, length },
    };
  },
  syncGeometry: (object) => linePropsForBox(object, parseProps(object.props)),
  shapeStyle: (props) => lineShapeStyle(props),
};

const REGISTRY = new Map<string, ObjectTypeBehavior>([['line', LINE_BEHAVIOR]]);
const GENERIC = new Map<string, ObjectTypeBehavior>();

export function objectBehavior(kind: string): ObjectTypeBehavior {
  const registered = REGISTRY.get(kind);
  if (registered) return registered;
  let behavior = GENERIC.get(kind);
  if (!behavior) {
    behavior = genericBehavior(kind);
    GENERIC.set(kind, behavior);
  }
  return behavior;
}
