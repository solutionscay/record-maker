// Display-only value formatting (#77 number/Boolean, #78 date/time) — a faithful
// TypeScript mirror of the server's `crates/server/src/format.rs`. The server owns
// the real render (Browse + canvas); this port drives ONLY the inspector's live
// "Sample" preview so the designer sees the effect of the `format` bag while they
// edit it. Keep it byte-for-byte in step with format.rs.
//
// Japanese / Kanji numeral + date display is intentionally out of scope.

export interface Formatted {
  text: string;
  color: string | null;
}

type Bag = Record<string, unknown>;

function asBag(v: unknown): Bag | undefined {
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Bag) : undefined;
}

// ---- small typed readers over the (untrusted) format bag ----

function getStr(v: Bag, key: string, def: string): string {
  const x = v[key];
  return typeof x === 'string' ? x : def;
}
function getStrOpt(v: Bag, key: string): string | undefined {
  const x = v[key];
  return typeof x === 'string' ? x : undefined;
}
function getBool(v: Bag, key: string, def: boolean): boolean {
  const x = v[key];
  return typeof x === 'boolean' ? x : def;
}
function getI64(v: Bag, key: string, def: number): number {
  const x = v[key];
  return typeof x === 'number' && Number.isInteger(x) ? x : def;
}

/** Format `raw` for display given the object's `format` spec and bound field kind.
 * Returns the raw value untouched whenever no formatting applies. */
export function formatValue(raw: string, format: Bag | undefined, kind: string): Formatted {
  switch (kind) {
    case 'number':
    case 'bool':
      return formatNumber(raw, format);
    case 'date':
      return formatDate(raw, format);
    case 'time':
      return formatTime(raw, format);
    case 'timestamp':
      return formatTimestamp(raw, format);
    default:
      return { text: raw, color: null };
  }
}

// ---------------------------------------------------------------------------
// Number / Boolean (#77)
// ---------------------------------------------------------------------------

function formatNumber(raw: string, f: Bag | undefined): Formatted {
  if (!f) return { text: raw, color: null };
  switch (getStr(f, 'mode', 'general')) {
    case 'asEntered':
      return { text: raw, color: null };
    case 'boolean': {
      const n = Number(raw.trim());
      const nonZero = raw.trim() !== '' && Number.isFinite(n) && n !== 0;
      return { text: nonZero ? getStr(f, 'booleanNonZero', '') : getStr(f, 'booleanZero', ''), color: null };
    }
    case 'decimal':
      return formatNumeric(raw, f, true);
    default:
      return formatNumeric(raw, f, false);
  }
}

function formatNumeric(raw: string, f: Bag, decimalMode: boolean): Formatted {
  const trimmed = raw.trim();
  const num = Number(trimmed);
  if (trimmed === '' || !Number.isFinite(num)) return { text: raw, color: null };

  if (decimalMode && getBool(f, 'hideZero', false) && num === 0) return { text: '', color: null };

  const decimals: number | null =
    decimalMode && getBool(f, 'fixedDecimals', false)
      ? Math.min(Math.max(getI64(f, 'decimalDigits', 2), 0), 15)
      : null;
  const decSep = getStr(f, 'decimalSeparator', '.');
  const thouSep = getStr(f, 'thousandsSeparator', '');
  const negStyle = getStr(f, 'negativeStyle', 'minus');
  const currency = decimalMode ? getStr(f, 'currency', 'none') : 'none';
  const symbol = getStr(f, 'currencySymbol', '');

  const negative = num < 0;
  const abs = Math.abs(num);
  const base = decimals !== null ? abs.toFixed(decimals) : String(abs);

  const dot = base.indexOf('.');
  const intPart = dot === -1 ? base : base.slice(0, dot);
  const fracPart = dot === -1 ? null : base.slice(dot + 1);
  const grouped = thouSep === '' ? intPart : groupThousands(intPart, thouSep);
  let digits = grouped;
  if (fracPart !== null) digits += decSep + fracPart;

  let out = digits;
  if (currency === 'inside') out = `${symbol}${out}`;
  if (negative) out = negStyle === 'parens' ? `(${out})` : `-${out}`;
  if (currency === 'leading') out = `${symbol}${out}`;

  const color = negative ? getStrOpt(f, 'negativeColor') ?? null : null;
  return { text: out, color };
}

