import { useEffect, useRef, useState } from "react";
import { request, RequestError } from "./api/client";
import type { SnapshotPinState } from "./api/types";
import { loadBookmarks } from "./state";
import type { Bookmark } from "./state";

const STORAGE = "ferrum-atlas.bookmarks";
const OWNER = "ferrum-atlas.bookmark-owner";
type Retention = "checking" | "retained" | "failed";

export function retentionLabel(status?: Retention) {
  return status === "retained"
    ? "Snapshot retained"
    : status === "checking"
      ? "Retention pending"
      : "Snapshot not confirmed retained";
}

export function useReadingTrail(session: string) {
  const [bookmarks, setBookmarks] = useState(loadBookmarks);
  const current = useRef(bookmarks);
  const [state, setState] = useState<{
    session: string;
    statuses: Record<string, Retention>;
  }>({ session: "", statuses: {} });
  const [error, setError] = useState<Error>();
  const [pending, setPending] = useState(false);
  const busy = useRef(false);
  const operation = useRef(0);
  const controller = useRef<AbortController | undefined>(undefined);
  const statuses = state.session === session ? state.statuses : {};

  function save(next: Bookmark[]) {
    localStorage.setItem(STORAGE, JSON.stringify(next));
    current.current = next;
    setBookmarks(next);
    const ids = new Set(next.map((item) => item.id));
    setState((previous) => ({
      ...previous,
      statuses: Object.fromEntries(
        Object.entries(previous.statuses).filter(([id]) => ids.has(id)),
      ),
    }));
  }
  function ownedName(existing?: string) {
    let owner = localStorage.getItem(OWNER);
    if (!owner || !/^[a-f0-9-]{36}$/.test(owner)) {
      owner = crypto.randomUUID();
      localStorage.setItem(OWNER, owner);
    }
    const prefix = `browser-${owner}-`;
    return existing?.startsWith(prefix) &&
      /^[A-Za-z0-9_-]{1,128}$/.test(existing)
      ? existing
      : `${prefix}${crypto.randomUUID()}`;
  }
  function status(id: string, value: Retention) {
    setState((previous) => ({
      session,
      statuses: {
        ...(previous.session === session ? previous.statuses : {}),
        [id]: value,
      },
    }));
  }
  async function updatePin(
    item: Bookmark,
    retain: boolean,
    signal: AbortSignal,
  ) {
    const response = await request<SnapshotPinState>(
      `/snapshots/${encodeURIComponent(item.location.snapshot)}/${retain ? "pin" : "unpin"}`,
      AbortSignal.any([signal, AbortSignal.timeout(10_000)]),
      { context_id: item.location.context, name: item.retentionName },
    );
    if (
      response.snapshot_id !== item.location.snapshot ||
      response.context_id !== item.location.context ||
      response.name !== item.retentionName ||
      response.retained !== retain
    )
      throw new Error(
        "Snapshot retention response did not match the selection",
      );
  }
  async function run(action: (signal: AbortSignal) => Promise<void>) {
    if (busy.current || !session) return;
    const id = ++operation.current;
    const abort = new AbortController();
    controller.current = abort;
    busy.current = true;
    setPending(true);
    setError(undefined);
    try {
      await action(abort.signal);
    } catch (cause) {
      if (!abort.signal.aborted)
        setError(cause instanceof Error ? cause : new Error(String(cause)));
    } finally {
      if (operation.current === id) {
        busy.current = false;
        setPending(false);
      }
    }
  }
  async function retain(item: Bookmark, signal: AbortSignal) {
    status(item.id, "checking");
    try {
      await updatePin(item, true, signal);
      if (!signal.aborted) status(item.id, "retained");
    } catch (cause) {
      if (!signal.aborted) status(item.id, "failed");
      throw cause;
    }
  }
  async function recheck(signal: AbortSignal) {
    const names = new Set<string>();
    const items = current.current.map((item) => {
      let retentionName = ownedName(item.retentionName);
      if (names.has(retentionName)) retentionName = ownedName();
      names.add(retentionName);
      return { ...item, retentionName };
    });
    // Persist ownership before touching the server so retries keep the same pin.
    save(items);
    let failures = 0;
    let firstFailure = "";
    for (const item of items) {
      if (signal.aborted) return;
      try {
        await retain(item, signal);
      } catch (cause) {
        failures++;
        if (!firstFailure)
          firstFailure = cause instanceof Error ? cause.message : String(cause);
      }
    }
    if (failures)
      throw new Error(
        `${failures} reading-trail snapshot(s) not confirmed retained: ${firstFailure}`,
      );
  }
  useEffect(() => {
    setPending(false);
    if (session && current.current.length) void run(recheck);
    return () => {
      controller.current?.abort();
      operation.current++;
      busy.current = false;
    };
  }, [session]);

  return {
    bookmarks,
    statuses,
    pending,
    error,
    retry: () => run(recheck),
    add: (item: Bookmark) =>
      run(async (signal) => {
        if (current.current.length >= 40)
          throw new Error(
            "Reading trail is full (40 selections). Remove a pin first.",
          );
        const next = { ...item, retentionName: ownedName() };
        save([...current.current, next]);
        await retain(next, signal);
      }),
    remove: (item: Bookmark) =>
      run(async (signal) => {
        const name = ownedName(item.retentionName);
        if (name === item.retentionName) {
          status(item.id, "checking");
          try {
            await updatePin(item, false, signal);
          } catch (cause) {
            if (!signal.aborted) status(item.id, "failed");
            const absent =
              cause instanceof RequestError &&
              cause.status === 404 &&
              ["unknown_snapshot", "not_found"].includes(cause.code);
            if (!absent) throw cause;
          }
        }
        if (signal.aborted) return;
        status(item.id, "failed");
        save(current.current.filter((saved) => saved.id !== item.id));
      }),
    editNote: (id: string, note: string) => {
      if (busy.current) return;
      try {
        save(
          current.current.map((item) =>
            item.id === id ? { ...item, note } : item,
          ),
        );
      } catch {
        setError(
          new Error("Reading-trail storage unavailable; note was not saved"),
        );
      }
    },
  };
}
