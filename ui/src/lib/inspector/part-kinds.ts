// The band-kind vocabulary the Inspector shows (ids match the engine's part
// kinds; legality rules live in ../part-rules). Shared by the root header
// (band subtitle) and the Part section's kind picker.

export const PART_KINDS: { id: string; label: string }[] = [
  { id: 'header', label: 'Header' },
  { id: 'body', label: 'Body' },
  { id: 'footer', label: 'Footer' },
  { id: 'subsummary', label: 'Sub-summary' },
  { id: 'grandsummary', label: 'Grand summary' },
];

export function partKindLabel(kind: string): string {
  return PART_KINDS.find((p) => p.id === kind)?.label ?? kind;
}
