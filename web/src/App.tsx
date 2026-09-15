import { useEffect, useRef, useState } from "react";
import type { CSSProperties, FormEvent } from "react";
import {
  Activity,
  ArrowLeft,
  ArrowRight,
  Bookmark as BookmarkIcon,
  Check,
  ChartNoAxesCombined,
  ChevronRight,
  Code2,
  Download,
  FileCode2,
  Folder,
  GitBranch,
  GitCompareArrows,
  Link2,
  ListTree,
  LockKeyhole,
  Network,
  PanelRight,
  Search,
  ShieldCheck,
  Workflow,
  X,
} from "lucide-react";
import { params, readToken, request, storeToken } from "./api/client";
import type {
  Capabilities,
  Definition,
  DefinitionDetail,
  Direction,
  Evidence,
  GraphResponse,
  QueryResponse,
  Relation,
  Snapshot,
  Span,
} from "./api/types";
import {
  loadBookmarks,
  locationUrl,
  useLocationState,
  useResource,
  views,
} from "./state";
import type { Bookmark } from "./state";
import {
  CoverageNotice,
  ErrorNotice,
  IconButton,
  Loading,
  ResizeHandle,
} from "./components/common";
import { SourcePane } from "./components/SourcePane";
import { GraphPane } from "./components/GraphPane";
import { AnalysisView } from "./components/AnalysisViews";
import {
  ChangesView,
  EvidenceList,
  EvidenceView,
  FlowView,
  HealthView,
} from "./components/ReviewViews";

const tabIcons = {
  explore: Network,
  flow: Workflow,
  analysis: ChartNoAxesCombined,
  changes: GitCompareArrows,
  evidence: ShieldCheck,
  health: Activity,
};

