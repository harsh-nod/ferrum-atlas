import { useEffect, useState } from "react";
import { GitCompareArrows, ShieldCheck } from "lucide-react";
import type {
  Capabilities,
  Definition,
  DiffResponse,
  Evidence,
  FunctionFlow,
  ObservationSummary,
  ObservationWindow,
  Snapshot,
  Span,
} from "../api/types";
import { params, request } from "../api/client";
import { useResource } from "../state";
import { CoverageNotice, ErrorNotice, Loading } from "./common";
import { SourcePane } from "./SourcePane";
import { JobsView } from "./JobsView";
import { TraceCompareView } from "./TraceCompareView";

export function EvidenceList({ evidence }: { evidence: Evidence[] }) {
  return (
    <div className="evidence-list">
      {evidence.length === 0 && (
        <p className="muted">No provenance records for this selection.</p>
      )}
      {evidence.map((item) => (
        <section key={item.id} className="evidence-record">
          <div className="section-label">
            <ShieldCheck size={15} />
            {item.basis}
          </div>
          <dl>
            <dt>Producer</dt>
            <dd>{item.producer}</dd>
            <dt>Evidence</dt>
            <dd className="mono">{item.id}</dd>
          </dl>
          {item.assumptions.length > 0 && (
            <>
              <h4>Assumptions</h4>
              <ul>
                {item.assumptions.map((text, index) => (
                  <li key={index}>{text}</li>
                ))}
              </ul>
            </>
          )}
          {item.limitations.length > 0 && (
            <>
              <h4>Limitations</h4>
              <ul>
                {item.limitations.map((text, index) => (
                  <li key={index}>{text}</li>
                ))}
              </ul>
            </>
          )}
          {item.inputs.length > 0 && (
            <details>
              <summary>Input identities ({item.inputs.length})</summary>
              <ul className="mono">
                {item.inputs.map((text, index) => (
                  <li key={index}>{text}</li>
                ))}
              </ul>
            </details>
          )}
        </section>
      ))}
    </div>
  );
}