function groupThousands(intDigits: string, sep: string): string {
  const len = intDigits.length;
  let out = '';
  for (let i = 0; i < len; i++) {
    if (i > 0 && (len - i) % 3 === 0) out += sep;
    out += intDigits[i];
  }
  return out;
}

// ---------------------------------------------------------------------------
// Date (#78)
// ---------------------------------------------------------------------------

interface DateParts {
  y: number;
  mo: number;
  d: number;
}

function parseDate(raw: string): DateParts | null {
  const date = raw.trim().split(/[T ]/)[0] ?? '';
  const parts = date.split('-');
  if (parts.length < 3) return null;
  const y = Number(parts[0]);
  const mo = Number(parts[1]);
  const d = Number(parts[2]);
  if (![y, mo, d].every((n) => Number.isInteger(n))) return null;
  return { y, mo, d };
}

const MONTHS_LONG = [
  'January', 'February', 'March', 'April', 'May', 'June',
  'July', 'August', 'September', 'October', 'November', 'December',
];
const MONTHS_SHORT = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const WEEKDAYS_LONG = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
const WEEKDAYS_SHORT = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

/** Day of week, 0 = Sunday .. 6 = Saturday (Sakamoto's algorithm). */
function weekday(y: number, m: number, d: number): number {
  const T = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
  if (m < 3) y -= 1;
  const w = (y + Math.floor(y / 4) - Math.floor(y / 100) + Math.floor(y / 400) + T[m - 1] + d) % 7;
  return ((w % 7) + 7) % 7;
}

function pad2(n: number): string {
  return String(Math.abs(n)).padStart(2, '0');
}

function formatDate(raw: string, f: Bag | undefined): Formatted {
  if (!f) return { text: raw, color: null };
  const mode = getStr(f, 'mode', 'asEntered');
  if (mode === 'asEntered') return { text: raw, color: null };
  const dp = parseDate(raw);
  if (!dp) return { text: raw, color: null };
  if (mode === 'predefined') return { text: renderPredefinedDate(dp, f), color: null };
  if (mode === 'custom') return { text: renderCustomDate(dp, f), color: null };
  return { text: raw, color: null };
}

function renderPredefinedDate(dp: DateParts, f: Bag): string {
  const name = getStr(f, 'predefined', 'mm/dd/yyyy');
  const defaultSep = name.includes('-') ? '-' : '/';
  const sep = getStr(f, 'dateSeparator', defaultSep);
  const mm = pad2(dp.mo);
  const dd = pad2(dp.d);
  const yy = pad2(((dp.y % 100) + 100) % 100);
  const yyyy = String(dp.y);
  switch (name) {
    case 'mm/dd/yy':
      return `${mm}${sep}${dd}${sep}${yy}`;
    case 'dd/mm/yy':
      return `${dd}${sep}${mm}${sep}${yy}`;
    case 'dd/mm/yyyy':
      return `${dd}${sep}${mm}${sep}${yyyy}`;
    case 'yyyy-mm-dd':
      return `${yyyy}${sep}${mm}${sep}${dd}`;
    default:
      return `${mm}${sep}${dd}${sep}${yyyy}`;
  }
}

function renderCustomDate(dp: DateParts, f: Bag): string {
  const comps = Array.isArray(f.components) ? (f.components as unknown[]) : null;
  if (!comps) return '';
  let out = '';
  for (const raw of comps) {
    const c = asBag(raw);
    if (!c) continue;
    out += getStr(c, 'leading', '');
    switch (getStr(c, 'type', '')) {
      case 'dayOfWeek': {
        const idx = weekday(dp.y, dp.mo, dp.d);
        out += getStr(c, 'style', 'long') === 'short' ? WEEKDAYS_SHORT[idx] : WEEKDAYS_LONG[idx];
        break;
      }
      case 'month':
        switch (getStr(c, 'style', 'number')) {
          case 'long':
            out += monthName(dp.mo, MONTHS_LONG);
            break;
          case 'short':
            out += monthName(dp.mo, MONTHS_SHORT);
            break;
          default:
            out += getBool(c, 'leadingZero', true) ? pad2(dp.mo) : String(dp.mo);
        }
        break;
      case 'day':
        out += getBool(c, 'leadingZero', false) ? pad2(dp.d) : String(dp.d);
        break;
      case 'year':
        out += getStr(c, 'style', 'full') === 'short' ? pad2(((dp.y % 100) + 100) % 100) : String(dp.y);
        break;
      default:
        break;
    }
  }
  return out;
}

