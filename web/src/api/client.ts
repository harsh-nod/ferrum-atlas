import type { ApiError } from "./types";

const TOKEN_KEY = "ferrum-atlas.session";

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
  const response = await fetch(`/v1${path}`, {
    method: body === undefined ? "GET" : "POST",
    signal,
    headers: {
      Authorization: `Bearer ${readToken()}`,
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
