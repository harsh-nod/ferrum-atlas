import { useEffect, useRef, useState } from "react";

export const views = [
  "explore",
  "flow",
  "analysis",
  "changes",
  "evidence",
  "health",
] as const;
export type View = (typeof views)[number];
export type LocationState = {
  snapshot: string;
  context: string;
  definition: string;
  edge: string;
  view: View;
  depth: number;
  direction: "incoming" | "outgoing" | "both";
};
export type Bookmark = {
  id: string;
  label: string;
  location: LocationState;
  note: string;
  retentionName?: string;
};

export function parseLocation(search: string): LocationState {
  const query = new URLSearchParams(search);
  const view = query.get("view") as View;
  return {
    snapshot: query.get("snapshot") ?? "",
    context: query.get("context") ?? "",
    definition: query.get("definition") ?? "",
    edge: query.get("edge") ?? "",
    view: views.includes(view) ? view : "explore",
    depth: ["1", "2", "3"].includes(query.get("depth") ?? "")
      ? Number(query.get("depth"))
      : 2,
    direction:
      query.get("direction") === "incoming"
        ? "incoming"
        : query.get("direction") === "outgoing"
          ? "outgoing"
          : "both",
  };
}

export function locationUrl(state: LocationState): string {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(state))
    if (value) query.set(key, String(value));
  return `${window.location.pathname}?${query}`;
}

export function useLocationState() {
  const [location, setLocation] = useState(() =>
    parseLocation(window.location.search),
  );
  useEffect(() => {
    const update = () => setLocation(parseLocation(window.location.search));
    window.addEventListener("popstate", update);
    return () => window.removeEventListener("popstate", update);
  }, []);
  const navigate = (next: Partial<LocationState>, replace = false) => {
    setLocation((previous) => {
      const merged = { ...previous, ...next };
      const url = locationUrl(merged);
      if (url !== `${window.location.pathname}${window.location.search}`) {
        if (replace) window.history.replaceState(null, "", url);
        else window.history.pushState(null, "", url);
      }
      return merged;
    });
  };
  return [location, navigate] as const;
}

export function useResource<T>(
  key: string,
  load: (signal: AbortSignal) => Promise<T>,
) {
  const loader = useRef(load);
  loader.current = load;
  const [state, setState] = useState<{ key: string; data?: T; error?: Error }>({
    key: "",
  });
  useEffect(() => {
    if (!key) return;
    const controller = new AbortController();
    loader.current(controller.signal).then(
      (data) => {
        if (!controller.signal.aborted) setState({ key, data });
      },
      (error) => {
        if (!controller.signal.aborted)
          setState({
            key,
            error: error instanceof Error ? error : new Error(String(error)),
          });
      },
    );
    return () => controller.abort();
  }, [key]);
  const current = state.key === key ? state : { key };
  return {
    ...current,
    loading: Boolean(key) && !current.data && !current.error,
  };
}

export function byteToCharacter(text: string, byteOffset: number): number {
  const bytes = new TextEncoder().encode(text);
  // CodeMirror document positions use one code unit per normalized line break.
  return new TextDecoder("utf-8", { fatal: false })
    .decode(bytes.subarray(0, Math.max(0, Math.min(byteOffset, bytes.length))))
    .replace(/\r\n?/g, "\n").length;
}

export function loadBookmarks(): Bookmark[] {
  try {
    const value: unknown = JSON.parse(
      localStorage.getItem("ferrum-atlas.bookmarks") ?? "[]",
    );
    if (!Array.isArray(value)) return [];
    return value
      .filter(
        (item): item is Bookmark =>
          item &&
          typeof item.id === "string" &&
          typeof item.label === "string" &&
          typeof item.note === "string" &&
          item.location &&
          typeof item.location.snapshot === "string" &&
          typeof item.location.context === "string" &&
          typeof item.location.definition === "string" &&
          typeof item.location.edge === "string" &&
          [1, 2, 3].includes(item.location.depth) &&
          ["incoming", "outgoing", "both"].includes(item.location.direction) &&
          views.includes(item.location.view),
      )
      .slice(0, 40)
      .map((item) => ({
        ...item,
        retentionName:
          typeof item.retentionName === "string" &&
          /^[A-Za-z0-9_-]{1,128}$/.test(item.retentionName)
            ? item.retentionName
            : undefined,
      }));
  } catch {
    return [];
  }
}
