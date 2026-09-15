import type { ApiError, Snapshot, SnapshotPreparation } from "./types";

const TOKEN_KEY = "ferrum-atlas.session";
type Scope = { snapshot: string; context: string };
type Preparation = {
  controller: AbortController;
  ready: boolean;
  users: number;
  promise: Promise<void>;
};
const preparations = new Map<string, Preparation>();
let preparationSession = "";

export function readToken(): string {
  const fragment = new URLSearchParams(window.location.hash.slice(1));
  const token = fragment.get("token");
  if (token) {
    sessionStorage.setItem(TOKEN_KEY, token);
    fragment.delete("token");
    window.history.replaceState(
      null,
      "",
      `${window.location.pathname}${window.location.search}${fragment.size ? `#${fragment}` : ""}`,
    );
  }
  return sessionStorage.getItem(TOKEN_KEY) ?? "";
}

export function storeToken(token: string): void {
  sessionStorage.setItem(TOKEN_KEY, token.trim());
}

export class RequestError extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly correlationId = "",
    readonly status = 0,
  ) {
    super(message);
    this.name = "RequestError";
  }
}

export async function request<T>(
  path: string,
  signal: AbortSignal,
  body?: unknown,
): Promise<T> {
  signal.throwIfAborted();
  const token = readToken();
  if (preparationSession !== token) {
    for (const entry of preparations.values()) entry.controller.abort();
    preparations.clear();
    preparationSession = token;
  }
  let scopes: Scope[] = [];
  const waiting = new AbortController();
  try {
    const verification = AbortSignal.any([
      signal,
      waiting.signal,
      AbortSignal.timeout(30_000),
    ]);
    scopes = await requestScopes(path, body, verification, token);
    await Promise.all(
      scopes.map((scope) => prepare(scope, verification, token)),
    );
    signal.throwIfAborted();
    if (token !== readToken())
      throw new DOMException("Session changed", "AbortError");
    const result = await fetchJson<T>(path, signal, body, token);
    if (token !== readToken())
      throw new DOMException("Session changed", "AbortError");
    const response = record(result);
    if (
      record(response.work).deadline_reached === true ||
      record(record(response.analysis).envelope).deadline_reached === true ||
      record(record(response.comparison).envelope).deadline_reached === true
    )
      invalidateReadiness(scopes);
    return result;
  } catch (error) {
    // Readiness is a performance hint, never a lasting integrity guarantee.
    if (!signal.aborted && preparationSession === token)
      invalidateReadiness(scopes);
    throw error;
  } finally {
    waiting.abort();
  }
}

function invalidateReadiness(scopes: Scope[]) {
  // A successful partial response may follow eviction from the server cache.
  for (const scope of scopes) {
    const key = scopeKey(scope);
    if (preparations.get(key)?.ready) preparations.delete(key);
  }
}

function scopeKey(scope: Scope) {
  return JSON.stringify([scope.snapshot, scope.context]);
}

function record(value: unknown): Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

async function requestScopes(
  path: string,
  body: unknown,
  signal: AbortSignal,
  token: string,
): Promise<Scope[]> {
  const url = new URL(path, window.location.origin);
  const input = record(body);
  if (url.pathname === "/diff") {
    const ids = [
      ...new Set(
        [input.before, input.after].filter(
          (id): id is string => typeof id === "string",
        ),
      ),
    ];
    return Promise.all(
      ids.map(async (id) => {
        const snapshot = await fetchJson<Snapshot>(
          `/snapshots/${encodeURIComponent(id)}`,
          signal,
          undefined,
          token,
        );
        if (snapshot.id !== id || typeof snapshot.context?.id !== "string")
          throw new RequestError(
            "invalid_response",
            "Snapshot metadata did not match the requested revision",
          );
        return { snapshot: id, context: snapshot.context.id };
      }),
    );
  }
  if (url.pathname === "/traces/compare") {
    return ["before", "after"].flatMap((side) => {
      const snapshot = input[`${side}_snapshot_id`],
        context = input[`${side}_context_id`];
      return typeof snapshot === "string" && typeof context === "string"
        ? [{ snapshot, context }]
        : [];
    });
  }
  if (
    !/^\/(search|definitions|source|graph|flow|bodies|evidence|compiler|analysis|observations|queries|traces)(\/|$)/.test(
      url.pathname,
    )
  )
    return [];
  const query = record(input.graph ?? input.selection ?? input);
  const snapshot = url.searchParams.get("snapshot_id") ?? query.snapshot_id;
  const context = url.searchParams.get("context_id") ?? query.context_id;
  return typeof snapshot === "string" && typeof context === "string"
    ? [{ snapshot, context }]
    : [];
}

