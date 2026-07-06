// Shared HTTP helpers for both sub-apps (#132). One fetch/throw implementation
// instead of the two persist.ts copies (Layout Mode + schema builder) plus the
// eight inline fetch blocks Layout Mode re-typed. Every failure throws the typed
// HttpError below, so callers can show the server's actual message (the schema
// endpoints return CONFLICT/BAD_REQUEST with a human-readable string) instead of
// an opaque status code.

/** A failed request — carries the server's status + message body. The message
 * falls back to `HTTP <status>` when the server sent no body, so existing
 * `e.message` consumers never regress to an empty string. */
export class HttpError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message || `HTTP ${status}`);
    this.name = 'HttpError';
    this.status = status;
  }
}

/** Optional per-call logging hook — Layout Mode injects its `llog` channel here
 * (see ui/src/lib/persist.ts); the schema builder passes nothing. */
export interface HttpLog {
  start?: (url: string, body: unknown) => void;
  ok?: (url: string, response: unknown) => void;
  fail?: (url: string, status: number) => void;
}

async function fail(r: Response, url: string, log?: HttpLog): Promise<never> {
  log?.fail?.(url, r.status);
  throw new HttpError(r.status, await r.text().catch(() => ''));
}

export async function getJson<T>(url: string, log?: HttpLog): Promise<T> {
  const r = await fetch(url);
  if (!r.ok) await fail(r, url, log);
  return (await r.json()) as T;
}

export async function postJson<T>(url: string, body?: unknown, log?: HttpLog): Promise<T> {
  log?.start?.(url, body);
  const r = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body ?? {}),
  });
  if (!r.ok) await fail(r, url, log);
  const json = (await r.json()) as T;
  log?.ok?.(url, json);
  return json;
}

/** POST that returns no JSON body (the delete/geometry endpoints just 200/OK).
 * With `body` undefined the request is sent bare — no content-type header, no
 * body — matching what the Layout Mode call sites always sent. */
export async function postVoid(url: string, body?: unknown, log?: HttpLog): Promise<void> {
  log?.start?.(url, body);
  const r = await fetch(
    url,
    body === undefined
      ? { method: 'POST' }
      : { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) },
  );
  if (!r.ok) await fail(r, url, log);
  log?.ok?.(url, undefined);
}