export function App() {
  const [token, setToken] = useState(readToken);
  const [tokenInput, setTokenInput] = useState("");
  const [session, setSession] = useState(0);
  const [location, navigate] = useLocationState();
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState<string | undefined>();
  const { direction, depth } = location;
  const [drawer, setDrawer] = useState<"scope" | "inspector" | "">("");
  const [mobilePanel, setMobilePanel] = useState<"source" | "graph">("source");
  const [scopeWidth, setScopeWidth] = useState(220);
  const [sourceHeight, setSourceHeight] = useState(320);
  const [bookmarks, setBookmarks] = useState<Bookmark[]>(loadBookmarks);
  const [note, setNote] = useState("");
  const [notification, setNotification] = useState("");
  const [flowSpan, setFlowSpan] = useState<Span>();
  const searchRef = useRef<HTMLInputElement>(null);
  const snapshotList = useResource<Snapshot[]>(
    token ? `snapshots:${session}` : "",
    (signal) => request("/snapshots", signal),
  );
  const capabilities = useResource<Capabilities>(
    token ? `capabilities:${session}` : "",
    (signal) => request("/capabilities", signal),
  );
  const snapshots = [...(snapshotList.data ?? [])].sort(
    (a, b) => Number(b.created_at) - Number(a.created_at),
  );
  const snapshot = snapshots.find((item) => item.id === location.snapshot);
  const pin =
    snapshot && (!location.context || location.context === snapshot.context.id)
      ? params({ snapshot_id: snapshot.id, context_id: snapshot.context.id })
      : "";
  const definition = useResource<DefinitionDetail>(
    pin && location.definition
      ? `definition:${pin}:${location.definition}:${session}`
      : "",
    (signal) =>
      request(
        `/definitions/${encodeURIComponent(location.definition)}${pin}`,
        signal,
      ),
  );
  const definitions = useResource<QueryResponse<Definition>>(
    pin ? `search:${pin}:${query}:${cursor ?? ""}:${session}` : "",
    (signal) =>
      request(
        `/search${params({ snapshot_id: snapshot!.id, context_id: snapshot!.context.id, q: query, limit: 50, cursor })}`,
        signal,
      ),
  );
  const graph = useResource<GraphResponse>(
    pin && location.definition
      ? `graph:${pin}:${location.definition}:${direction}:${depth}:${session}`
      : "",
    (signal) =>
      request("/graph/neighborhood", signal, {
        snapshot_id: snapshot!.id,
        context_id: snapshot!.context.id,
        definition_id: location.definition,
        direction,
        depth,
        max_nodes: 200,
        max_edges: 500,
      }),
  );
  const edge = graph.data?.edges.find((item) => item.id === location.edge);
  const edgeEvidence = useResource<Evidence>(
    edge ? `evidence:${pin}:${edge.evidence_id}:${session}` : "",
    (signal) =>
      request(
        `/evidence/${encodeURIComponent(edge!.evidence_id)}${pin}`,
        signal,
      ),
  );
  const evidence = location.edge
    ? edgeEvidence.data
      ? [edgeEvidence.data]
      : []
    : (definition.data?.evidence ?? []);
  const activeDefinition = definition.data?.definition;
  const selectedSpan = location.edge
    ? edge?.span
    : (flowSpan ?? activeDefinition?.span);
  const missingEdge = Boolean(location.edge && graph.data && !edge);
  const evidenceLoading = location.edge
    ? graph.loading || edgeEvidence.loading
    : definition.loading;
  const evidenceError = location.edge
    ? (graph.error ?? edgeEvidence.error)
    : definition.error;
  const evidenceSelected = location.edge
    ? Boolean(edge)
    : Boolean(activeDefinition);
  const activeBookmark = bookmarks.find(
    (item) =>
      item.location.snapshot === location.snapshot &&
      item.location.context === location.context &&
      item.location.definition === location.definition &&
      item.location.edge === location.edge &&
      item.location.view === location.view &&
      item.location.depth === location.depth &&
      item.location.direction === location.direction,
  );

  useEffect(() => {
    if (!location.snapshot && snapshots[0])
      navigate(
        { snapshot: snapshots[0].id, context: snapshots[0].context.id },
        true,
      );
    else if (snapshot && !location.context)
      navigate({ context: snapshot.context.id }, true);
  }, [snapshotList.data, location.snapshot, location.context]);
  useEffect(() => {
    if (!location.definition && definitions.data?.items.length) {
      const initial =
        definitions.data.items.find((item) => item.name === "main") ??
        definitions.data.items.find((item) => item.name === "entry") ??
        definitions.data.items[0];
      navigate({ definition: initial.id }, true);
    }
  }, [definitions.data, location.definition]);
  useEffect(() => {
    const timer = setTimeout(() => {
      setQuery(search);
      setCursor(undefined);
    }, 180);
    return () => clearTimeout(timer);
  }, [search]);
  useEffect(() => {
    setCursor(undefined);
    setFlowSpan(undefined);
    setNote("");
  }, [location.snapshot, location.definition]);
  useEffect(() => {
    const timer = setTimeout(() => setNotification(""), 3000);
    return () => clearTimeout(timer);
  }, [notification]);
  useEffect(() => {
    const keyboard = (event: KeyboardEvent) => {
      if (event.key === "Escape") setDrawer("");
      if ((event.metaKey || event.ctrlKey) && event.key === "k") {
        event.preventDefault();
        searchRef.current?.focus();
      }
    };
    window.addEventListener("keydown", keyboard);
    return () => window.removeEventListener("keydown", keyboard);
  }, []);
  useEffect(() => {
    if (!drawer) return;
    const media = window.matchMedia(
      drawer === "scope" ? "(max-width: 760px)" : "(max-width: 1150px)",
    );
    if (!media.matches) return;
    const panel = document.querySelector<HTMLElement>(
      drawer === "scope" ? ".scope-panel" : ".inspector-panel",
    );
    if (!panel) return;
    const previous = document.activeElement as HTMLElement | null;
    const background = Array.from(
      document.querySelectorAll<HTMLElement>(
        ".app-header, .view-tabs, .main-workspace, .status-bar, " +
          (drawer === "scope" ? ".inspector-panel" : ".scope-panel"),
      ),
    ).map((element) => ({ element, inert: element.inert }));
    background.forEach(({ element }) => {
      element.inert = true;
    });
    panel.setAttribute("role", "dialog");
    panel.setAttribute("aria-modal", "true");
    const focusable = () =>
      Array.from(
        panel.querySelectorAll<HTMLElement>(
          'button:not(:disabled), input, select, textarea, a[href], [tabindex="0"]',
        ),
      ).filter((element) => element.getClientRects().length > 0);
    focusable()[0]?.focus();
    const trap = (event: KeyboardEvent) => {
      if (event.key !== "Tab") return;
      const elements = focusable();
      const first = elements[0],
        last = elements.at(-1);
      if (
        !panel.contains(document.activeElement) ||
        (event.shiftKey && document.activeElement === first) ||
        (!event.shiftKey && document.activeElement === last)
      ) {
        event.preventDefault();
        (event.shiftKey ? last : first)?.focus();
      }
    };
    const resize = () => setDrawer("");
    panel.addEventListener("keydown", trap);
    media.addEventListener("change", resize);
    return () => {
      panel.removeEventListener("keydown", trap);
      media.removeEventListener("change", resize);
      background.forEach(({ element, inert }) => {
        element.inert = inert;
      });
      panel.removeAttribute("role");
      panel.removeAttribute("aria-modal");
      previous?.focus();
    };
  }, [drawer]);

  function selectDefinition(id: string) {
    navigate({ definition: id, edge: "" });
    setFlowSpan(undefined);
    setDrawer("");
  }
  function selectEdge(item: Relation) {
    navigate({ edge: item.id });
    setFlowSpan(undefined);
    setMobilePanel("source");
  }
  function selectSnapshot(id: string) {
    const next = snapshots.find((item) => item.id === id);
    if (!next) return;
    navigate({
      snapshot: id,
      context: next.context.id,
      definition: "",
      edge: "",
    });
    setSearch("");
    setQuery("");
  }
  function saveBookmarks(next: Bookmark[]) {
    setBookmarks(next);
    try {
      localStorage.setItem("ferrum-atlas.bookmarks", JSON.stringify(next));
    } catch {
      setNotification("Bookmark storage unavailable");
    }
  }
  function toggleBookmark() {
    if (activeBookmark)
      saveBookmarks(bookmarks.filter((item) => item.id !== activeBookmark.id));
    else if (activeDefinition)
      saveBookmarks([
        ...bookmarks.slice(-39),
        {
          id: crypto.randomUUID(),
          label: activeDefinition.qualified_name,
          location,
          note,
        },
      ]);
  }
  async function copyLink() {
    try {
      await navigator.clipboard.writeText(
        new URL(locationUrl(location), window.location.origin).href,
      );
      setNotification("Pinned link copied");
    } catch {
      setNotification("Clipboard unavailable");
    }
  }
  function exportTrail() {
    const url = URL.createObjectURL(
      new Blob(
        [
          JSON.stringify(
            { schema: "ferrum-atlas.reading-trail.v1", bookmarks },
            null,
            2,
          ),
        ],
        { type: "application/json" },
      ),
    );
    const link = document.createElement("a");
    link.href = url;
    link.download = "ferrum-atlas-reading-trail.json";
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  function connect(event: FormEvent) {
    event.preventDefault();
    storeToken(tokenInput);
    setToken(tokenInput.trim());
    setSession((value) => value + 1);
    setTokenInput("");
  }

  if (!token)
    return (
      <main className="connection-screen">
        <form className="connection-form" onSubmit={connect}>
          <Network size={34} />
          <h1>Ferrum Atlas</h1>
          <label htmlFor="session-token">Session token</label>
          <input
            id="session-token"
            type="password"
            autoComplete="off"
            value={tokenInput}
            onChange={(event) => setTokenInput(event.target.value)}
            required
            autoFocus
          />
          <button className="primary-button" type="submit">
            <LockKeyhole size={16} />
            Connect
          </button>
        </form>
      </main>
    );
  const stale = snapshotList.data && location.snapshot && !snapshot;
  const wrongContext =
    snapshot && location.context && location.context !== snapshot.context.id;
  return (
    <div className="app-shell">
      <header className="app-header">
        <div className="brand">
          <span className="brand-mark">
            <Network size={21} />
          </span>
          <strong>Ferrum Atlas</strong>
        </div>
        <div className="repository-context">
          <GitBranch size={15} />
          <select
            aria-label="Snapshot"
            value={snapshot?.id ?? ""}
            onChange={(event) => selectSnapshot(event.target.value)}
          >
            <option value="" disabled>
              Revision
            </option>
            {snapshots.map((item) => (
              <option key={item.id} value={item.id}>
                {item.revision} / {item.context.name}
              </option>
            ))}
          </select>
          <span className="context-name" title={snapshot?.context.target}>
            {snapshot?.context.name}
          </span>
        </div>
        <div className="global-search">
          <IconButton
            label="Show symbol results"
            onClick={() => setDrawer("scope")}
          >
            <Search size={16} />
          </IconButton>
          <input
            ref={searchRef}
            type="search"
            aria-label="Search symbols"
            placeholder="Search symbols"
            value={search}
            onChange={(event) => {
              setSearch(event.target.value);
              if (window.innerWidth > 760) setDrawer("scope");
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                setDrawer("scope");
              }
            }}
          />
        </div>
        <span className="index-status">
          <span className="status-dot" />
          {snapshot ? "Snapshot pinned" : "Connecting"}
        </span>
      </header>
      <nav className="view-tabs" aria-label="Workspace views">
        <IconButton
          label="Scope and bookmarks"
          aria-expanded={drawer === "scope"}
          onClick={() => setDrawer(drawer === "scope" ? "" : "scope")}
        >
          <ListTree size={18} />
        </IconButton>
        {views.map((view) => {
          const Icon = tabIcons[view];
          return (
            <button
              key={view}
              className={`view-tab ${location.view === view ? "active" : ""}`}
              aria-current={location.view === view ? "page" : undefined}
              onClick={() => navigate({ view })}
            >
              <Icon size={16} />
              <span>{view[0].toUpperCase() + view.slice(1)}</span>
            </button>
          );
        })}
        <div className="spacer" />
        <IconButton
          label="Evidence inspector"
          aria-expanded={drawer === "inspector"}
          onClick={() => setDrawer(drawer === "inspector" ? "" : "inspector")}
        >
          <PanelRight size={18} />
        </IconButton>
      </nav>
      {snapshotList.loading && <Loading label="Opening indexed workspace" />}
      <ErrorNotice error={snapshotList.error} />
      <ErrorNotice error={capabilities.error} />
      {snapshotList.error && (
        <form className="reconnect-form" onSubmit={connect}>
          <label>
            Session token
            <input
              aria-label="New session token"
              type="password"
              value={tokenInput}
              onChange={(event) => setTokenInput(event.target.value)}
              required
            />
          </label>
          <button type="submit" className="primary-button">
            Reconnect
          </button>
        </form>
      )}
      {stale && (
        <div className="notice error" role="alert">
          This pinned snapshot is unavailable. Select an indexed revision to
          continue.
        </div>
      )}
      {wrongContext && (
        <div className="notice error" role="alert">
          The pinned build context does not match this snapshot.
          <button
            className="text-button"
            onClick={() =>
              navigate({
                context: snapshot.context.id,
                definition: "",
                edge: "",
              })
            }
          >
            Open snapshot context
          </button>
        </div>
      )}
      {snapshotList.data?.length === 0 && (
        <div className="empty">No published snapshots.</div>
      )}
      {snapshot && !wrongContext && (
        <div
          className={`workspace drawer-${drawer}`}
          style={
            {
              "--scope-width": `${scopeWidth}px`,
              "--source-height": `${sourceHeight}px`,
            } as CSSProperties
          }
        >
          {drawer && (
            <button
              className="drawer-backdrop"
              aria-label="Close panel"
              onClick={() => setDrawer("")}
            />
          )}
          <aside className="scope-panel" aria-label="Scope and reading trail">
            <div className="panel-heading">
              <Folder size={15} />
              <strong>Workspace</strong>
              <IconButton label="Close scope" onClick={() => setDrawer("")}>
                <X size={14} />
              </IconButton>
            </div>
            <div className="scope-title">
              {snapshot.context.crates.map((crate) => crate.name).join(", ") ||
                "Source tree"}
            </div>
            <div className="scope-tree">
              {definitions.loading && <Loading label="Searching" />}
              <ErrorNotice error={definitions.error} />
              {definitions.data?.items.map((item) => (
                <button
                  key={item.id}
                  className={`symbol-row ${item.id === location.definition ? "selected" : ""} cfg-${item.cfg_status}`}
                  title={item.qualified_name}
                  onClick={() => selectDefinition(item.id)}
                >
                  <span className={`symbol-kind ${item.kind}`}>
                    {item.kind === "function"
                      ? "fn"
                      : item.kind === "module"
                        ? "mod"
                        : item.kind.slice(0, 2)}
                  </span>
                  <span>{item.name}</span>
                </button>
              ))}
              {definitions.data?.items.length === 0 && (
                <p className="empty">No matching symbols in this result.</p>
              )}
              {definitions.data?.page.next_cursor && (
                <button
                  className="text-button next-page"
                  onClick={() => setCursor(definitions.data!.page.next_cursor!)}
                >
                  Next 50 symbols <ChevronRight size={14} />
                </button>
              )}
              {cursor && (
                <button
                  className="text-button next-page"
                  onClick={() => setCursor(undefined)}
                >
                  First page
                </button>
              )}
              {definitions.data?.page.truncated &&
                !definitions.data.page.next_cursor && (
                  <p className="window-notice">
                    Search result limited by its work budget.
                  </p>
                )}
            </div>
            <div className="trail-heading">
              <span>Reading trail</span>
              <IconButton
                label="Export reading trail"
                disabled={!bookmarks.length}
                onClick={exportTrail}
              >
                <Download size={14} />
              </IconButton>
            </div>
            <div className="bookmarks">
              {bookmarks.map((item) => (
                <button
                  key={item.id}
                  className="bookmark-row"
                  title={`${item.label}\n${item.note}`}
                  onClick={() => {
                    navigate(item.location);
                    setDrawer("");
                  }}
                >
                  <BookmarkIcon size={13} />
                  <span>{item.label}</span>
                </button>
              ))}
              {!bookmarks.length && (
                <p className="muted small">No pinned selections.</p>
              )}
            </div>
            <div className="scope-bottom">
              <LockKeyhole size={13} />
              {snapshot.context.trust}
            </div>
          </aside>
          <ResizeHandle
            label="Resize scope"
            value={scopeWidth}
            minimum={170}
            maximum={350}
            onChange={setScopeWidth}
          />
          <main className="main-workspace">
            <div className="selection-toolbar">
              <IconButton label="Back" onClick={() => window.history.back()}>
                <ArrowLeft size={16} />
              </IconButton>
              <IconButton
                label="Forward"
                onClick={() => window.history.forward()}
              >
                <ArrowRight size={16} />
              </IconButton>
              <span
                className="selection-path"
                title={activeDefinition?.qualified_name}
              >
                {activeDefinition?.qualified_name ?? "Workspace"}
                {edge && (
                  <>
                    <ChevronRight size={13} />
                    call site
                  </>
                )}
              </span>
              <div className="spacer" />
              <IconButton
                label={activeBookmark ? "Unpin selection" : "Pin selection"}
                aria-pressed={Boolean(activeBookmark)}
                disabled={!activeDefinition}
                onClick={toggleBookmark}
              >
                <BookmarkIcon
                  size={16}
                  fill={activeBookmark ? "currentColor" : "none"}
                />
              </IconButton>
              <IconButton label="Copy pinned link" onClick={copyLink}>
                <Link2 size={16} />
              </IconButton>
            </div>
            <ErrorNotice error={definition.error} />
            {missingEdge && (
              <div className="notice error" role="alert">
                The pinned call site is unavailable in this bounded result.
                <button
                  className="text-button"
                  onClick={() => navigate({ edge: "" })}
                >
                  Open definition
                </button>
              </div>
            )}
            {(location.view === "explore" || location.view === "flow") && (
              <>
                <div className="mobile-view-picker segmented">
                  <IconButton
                    label="Show source"
                    aria-pressed={mobilePanel === "source"}
                    onClick={() => setMobilePanel("source")}
                  >
                    <FileCode2 size={17} />
                  </IconButton>
                  <IconButton
                    label="Show graph or flow"
                    aria-pressed={mobilePanel === "graph"}
                    onClick={() => setMobilePanel("graph")}
                  >
                    <Workflow size={17} />
                  </IconButton>
                </div>
                <div className={`explore-view mobile-${mobilePanel}`}>
                  <div className="source-area">
                    <SourcePane
                      snapshot={snapshot.id}
                      context={snapshot.context.id}
                      span={selectedSpan}
                    />
                  </div>
                  <ResizeHandle
                    label="Resize source"
                    value={sourceHeight}
                    minimum={180}
                    maximum={600}
                    onChange={setSourceHeight}
                    horizontal
                  />
                  <div className="graph-area">
                    {location.view === "explore" ? (
                      <>
                        <div className="query-controls">
                          <label>
                            Direction
                            <select
                              aria-label="Relationship direction"
                              value={direction}
                              onChange={(event) =>
                                navigate({
                                  direction: event.target.value as Direction,
                                  edge: "",
                                })
                              }
                            >
                              <option value="both">Both</option>
                              <option value="incoming">Incoming</option>
                              <option value="outgoing">Outgoing</option>
                            </select>
                          </label>
                          <label>
                            Depth
                            <select
                              aria-label="Expansion depth"
                              value={depth}
                              onChange={(event) =>
                                navigate({
                                  depth: Number(event.target.value),
                                  edge: "",
                                })
                              }
                            >
                              <option value={1}>1 hop</option>
                              <option value={2}>2 hops</option>
                              <option value={3}>3 hops</option>
                            </select>
                          </label>
                        </div>
                        {graph.loading && (
                          <Loading label="Loading relationships" />
                        )}
                        <ErrorNotice error={graph.error} />
                        {graph.data && (
                          <GraphPane
                            graph={graph.data}
                            selected={location.definition}
                            selectedEdge={location.edge}
                            onNode={selectDefinition}
                            onEdge={selectEdge}
                          />
                        )}
                      </>
                    ) : (
                      <FlowView
                        snapshot={snapshot}
                        definition={activeDefinition}
                        onSpan={(span) => {
                          setFlowSpan(span);
                          navigate({ edge: "" });
                          setMobilePanel("source");
                        }}
                      />
                    )}
                  </div>
                </div>
              </>
            )}
            {location.view === "changes" && (
              <ChangesView snapshots={snapshots} current={snapshot} />
            )}
            {location.view === "analysis" && (
              <AnalysisView
                snapshot={snapshot}
                definition={definition.data?.definition}
                graph={graph.data}
                direction={direction}
                depth={depth}
                onSelect={(id) =>
                  navigate({ definition: id, edge: "", view: "explore" })
                }
              />
            )}
            {location.view === "evidence" && (
              <EvidenceView
                key={snapshot.id}
                snapshot={snapshot}
                snapshots={snapshots}
                evidence={evidence}
                loading={evidenceLoading}
                error={evidenceError}
                selected={evidenceSelected}
              />
            )}
            {location.view === "health" && (
              <HealthView
                snapshot={snapshot}
                capabilities={capabilities.data}
              />
            )}
          </main>
          <aside className="inspector-panel" aria-label="Evidence inspector">
            <div className="panel-heading">
              <ShieldCheck size={16} />
              <strong>Inspector</strong>
              <IconButton label="Close inspector" onClick={() => setDrawer("")}>
                <X size={15} />
              </IconButton>
            </div>
            <div className="inspector-content">
              <span className="section-label">
                {location.edge ? "Call site" : "Selected definition"}
              </span>
              <h2>
                {edge
                  ? edge.target.kind === "unknown"
                    ? edge.target.label
                    : (graph.data?.nodes.find(
                        (node) =>
                          edge.target.kind === "resolved" &&
                          node.id === edge.target.id,
                      )?.name ?? "Resolved target")
                  : location.edge
                    ? "Call site unavailable"
                    : (activeDefinition?.name ?? "No selection")}
              </h2>
              {activeDefinition && !location.edge && (
                <>
                  <code className="signature">
                    {activeDefinition.signature}
                  </code>
                  <div className="definition-meta">
                    <span className="badge">{activeDefinition.kind}</span>
                    <span className="badge">
                      {activeDefinition.visibility || "private"}
                    </span>
                    <span
                      className={`badge cfg-${activeDefinition.cfg_status}`}
                    >
                      cfg: {activeDefinition.cfg_status}
                    </span>
                  </div>
                  {activeDefinition.cfg.map((condition, index) => (
                    <code key={index} className="cfg-condition">
                      {condition}
                    </code>
                  ))}
                  <dl className="metrics-list">
                    <dt>Source lines</dt>
                    <dd>{activeDefinition.metrics.lines}</dd>
                    <dt>Branches</dt>
                    <dd>{activeDefinition.metrics.branches}</dd>
                    <dt>Await points</dt>
                    <dd>{activeDefinition.metrics.awaits}</dd>
                    <dt>Unsafe blocks</dt>
                    <dd>{activeDefinition.metrics.unsafe_blocks}</dd>
                  </dl>
                </>
              )}
              {edge?.target.kind === "unknown" && (
                <div className="notice unknown">
                  <strong>Unknown boundary</strong>
                  <p>{edge.target.reason.replaceAll("_", " ")}</p>
                </div>
              )}
              {edge && (
                <dl className="metrics-list">
                  <dt>Caller</dt>
                  <dd className="mono">
                    {graph.data?.nodes.find((node) => node.id === edge.source)
                      ?.qualified_name ?? edge.source}
                  </dd>
                  <dt>Target</dt>
                  <dd className="mono">
                    {edge.target.kind === "resolved"
                      ? (graph.data?.nodes.find(
                          (node) =>
                            edge.target.kind === "resolved" &&
                            node.id === edge.target.id,
                        )?.qualified_name ?? edge.target.id)
                      : edge.target.label}
                  </dd>
                  <dt>UTF-8 span</dt>
                  <dd>
                    {edge.span.start}..{edge.span.end}
                  </dd>
                </dl>
              )}
              <CoverageNotice
                coverage={
                  graph.data?.coverage ??
                  definition.data?.coverage ??
                  snapshot.coverage
                }
              />
              <ErrorNotice error={evidenceError} />
              {evidenceLoading && <Loading label="Loading provenance" />}
              {!evidenceLoading && !evidenceError && evidenceSelected && (
                <EvidenceList evidence={evidence} />
              )}
              <section className="note-section">
                <label htmlFor="trail-note">Review note</label>
                <textarea
                  id="trail-note"
                  placeholder="Invariant or open question"
                  value={activeBookmark?.note ?? note}
                  onChange={(event) => {
                    if (activeBookmark)
                      saveBookmarks(
                        bookmarks.map((item) =>
                          item.id === activeBookmark.id
                            ? { ...item, note: event.target.value }
                            : item,
                        ),
                      );
                    else setNote(event.target.value);
                  }}
                />
                <span className="muted small">
                  {activeBookmark ? "Saved" : "Unpinned note"}
                </span>
              </section>
            </div>
          </aside>
        </div>
      )}
      <footer className="status-bar">
        <Code2 size={13} />
        <span>{snapshot?.context.target ?? "Local workspace"}</span>
        <span className="status-selection">
          {snapshot?.revision.slice(0, 24)}
          {activeDefinition ? ` / ${activeDefinition.name}` : ""}
        </span>
        <div className="spacer" />
        {notification ? (
          <span role="status">
            <Check size={13} />
            {notification}
          </span>
        ) : (
          <span>
            {graph.data
              ? `${graph.data.work.elapsed_ms} ms / coverage ${graph.data.coverage.status}`
              : "Read-only source"}
          </span>
        )}
      </footer>
    </div>
  );
}
