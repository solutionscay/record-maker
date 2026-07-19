export type GuideBox = { x: number; y: number; w: number; h: number };
export type GuideCandidate = { id: number; box: GuideBox };
export type ActiveGuide = { axis: 'x' | 'y'; position: number };
type GuideIndexEntry = { position: number; candidate: GuideCandidate };
export type GuideIndex = { x: GuideIndexEntry[]; y: GuideIndexEntry[] };

export type SnapResult = {
  box: GuideBox;
  guides: ActiveGuide[];
};

type AxisMatch = { offset: number; position: number; rank: number };

function anchors(start: number, size: number): number[] {
  return [start, start + size / 2, start + size];
}

function bestAxisMatch(
  active: number[],
  candidates: number[],
  threshold: number,
): AxisMatch | null {
  let best: AxisMatch | null = null;
  for (let activeIndex = 0; activeIndex < active.length; activeIndex++) {
    for (let candidateIndex = 0; candidateIndex < candidates.length; candidateIndex++) {
      const offset = candidates[candidateIndex] - active[activeIndex];
      if (Math.abs(offset) > threshold) continue;
      // Stable ties prefer edges over centers, then earlier canvas geometry.
      const edgeRank = (activeIndex === 1 ? 2 : 0) + (candidateIndex === 1 ? 1 : 0);
      const rank = edgeRank * 1_000_000 + activeIndex * 10_000 + candidateIndex;
      if (
        !best ||
        Math.abs(offset) < Math.abs(best.offset) ||
        (Math.abs(offset) === Math.abs(best.offset) && rank < best.rank)
      ) {
        best = { offset, position: candidates[candidateIndex], rank };
      }
    }
  }
  return best;
}

function candidateAnchors(candidates: GuideCandidate[], axis: 'x' | 'y'): number[] {
  return candidates
    .flatMap(({ box }) => axis === 'x' ? anchors(box.x, box.w) : anchors(box.y, box.h))
    .sort((a, b) => a - b);
}

/** Immutable per-gesture spatial index. Each candidate contributes its two
 * edges and center on both axes; pointer samples binary-search only the ranges
 * surrounding the active anchors instead of scanning the canvas. */
export function buildGuideIndex(candidates: GuideCandidate[]): GuideIndex {
  const entries = (axis: 'x' | 'y') => candidates
    .flatMap((candidate) => {
      const { box } = candidate;
      return (axis === 'x' ? anchors(box.x, box.w) : anchors(box.y, box.h))
        .map((position) => ({ position, candidate }));
    })
    .sort((a, b) => a.position - b.position || a.candidate.id - b.candidate.id);
  return { x: entries('x'), y: entries('y') };
}

function lowerBound(entries: GuideIndexEntry[], value: number): number {
  let low = 0;
  let high = entries.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (entries[middle].position < value) low = middle + 1;
    else high = middle;
  }
  return low;
}

/** Return only candidates capable of producing a guide for this exact move or
 * active resize edge. A candidate matching either axis is retained because the
 * authoritative resolver chooses x/y independently from the same set. */
export function candidatesNearGuideBox(
  index: GuideIndex,
  box: GuideBox,
  threshold: number,
  direction?: readonly number[],
): GuideCandidate[] {
  const selected = new Map<number, GuideCandidate>();
  const sourceAnchors = (axis: 'x' | 'y'): number[] => {
    const start = axis === 'x' ? box.x : box.y;
    const size = axis === 'x' ? box.w : box.h;
    if (!direction) return anchors(start, size);
    const dir = Math.sign(direction[axis === 'x' ? 0 : 1] ?? 0);
    return dir === 0 ? [] : [dir < 0 ? start : start + size];
  };
  for (const axis of ['x', 'y'] as const) {
    const entries = index[axis];
    for (const source of sourceAnchors(axis)) {
      const limit = source + threshold;
      for (let cursor = lowerBound(entries, source - threshold); cursor < entries.length; cursor += 1) {
        const entry = entries[cursor];
        if (entry.position > limit) break;
        selected.set(entry.candidate.id, entry.candidate);
      }
    }
  }
  return [...selected.values()];
}

/** Snap a translated single/group union. One common x/y offset preserves every
 * member's relative geometry. */
export function resolveMoveGuides(
  box: GuideBox,
  candidates: GuideCandidate[],
  threshold: number,
): SnapResult {
  const x = bestAxisMatch(anchors(box.x, box.w), candidateAnchors(candidates, 'x'), threshold);
  const y = bestAxisMatch(anchors(box.y, box.h), candidateAnchors(candidates, 'y'), threshold);
  return {
    box: { ...box, x: box.x + (x?.offset ?? 0), y: box.y + (y?.offset ?? 0) },
    guides: [
      ...(x ? [{ axis: 'x' as const, position: x.position }] : []),
      ...(y ? [{ axis: 'y' as const, position: y.position }] : []),
    ],
  };
}

/** Snap only the resize edges that the active handle owns. */
export function resolveResizeGuides(
  box: GuideBox,
  direction: readonly number[],
  candidates: GuideCandidate[],
  threshold: number,
): SnapResult {
  const next = { ...box };
  const guides: ActiveGuide[] = [];
  const dirX = Math.sign(direction[0] ?? 0);
  const dirY = Math.sign(direction[1] ?? 0);
  if (dirX !== 0) {
    const edge = dirX < 0 ? box.x : box.x + box.w;
    const match = bestAxisMatch([edge], candidateAnchors(candidates, 'x'), threshold);
    if (match) {
      if (dirX < 0) {
        next.x += match.offset;
        next.w -= match.offset;
      } else next.w += match.offset;
      guides.push({ axis: 'x', position: match.position });
    }
  }
  if (dirY !== 0) {
    const edge = dirY < 0 ? box.y : box.y + box.h;
    const match = bestAxisMatch([edge], candidateAnchors(candidates, 'y'), threshold);
    if (match) {
      if (dirY < 0) {
        next.y += match.offset;
        next.h -= match.offset;
      } else next.h += match.offset;
      guides.push({ axis: 'y', position: match.position });
    }
  }
  next.w = Math.max(1, next.w);
  next.h = Math.max(1, next.h);
  return { box: next, guides };
}

export function unionGuideBoxes(boxes: GuideBox[]): GuideBox | null {
  if (boxes.length === 0) return null;
  const x = Math.min(...boxes.map((box) => box.x));
  const y = Math.min(...boxes.map((box) => box.y));
  const right = Math.max(...boxes.map((box) => box.x + box.w));
  const bottom = Math.max(...boxes.map((box) => box.y + box.h));
  return { x, y, w: right - x, h: bottom - y };
}
