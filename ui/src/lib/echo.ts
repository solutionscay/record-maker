// Layout Mode motion feedback (#88). History actions mutate the document store
// first, then this module animates the already-rendered DOM from the prior
// geometry into the new geometry and leaves a short-lived ghost at the old spot.

import { objectIdsInPaintOrder } from './canvas-edit';
import type { EditorDoc, ObjectDoc, Step } from './doc.svelte';
import type { DesignModel } from './model';

type Direction = 'undo' | 'redo';
type GeometryProp = 'partId' | 'x' | 'y' | 'w' | 'h';

interface Rect {
  partId: number;
  x: number;
  y: number;
  w: number;
  h: number;
  z: number;
}

interface EchoSpec {
  id: number;
  from: Rect;
  to: Rect;
}

const GEOMETRY_PROPS = new Set<GeometryProp>(['partId', 'x', 'y', 'w', 'h']);
const stages = new WeakMap<EditorDoc, HTMLElement>();
const active = new WeakMap<HTMLElement, Animation>();

export function registerEchoStage(doc: EditorDoc, stage: HTMLElement): () => void {
  stages.set(doc, stage);
  return () => {
    if (stages.get(doc) === stage) stages.delete(doc);
  };
}

export function playHistoryEcho(doc: EditorDoc, step: Step, direction: Direction): void {
  if (!echoesEnabled()) return;
  const stage = stages.get(doc);
  if (!stage) return;
  const specs = buildHistoryEchoSpecs(doc, step, direction);
  if (specs.length === 0) return;

  queueMicrotask(() => {
    requestAnimationFrame(() => playSpecs(stage, doc.renderModel, specs, direction));
  });
}

export function buildHistoryEchoSpecs(doc: EditorDoc, step: Step, direction: Direction): EchoSpec[] {
  const changed = new Map<number, Map<GeometryProp, number>>();
  for (const d of step) {
    if (d.target !== 'object' || !GEOMETRY_PROPS.has(d.prop as GeometryProp)) continue;
    const prop = d.prop as GeometryProp;
    const value = direction === 'undo' ? d.after : d.before;
    if (typeof value !== 'number') continue;
    let props = changed.get(d.id);
    if (!props) {
      props = new Map();
      changed.set(d.id, props);
    }
    if (direction === 'redo' && props.has(prop)) continue;
    props.set(prop, value);
  }

  const specs: EchoSpec[] = [];
  const tops = partTops(doc.renderModel);
  for (const [id, props] of changed) {
    const current = doc.getObject(id);
    if (!current) continue;
    const to = rectFromObject(current);
    const from = { ...to };
    for (const [prop, value] of props) from[prop] = value;
    if (!tops.has(from.partId) || !tops.has(to.partId)) continue;
    if (sameAbsoluteRect(from, to, tops)) continue;
    specs.push({ id, from, to });
  }
  return specs;
}

function playSpecs(stage: HTMLElement, model: DesignModel, specs: EchoSpec[], direction: Direction): void {
  if (typeof window === 'undefined') return;
  const canvas = stage.querySelector<HTMLElement>('.fm-canvas');
  if (!canvas) return;

  const reduceMotion = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
  const ids = objectIdsInPaintOrder(model);
  const elements = Array.from(canvas.querySelectorAll<HTMLElement>('.fm-obj'));
  const parts = Array.from(canvas.querySelectorAll<HTMLElement>('.fm-part'));
  const tops = partTops(model);

  for (const spec of specs) {
    const element = elements[ids.indexOf(spec.id)];
    if (!element) continue;
    paintGhost(element, parts, model, spec, direction, reduceMotion);
    if (!reduceMotion) animateElement(element, spec, tops);
  }
}

function animateElement(element: HTMLElement, spec: EchoSpec, tops: Map<number, number>): void {
  const fromTop = tops.get(spec.from.partId);
  const toTop = tops.get(spec.to.partId);
  if (fromTop === undefined || toTop === undefined) return;

  const dx = spec.from.x - spec.to.x;
  const dy = fromTop + spec.from.y - (toTop + spec.to.y);
  const sx = spec.to.w > 0 ? spec.from.w / spec.to.w : 1;
  const sy = spec.to.h > 0 ? spec.from.h / spec.to.h : 1;

  active.get(element)?.cancel();
  element.classList.add('le-echo-active');
  const animation = element.animate(
    [
      { transform: `translate(${dx}px, ${dy}px) scale(${sx}, ${sy})`, transformOrigin: 'top left' },
      { transform: `translate(${-dx * 0.035}px, ${-dy * 0.035}px) scale(1, 1)`, transformOrigin: 'top left' },
      { transform: 'translate(0, 0) scale(1, 1)', transformOrigin: 'top left' },
    ],
    {
      duration: 260,
      easing: 'cubic-bezier(0.2, 0.9, 0.18, 1)',
      fill: 'none',
    },
  );
  active.set(element, animation);
  void animation.finished
    .catch(() => {})
    .then(() => {
      if (active.get(element) === animation) {
        active.delete(element);
        element.classList.remove('le-echo-active');
      }
    });
}

function paintGhost(
  element: HTMLElement,
  parts: HTMLElement[],
  model: DesignModel,
  spec: EchoSpec,
  direction: Direction,
  reduceMotion: boolean,
): void {
  const partIndex = model.parts.findIndex((p) => p.id === spec.from.partId);
  const part = partIndex >= 0 ? parts[partIndex] : null;
  if (!part) return;

  const ghost = element.cloneNode(true) as HTMLElement;
  ghost.classList.add('le-echo-ghost', `le-echo-${direction}`);
  ghost.style.left = `${spec.from.x}px`;
  ghost.style.top = `${spec.from.y}px`;
  ghost.style.width = `${spec.from.w}px`;
  ghost.style.height = `${spec.from.h}px`;
  ghost.style.zIndex = `${spec.from.z}`;
  part.appendChild(ghost);

  const duration = reduceMotion ? 140 : 250;
  const animation = ghost.animate(
    [
      { opacity: reduceMotion ? 0.28 : 0.42, transform: 'scale(1)' },
      { opacity: 0, transform: reduceMotion ? 'scale(1)' : 'scale(0.985)' },
    ],
    { duration, easing: 'ease-out', fill: 'forwards' },
  );
  void animation.finished
    .catch(() => {})
    .then(() => ghost.remove());
}

function rectFromObject(o: Readonly<ObjectDoc>): Rect {
  return { partId: o.partId, x: o.x, y: o.y, w: o.w, h: o.h, z: o.z };
}

function partTops(model: DesignModel): Map<number, number> {
  const tops = new Map<number, number>();
  let top = 0;
  for (const part of model.parts) {
    tops.set(part.id, top);
    top += part.height;
  }
  return tops;
}

function sameAbsoluteRect(a: Rect, b: Rect, tops: Map<number, number>): boolean {
  return a.x === b.x && tops.get(a.partId)! + a.y === tops.get(b.partId)! + b.y && a.w === b.w && a.h === b.h;
}

function echoesEnabled(): boolean {
  if (typeof window === 'undefined') return false;
  const w = window as unknown as { RM_ECHOES?: boolean };
  if (w.RM_ECHOES === false) return false;
  if (w.RM_ECHOES === true) return true;
  try {
    const stored = localStorage.getItem('rmEchoes');
    if (stored === '0' || stored === 'false') return false;
    if (stored === '1' || stored === 'true') return true;
  } catch {
    // Some embedded contexts block localStorage; keep the default.
  }
  return true;
}
