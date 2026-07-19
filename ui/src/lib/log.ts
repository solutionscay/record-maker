// Structured diagnostic logging for Layout Mode (#62). The design canvas has a
// lot of moving parts — tool arming, click→model coordinate mapping, create
// round-trips, store mutations, selection, and the moveable drag/resize lifecycle
// — and when any of them is "off" the symptom (object lands in the wrong place,
// resize does nothing) is far from the cause. This logger makes the whole flow
// observable: every step emits ONE labelled, categorised, sequence-numbered line
// with its structured payload, so a repro reads top-to-bottom as a trace.
//
// Browser-only: under SSR / the headless test harness there is no `window`, so
// every call is a no-op and parity/doc-check output stays clean. On by default in
// the browser; silence it from the devtools console with `RM_LOG = false` (or
// `localStorage.rmLog = '0'`), re-enable with `RM_LOG = true`.

/** Log categories — one per stage of the Layout-Mode interaction flow. */
export type LogCat =
  | 'init' // controller / island lifecycle
  | 'tool' // tool palette arming
  | 'place' // click → model coordinate mapping → placement
  | 'create' // object/part create round-trips
  | 'persist' // geometry/props persistence
  | 'store' // document mutations (add/remove/geometry)
  | 'select' // selection changes
  | 'target' // moveable target reconciliation (id ↔ element)
  | 'drag' // moveable drag lifecycle
  | 'resize' // moveable resize lifecycle
  | 'rotate' // moveable rotate lifecycle
  | 'zoom' // canvas zoom
  | 'clipboard' // cut/copy/paste of objects
  | 'error'; // surfaced failures

const COLOR: Record<LogCat, string> = {
  init: '#6b7280',
  tool: '#7c3aed',
  place: '#0ea5e9',
  create: '#0891b2',
  persist: '#64748b',
  store: '#2563eb',
  select: '#16a34a',
  target: '#d97706',
  drag: '#db2777',
  resize: '#e11d48',
  rotate: '#c026d3',
  zoom: '#9333ea',
  clipboard: '#0d9488',
  error: '#dc2626',
};

/** One captured log line — also pushed to `window.__rmLogs` so a driver
 * (devtools, an automated monitor) can read the trace as structured data instead
 * of scraping console text. */
export interface LogEntry {
  seq: number;
  t: number;
  cat: LogCat;
  message: string;
  data?: Record<string, unknown>;
}

let seq = 0;
const start = typeof performance !== 'undefined' ? performance.now() : 0;

/** Append to the in-page ring buffer (browser only; capped so it can't grow
 * unbounded during a long session). */
function buffer(entry: LogEntry): void {
  if (typeof window === 'undefined') return;
  const w = window as unknown as { __rmLogs?: LogEntry[] };
  const buf = w.__rmLogs ?? (w.__rmLogs = []);
  buf.push(entry);
  if (buf.length > 2000) buf.shift();
}

export function layoutLogEnabled(): boolean {
  if (typeof window === 'undefined') return false;
  const w = window as unknown as { RM_LOG?: boolean };
  if (w.RM_LOG === false) return false;
  try {
    if (typeof localStorage !== 'undefined' && localStorage.getItem('rmLog') === '0') return false;
  } catch {
    /* localStorage may throw in some sandboxes — treat as enabled */
  }
  return true;
}

/** Emit one Layout-Mode log line: `[layout:cat] +Δms #seq message {data}`. The
 * `data` object is logged live (devtools shows the object), so expand it to
 * inspect coordinates, ids, element counts, etc. No-op outside the browser. */
export function llog(cat: LogCat, message: string, data?: Record<string, unknown>): void {
  if (!layoutLogEnabled()) return;
  seq += 1;
  const dt = (typeof performance !== 'undefined' ? performance.now() : 0) - start;
  buffer({ seq, t: Math.round(dt), cat, message, data });
  const head = `%c[layout:${cat}]%c +${dt.toFixed(0)}ms #${seq} ${message}`;
  const tagStyle = `color:#fff;background:${COLOR[cat]};padding:0 4px;border-radius:3px;font-weight:600`;
  const restStyle = 'color:inherit';
  if (data !== undefined) {
    // eslint-disable-next-line no-console
    console.debug(head, tagStyle, restStyle, data);
  } else {
    // eslint-disable-next-line no-console
    console.debug(head, tagStyle, restStyle);
  }
}

/** Log a failure with its category (always shown when logging is on). */
export function lerror(cat: LogCat, message: string, err: unknown): void {
  llog('error', `${cat}: ${message}`, { error: err instanceof Error ? err.message : String(err) });
}

// Announce the toggle once, so the trace is discoverable in a fresh console.
if (layoutLogEnabled()) {
  llog('init', 'Layout-Mode logging on — disable with RM_LOG = false');
}