function monthName(mo: number, names: string[]): string {
  const idx = Math.min(Math.max(mo, 1), 12) - 1;
  return names[idx];
}

// ---------------------------------------------------------------------------
// Time (#78)
// ---------------------------------------------------------------------------

interface TimeParts {
  h: number;
  mi: number;
  s: number;
}

function parseTime(raw: string): TimeParts | null {
  const segs = raw.trim().split(/[T ]/);
  const t = segs[segs.length - 1] ?? '';
  const parts = t.split(':');
  if (parts.length < 2) return null;
  const h = Number(parts[0]);
  const mi = Number(parts[1]);
  const s = parts.length > 2 ? Number(parts[2]) : 0;
  if (![h, mi, s].every((n) => Number.isInteger(n))) return null;
  return { h, mi, s };
}

function formatTime(raw: string, f: Bag | undefined): Formatted {
  if (!f) return { text: raw, color: null };
  const mode = getStr(f, 'mode', 'asEntered');
  if (mode === 'asEntered') return { text: raw, color: null };
  const tp = parseTime(raw);
  if (!tp) return { text: raw, color: null };
  let showSeconds: boolean;
  if (mode === 'predefined') showSeconds = getStr(f, 'predefined', 'hh:mm:ss').includes('ss');
  else if (mode === 'custom') showSeconds = getBool(f, 'showSeconds', true);
  else return { text: raw, color: null };
  return { text: renderTime(tp, f, showSeconds), color: null };
}

function renderTime(tp: TimeParts, f: Bag, showSeconds: boolean): string {
  const h24 = getBool(f, 'hours24', true);
  const sep = getStr(f, 'timeSeparator', ':');
  const hZero = getBool(f, 'hoursLeadingZero', true);
  const msZero = getBool(f, 'minutesSecondsLeadingZero', true);

  let dispH: number;
  let am: boolean | null;
  if (h24) {
    dispH = tp.h;
    am = null;
  } else {
    am = tp.h < 12;
    dispH = tp.h % 12;
    if (dispH === 0) dispH = 12;
  }

  const hstr = hZero ? pad2(dispH) : String(dispH);
  const mstr = msZero ? pad2(tp.mi) : String(tp.mi);
  let out = `${hstr}${sep}${mstr}`;
  if (showSeconds) {
    const sstr = msZero ? pad2(tp.s) : String(tp.s);
    out += `${sep}${sstr}`;
  }

  if (am !== null) {
    const placement = getStr(f, 'amPmPlacement', 'after');
    if (placement !== 'none') {
      const label = am ? getStr(f, 'amLabel', 'AM') : getStr(f, 'pmLabel', 'PM');
      out = placement === 'before' ? `${label} ${out}` : `${out} ${label}`;
    }
  }
  return out;
}

// ---------------------------------------------------------------------------
// Timestamp (#78) — the date panel + the time panel together.
// ---------------------------------------------------------------------------

function formatTimestamp(raw: string, f: Bag | undefined): Formatted {
  if (!f) return { text: raw, color: null };
  const dateSpec = asBag(f.date);
  const timeSpec = asBag(f.time);
  if (dateSpec === undefined && timeSpec === undefined) return { text: raw, color: null };
  const trimmed = raw.trim();
  const splitIdx = trimmed.search(/[T ]/);
  const datePart = splitIdx === -1 ? trimmed : trimmed.slice(0, splitIdx);
  const timePart = splitIdx === -1 ? '' : trimmed.slice(splitIdx + 1);
  const sep = getStr(f, 'separator', ' ');
  const dateStr = formatDate(datePart, dateSpec).text;
  const timeStr = formatTime(timePart, timeSpec).text;
  return { text: `${dateStr}${sep}${timeStr}`, color: null };
}
