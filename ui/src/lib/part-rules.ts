// Part-structure rules — the ONE UI transcription of the engine's band
// legality rules (crates/engine/src/layout.rs): a form allows only
// header/body/footer; header/body/footer are singletons; a layout takes at most
// one leading and one trailing grand summary around the body. Both the rail's
// add-band combo (RailTools) and the Band inspector's kind select (Inspector)
// gate through these predicates; the engine remains the enforcing authority —
// these only decide what the controls offer.

/** The slice of a part the rules need (structurally matches `PartDoc`). */
export interface PartLike {
  id: number;
  kind: string;
  position: number;
}

/** Header/body/footer exist at most once per layout. */
export function isSingletonPartKind(kind: string): boolean {
  return kind === 'header' || kind === 'body' || kind === 'footer';
}

/** Whether `kind` is offered at all on a layout of the given Browse view — a
 * form is a single-record view, so sub/grand summaries are List/Table only
 * (Issue 3). */
export function partKindAllowedInView(view: string, kind: string): boolean {
  return view !== 'form' || (kind !== 'subsummary' && kind !== 'grandsummary');
}

/** Whether a new band of `kind` may be added to a layout with `parts`. */
export function canAddPartKind(view: string, parts: readonly PartLike[], kind: string): boolean {
  if (!partKindAllowedInView(view, kind)) return false;
  if (isSingletonPartKind(kind) && parts.some((p) => p.kind === kind)) return false;
  if (kind === 'grandsummary') {
    // At most one grand summary on each side of the body.
    const body = parts.find((p) => p.kind === 'body');
    if (!body) return false;
    const leading = parts.some((p) => p.kind === 'grandsummary' && p.position < body.position);
    const trailing = parts.some((p) => p.kind === 'grandsummary' && p.position > body.position);
    return !(leading && trailing);
  }
  return true;
}

/** Whether an existing band may change kind to `kind`. Its current kind is
 * always allowed, so an existing band never shows an illegal blank. */
export function canSetPartKind(
  view: string,
  parts: readonly PartLike[],
  part: PartLike,
  kind: string,
): boolean {
  if (part.kind === kind) return true;
  if (!partKindAllowedInView(view, kind)) return false;
  if (part.kind === 'body') return false;
  // Header/footer are structural anchors (top/bottom) — they can't become
  // summaries, which would strand a summary above the header or below the
  // footer (mirrors the engine's move_part rules).
  if (
    (part.kind === 'header' || part.kind === 'footer') &&
    (kind === 'subsummary' || kind === 'grandsummary')
  ) {
    return false;
  }
  if (isSingletonPartKind(kind) && parts.some((p) => p.id !== part.id && p.kind === kind)) {
    return false;
  }
  if (kind === 'grandsummary') {
    const body = parts.find((p) => p.kind === 'body');
    if (!body) return false;
    const wantsTrailing = part.position > body.position;
    return !parts.some(
      (p) =>
        p.id !== part.id && p.kind === 'grandsummary' && (p.position > body.position) === wantsTrailing,
    );
  }
  return true;
}