export function EvidenceView({
  evidence,
  snapshot,
  snapshots,
  loading = false,
  error,
  selected: evidenceSelected = true,
}: {
  evidence: Evidence[];
  snapshot: Snapshot;
  snapshots: Snapshot[];
  loading?: boolean;
  error?: Error;
  selected?: boolean;
}) {
  const pin = { snapshot_id: snapshot.id, context_id: snapshot.context.id };
  const summaries = useResource<ObservationSummary[]>(
    snapshot.id + ":observations",
    (signal) => request("/observations" + params(pin), signal),
  );
  const [selected, setSelected] = useState("");
  const [offset, setOffset] = useState(0);
  const window = useResource<ObservationWindow>(
    selected ? snapshot.id + selected + offset : "",
    (signal) =>
      request(
        "/observations/" +
          encodeURIComponent(selected) +
          params({ ...pin, offset, limit: 100 }),
        signal,
      ),
  );
  useEffect(() => {
    setSelected("");
    setOffset(0);
  }, [snapshot.id]);
  const sourceLink = (id: string) =>
    "?" +
    new URLSearchParams({
      snapshot: snapshot.id,
      context: snapshot.context.id,
      definition: id,
      view: "explore",
      depth: "2",
      direction: "both",
    });
  return (
    <div className="document-view">
      <div className="view-heading">
        <ShieldCheck size={20} />
        <h2>Evidence</h2>
      </div>
      <TraceCompareView snapshot={snapshot} snapshots={snapshots} />
      {loading && <Loading label="Loading provenance" />}
      <ErrorNotice error={error} />
      {!loading && !error && evidenceSelected && (
        <EvidenceList evidence={evidence} />
      )}
      <section className="unframed-section">
        <h3>Imported Tests and Traces</h3>
        <ErrorNotice error={summaries.error} />
        {summaries.loading && <Loading label="Loading observations" />}
        {summaries.data?.length === 0 && (
          <p className="muted">
            No artifact-matched observations for this snapshot.
          </p>
        )}
        {summaries.data?.map((item) => (
          <button
            key={item.id}
            className="change-row"
            onClick={() => {
              setSelected(item.id);
              setOffset(0);
            }}
          >
            <strong>{item.artifact.producer}</strong>
            <span>
              {item.test_count} tests / {item.event_count} events
            </span>
            <span>{item.clock_domains.join(", ")}</span>
          </button>
        ))}
        <ErrorNotice error={window.error} />
        {window.loading && <Loading label="Loading observation window" />}
        {window.data && (
          <>
            <dl>
              <dt>Artifact SHA-256</dt>
              <dd className="mono">{window.data.summary.artifact.sha256}</dd>
            </dl>
            <ul>
              {window.data.summary.limitations.map((text, i) => (
                <li key={i}>{text}</li>
              ))}
            </ul>
            {window.data.truncated && (
              <p className="window-notice">Bounded observation window</p>
            )}
            <div className="observation-table-scroll">
              <table className="observation-table">
                <thead>
                  <tr>
                    <th>Test</th>
                    <th>Outcome</th>
                    <th>Elapsed ns</th>
                    <th>Timeout ns</th>
                    <th>Reason / source</th>
                  </tr>
                </thead>
                <tbody>
                  {window.data.tests.map((test, i) => (
                    <tr key={i}>
                      <td>{test.name}</td>
                      <td>{test.outcome.replaceAll("_", " ")}</td>
                      <td>{test.elapsed_ns ?? "unknown"}</td>
                      <td>{test.timeout_ns ?? "unknown"}</td>
                      <td>
                        {test.reason ?? "Not recorded"}
                        {test.definition_ids.map((id) => (
                          <div key={id}>
                            <a
                              className="observation-source"
                              href={sourceLink(id)}
                              title={id}
                            >
                              Source
                            </a>
                          </div>
                        ))}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            {window.data.streams.map((stream) => (
              <section key={stream.id} className="trace-stream">
                <h4>
                  {stream.id} / {stream.clock_domain} / {stream.timestamp_unit}
                </h4>
                <p className="muted">
                  {stream.process_or_device} / {stream.thread_or_hart}
                </p>
                <div className="observation-table-scroll">
                  <table className="observation-table">
                    <thead>
                      <tr>
                        <th>Sequence</th>
                        <th>Timestamp</th>
                        <th>Event</th>
                        <th>Loss</th>
                        <th>Source / correlation</th>
                      </tr>
                    </thead>
                    <tbody>
                      {stream.events.map((event) => (
                        <tr key={event.sequence}>
                          <td>{event.sequence}</td>
                          <td>{event.timestamp}</td>
                          <td>{event.kind}</td>
                          <td>{event.loss_count ?? "unknown"}</td>
                          <td>
                            {event.definition_id ? (
                              <a
                                className="observation-source"
                                href={sourceLink(event.definition_id)}
                                title={event.definition_id}
                              >
                                Source
                              </a>
                            ) : (
                              "Unmapped"
                            )}
                            <div>
                              {event.correlation_id ?? "No correlation"}
                            </div>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </section>
            ))}
            {window.data.next_offset !== null && (
              <button
                className="text-button"
                onClick={() => setOffset(window.data!.next_offset!)}
              >
                Next observation window
              </button>
            )}
            {offset > 0 && (
              <button className="text-button" onClick={() => setOffset(0)}>
                First observation window
              </button>
            )}
          </>
        )}
      </section>
    </div>
  );
}

export function FlowView({
  snapshot,
  definition,
  onSpan,
}: {
  snapshot: Snapshot;
  definition?: Definition;
  onSpan: (span: Span) => void;
}) {
  const flow = useResource<FunctionFlow>(
    definition ? snapshot.id + ":" + definition.id + ":flow" : "",
    (signal) =>
      request(
        "/flow/" +
          encodeURIComponent(definition!.id) +
          params({
            snapshot_id: snapshot.id,
            context_id: snapshot.context.id,
            phase: "source",
          }),
        signal,
      ),
  );
  return (
    <section className="flow-view">
      <div className="panel-heading">
        <strong>Source flow</strong>
        <span className="muted">Compiler CFG unavailable</span>
      </div>
      {flow.loading && <Loading label="Loading flow" />}
      <ErrorNotice error={flow.error} />
      {flow.data && (
        <div className="document-view">
          <p className="phase-label">Phase: {flow.data.phase}</p>
          <ol className="flow-points">
            {flow.data.points.map((point, index) => (
              <li key={point.span.start + ":" + index}>
                <span className="flow-number">{index + 1}</span>
                <button
                  className="flow-point"
                  onClick={() => onSpan(point.span)}
                >
                  <span className="badge">
                    {point.kind.replaceAll("_", " ")}
                  </span>
                  <code>{point.label}</code>
                </button>
              </li>
            ))}
          </ol>
          {flow.data.points.length === 0 && (
            <p className="muted">No source-flow points in this result.</p>
          )}
          <CoverageNotice coverage={flow.data.coverage} />
        </div>
      )}
    </section>
  );
}

export function ChangesView({
  snapshots,
  current,
}: {
  snapshots: Snapshot[];
  current: Snapshot;
}) {
  const candidates = snapshots.filter(
    (snapshot) =>
      snapshot.repository_id === current.repository_id &&
      snapshot.id !== current.id,
  );
  const [beforeId, setBeforeId] = useState(() => candidates[0]?.id ?? "");
  const [selected, setSelected] = useState(0);
  useEffect(() => {
    if (!candidates.some((snapshot) => snapshot.id === beforeId))
      setBeforeId(candidates[0]?.id ?? "");
  }, [current.id, snapshots, beforeId]);
  const validBefore = candidates.some((snapshot) => snapshot.id === beforeId)
    ? beforeId
    : "";
  const diff = useResource<DiffResponse>(
    validBefore ? validBefore + ":" + current.id + ":diff" : "",
    (signal) =>
      request("/diff", signal, { before: beforeId, after: current.id }),
  );
  useEffect(() => setSelected(0), [diff.data]);
  const before = snapshots.find((snapshot) => snapshot.id === beforeId);
  const change = diff.data?.changes[selected];
  return (
    <section className="changes-view">
      <div className="panel-heading">
        <GitCompareArrows size={17} />
        <strong>Changes</strong>
        <select
          aria-label="Base snapshot"
          value={beforeId}
          onChange={(event) => setBeforeId(event.target.value)}
        >
          <option value="">Choose base revision</option>
          {candidates.map((snapshot) => (
            <option key={snapshot.id} value={snapshot.id}>
              {snapshot.revision} / {snapshot.context.name}
            </option>
          ))}
        </select>
      </div>
      <ErrorNotice error={diff.error} />
      {diff.loading && <Loading label="Comparing snapshots" />}
      {!beforeId && (
        <div className="empty">
          A second indexed snapshot is required for comparison.
        </div>
      )}
      {diff.data && (
        <>
          <div className="diff-summary">
            <span>{diff.data.changes.length} changed definitions</span>
            <span>{diff.data.impact_candidates.length} impact candidates</span>
            <CoverageNotice coverage={diff.data.coverage} compact />
            {diff.data.context_changed && (
              <span className="badge partial">Configuration changed</span>
            )}
          </div>
          <div
            className="change-list"
            role="list"
            aria-label="Changed definitions"
          >
            {diff.data.changes.map((item, index) => (
              <button
                key={String(item.before?.id) + ":" + item.after?.id}
                className={
                  "change-row " + (selected === index ? "selected" : "")
                }
                onClick={() => setSelected(index)}
              >
                <span className={"badge " + item.kind}>{item.kind}</span>
                <strong>{(item.after ?? item.before)?.qualified_name}</strong>
                <span className="muted">{item.changed_fields.join(", ")}</span>
              </button>
            ))}
          </div>
          {change && (
            <>
              <div className="correspondence">
                Correspondence: {change.correspondence}
              </div>
              <div className="diff-sources">
                <SourcePane
                  snapshot={beforeId}
                  context={before?.context.id ?? ""}
                  span={change.before?.span}
                  title="Before"
                />
                <SourcePane
                  snapshot={current.id}
                  context={current.context.id}
                  span={change.after?.span}
                  title="After"
                />
              </div>
            </>
          )}
          <div className="document-view compact-document">
            <CoverageNotice coverage={diff.data.coverage} />
            {diff.data.truncated && (
              <p className="window-notice">
                Comparison truncated. Additional changes may exist.
              </p>
            )}
            {diff.data.impact_candidates.length > 0 && (
              <section>
                <h3>Impact candidates</h3>
                <ul>
                  {diff.data.impact_candidates.map((item) => (
                    <li key={item.id}>
                      <code>{item.qualified_name}</code>
                    </li>
                  ))}
                </ul>
              </section>
            )}
          </div>
        </>
      )}
    </section>
  );
}

export function HealthView({
  snapshot,
  capabilities,
}: {
  snapshot: Snapshot;
  capabilities?: Capabilities;
}) {
  return (
    <div className="document-view">
      <div className="view-heading">
        <ShieldCheck size={20} />
        <h2>Index Health</h2>
      </div>
      <JobsView snapshot={snapshot} />
      <div className="stat-strip">
        <div>
          <strong>{snapshot.file_count.toLocaleString()}</strong>
          <span>Source files</span>
        </div>
        <div>
          <strong>{snapshot.definition_count.toLocaleString()}</strong>
          <span>Definitions</span>
        </div>
        <div>
          <strong>{snapshot.relation_count.toLocaleString()}</strong>
          <span>Relationships</span>
        </div>
      </div>
      <CoverageNotice coverage={snapshot.coverage} />
      <section className="unframed-section">
        <h3>Build context</h3>
        <dl className="context-list">
          <dt>Context</dt>
          <dd>{snapshot.context.name}</dd>
          <dt>Target</dt>
          <dd>{snapshot.context.target}</dd>
          <dt>Features</dt>
          <dd>{snapshot.context.features.join(", ") || "None selected"}</dd>
          <dt>Default features</dt>
          <dd>{snapshot.context.default_features ? "Enabled" : "Disabled"}</dd>
          <dt>Trust</dt>
          <dd>{snapshot.context.trust}</dd>
          <dt>Producer</dt>
          <dd>{snapshot.producer}</dd>
          <dt>Published</dt>
          <dd>
            {new Date(Number(snapshot.created_at) * 1000).toLocaleString()}
          </dd>
          <dt>Snapshot</dt>
          <dd className="mono">{snapshot.id}</dd>
          <dt>Fact digest</dt>
          <dd className="mono">{snapshot.fact_digest}</dd>
        </dl>
        <CoverageNotice coverage={snapshot.context.coverage} />
      </section>
      {capabilities && (
        <section className="unframed-section">
          <h3>Available analysis</h3>
          <p>{capabilities.analysis_levels.join(", ")}</p>
          <ul>
            {capabilities.limitations.map((item, index) => (
              <li key={index}>{item}</li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}
