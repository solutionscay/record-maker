// The field-kind vocabulary shared by both sub-apps (#132): the six logical
// kinds the engine understands (`FieldKind`, model.rs) and the one
// kind -> icon/label table — previously duplicated between the schema builder's
// types.ts and Layout Mode's FieldSelect. The schema builder never exposes
// physical/SQL types — only these.

export type FieldKind = 'text' | 'number' | 'date' | 'time' | 'timestamp' | 'bool';

/** Ordered kind choices for the type picker: `kind` is what we POST, `label` is
 * the human name, `icon` names the shared sprite symbol (`#icon-type-<kind>`). */
export const FIELD_KINDS: { kind: FieldKind; label: string; icon: string }[] = [
  { kind: 'text', label: 'Text', icon: 'type-text' },
  { kind: 'number', label: 'Number', icon: 'type-number' },
  { kind: 'date', label: 'Date', icon: 'type-date' },
  { kind: 'time', label: 'Time', icon: 'type-time' },
  { kind: 'timestamp', label: 'Timestamp', icon: 'type-timestamp' },
  { kind: 'bool', label: 'Boolean', icon: 'type-bool' },
];

/** Human label for a kind string (falls back to the raw string if unknown). */
export function kindLabel(kind: string): string {
  return FIELD_KINDS.find((k) => k.kind === kind)?.label ?? kind;
}

/** Sprite symbol name for a kind string. The fallback for an unknown kind is
 * caller-chosen: the schema builder shows the generic `field` glyph, Layout
 * Mode's picker shows `type-text` — both preserved from before the merge. */
export function kindIcon(kind: string, fallback = 'field'): string {
  return FIELD_KINDS.find((k) => k.kind === kind)?.icon ?? fallback;
}