function prepare(
  scope: Scope,
  signal: AbortSignal,
  token: string,
): Promise<void> {
  signal.throwIfAborted();
  if (preparationSession !== token)
    throw new DOMException("Session changed", "AbortError");
  const key = scopeKey(scope);
  let entry = preparations.get(key);
  if (!entry) {
    if (preparations.size >= 16) {
      const idle = [...preparations].find(([, value]) => value.ready);
      if (!idle)
        throw new RequestError(
          "prepare_busy",
          "Snapshot verification capacity is busy",
        );
      preparations.delete(idle[0]);
    }
    entry = {
      controller: new AbortController(),
      ready: false,
      users: 0,
      promise: Promise.resolve(),
    };
    const created = entry;
    preparations.set(key, created);
    const bounded = AbortSignal.any([
      created.controller.signal,
      AbortSignal.timeout(30_000),
    ]);
    created.promise = fetchJson<SnapshotPreparation>(
      `/snapshots/${encodeURIComponent(scope.snapshot)}/prepare${params({ context_id: scope.context })}`,
      bounded,
      undefined,
      token,
      "POST",
    )
      .then((response) => {
        bounded.throwIfAborted();
        if (
          response.snapshot_id !== scope.snapshot ||
          response.context_id !== scope.context ||
          response.ready !== true
        )
          throw new RequestError(
            "invalid_response",
            "Snapshot verification did not match the requested scope",
          );
        created.ready = true;
      })
      .catch((error) => {
        if (preparations.get(key) === created) preparations.delete(key);
        throw error;
      });
  } else {
    preparations.delete(key);
    preparations.set(key, entry);
  }
  const shared = entry;
  shared.users++;
  return new Promise<void>((resolve, reject) => {
    let finished = false;
    const finish = (error?: unknown) => {
      if (finished) return;
      finished = true;
      signal.removeEventListener("abort", abort);
      shared.users--;
      // A cancelled pane must not cancel another pane's shared verification.
      queueMicrotask(() => {
        if (!shared.users && !shared.ready) {
          shared.controller.abort();
          if (preparations.get(key) === shared) preparations.delete(key);
        }
      });
      if (error !== undefined) reject(error);
      else resolve();
    };
    const abort = () => finish(signal.reason);
    signal.addEventListener("abort", abort, { once: true });
    if (signal.aborted) abort();
    shared.promise.then(() => finish(), finish);
  });
}

async function fetchJson<T>(
  path: string,
  signal: AbortSignal,
  body: unknown,
  token: string,
  method = body === undefined ? "GET" : "POST",
): Promise<T> {
  const response = await fetch(`/v1${path}`, {
    method,
    signal,
    headers: {
      Authorization: `Bearer ${token}`,
      ...(body === undefined ? {} : { "Content-Type": "application/json" }),
    },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  if (!response.ok) {
    let error: Partial<ApiError> = {};
    try {
      error = await response.json();
    } catch {
      /* A proxy may return non-JSON errors. */
    }
    throw new RequestError(
      error.code ?? `http_${response.status}`,
      error.message ?? `Request failed (${response.status})`,
      error.correlation_id,
      response.status,
    );
  }
  return response.json() as Promise<T>;
}

export function params(
  values: Record<string, string | number | undefined>,
): string {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(values))
    if (value !== undefined) query.set(key, String(value));
  return `?${query}`;
}
