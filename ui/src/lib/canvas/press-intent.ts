import type { ToolKind } from '../doc.svelte';

/** One decision for the pointer-down stream shared by Selecto and Moveable.
 * `toggle` is pending until release: movement turns it into a drag while a
 * stationary release commits the membership toggle. */
export type GestureIntent =
  | { kind: 'place' }
  | { kind: 'marquee' }
  | { kind: 'containment-marquee' }
  | { kind: 'toggle'; id: number }
  | { kind: 'drag'; id: number | null; select: boolean }
  | { kind: 'deselect' };

export type PressInput = {
  activeTool: ToolKind;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  objectId: number | null;
  objectIsTargeted: boolean;
  moveableChrome: boolean;
};

export function classifyPress(input: PressInput): GestureIntent {
  if (input.activeTool !== 'pointer') return { kind: 'place' };
  if (input.ctrlKey && !input.metaKey && !input.shiftKey) {
    return { kind: 'containment-marquee' };
  }
  if (input.objectId !== null && (input.shiftKey || input.metaKey)) {
    return { kind: 'toggle', id: input.objectId };
  }
  if (input.moveableChrome) return { kind: 'drag', id: input.objectId, select: false };
  if (input.objectId !== null) {
    return { kind: 'drag', id: input.objectId, select: !input.objectIsTargeted };
  }
  return { kind: 'marquee' };
}

