export type GuideBox = { x: number; y: number; w: number; h: number };
export type GuideCandidate = { id: number; box: GuideBox };
export type ActiveGuide = { axis: 'x' | 'y'; position: number };

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

