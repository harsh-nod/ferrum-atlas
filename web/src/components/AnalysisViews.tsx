import { useEffect, useState } from "react";
import { ChartNoAxesCombined, FileCode2, Route } from "lucide-react";
import type {
  AnalysisEnvelope,
  AnalysisResponse,
  Definition,
  Direction,
  GraphAnalysis,
  GraphRequest,
  GraphResponse,
  PathAnalysis,
  PathRequest,
  Snapshot,
} from "../api/types";
import { request } from "../api/client";
import { useResource } from "../state";
import { CoverageNotice, ErrorNotice, Loading } from "./common";

export function AnalysisNotes({ envelope }: { envelope: AnalysisEnvelope }) {
  return (
    <>
      <CoverageNotice coverage={envelope.coverage} />
      {(envelope.truncated ||
        envelope.cancelled ||
        envelope.deadline_reached) && (
        <p className="notice" role="status">
          {envelope.cancelled
            ? "Cancelled"
            : envelope.deadline_reached
              ? "Deadline reached"
              : "Bounded result"}
        </p>
      )}
      <details className="analysis-assumptions">
        <summary>Algorithm and assumptions</summary>
        <p className="mono">{envelope.algorithm_version}</p>
        <ul>
          {envelope.assumptions.map((assumption, index) => (
            <li key={index}>{assumption}</li>
          ))}
        </ul>
      </details>
    </>
  );
}

export function AnalysisView({
  snapshot,
  definition,
  graph,
  direction,
  depth,
  onSelect,
}: {
  snapshot: Snapshot;
  definition?: Definition;
  graph?: GraphResponse;
  direction: Direction;
  depth: number;
  onSelect: (id: string) => void;
}) {
  const [target, setTarget] = useState("");
  const [pathRequest, setPathRequest] = useState<PathRequest | null>(null);
  const [pathScope, setPathScope] = useState("");
  const graphRequest: GraphRequest | null = definition
    ? {
        snapshot_id: snapshot.id,
        context_id: snapshot.context.id,
        definition_id: definition.id,
        direction,
        depth,
        max_nodes: 200,
        max_edges: 500,
      }
    : null;
  const selectionKey = graphRequest ? JSON.stringify(graphRequest) : "";
  const analysis = useResource<AnalysisResponse<GraphAnalysis>>(
    selectionKey,
    (signal) => request("/graph/analysis", signal, graphRequest),
  );
  const path = useResource<AnalysisResponse<PathAnalysis>>(
    pathRequest && pathScope === selectionKey
      ? JSON.stringify(pathRequest)
      : "",
    (signal) => request("/graph/path", signal, pathRequest),
  );
  useEffect(() => {
    setTarget("");
    setPathRequest(null);
  }, [selectionKey]);
  const name = (id: string) =>
    graph?.nodes.find((node) => node.id === id)?.qualified_name ?? id;
  const source = (id: string) => (
    <button
      type="button"
      className="analysis-source"
      onClick={() => onSelect(id)}
      title={`Open ${name(id)}`}
    >
      <FileCode2 size={14} />
      <span>{name(id)}</span>
    </button>
  );
  return (
    <div className="document-view analysis-view">
      <div className="view-heading">
        <ChartNoAxesCombined size={20} />
        <h2>Selected Graph Analysis</h2>
      </div>
      {!definition && <p className="muted">No definition selected.</p>}
      <ErrorNotice error={analysis.error} />
      {analysis.loading && <Loading label="Analyzing selected call graph" />}
      {analysis.data && (
        <>
          <div className="stat-strip">
            <div>
              <strong>{analysis.data.analysis.selected_nodes}</strong>
              <span>Selected definitions</span>
            </div>
            <div>
              <strong>{analysis.data.analysis.selected_call_sites}</strong>
              <span>Selected call sites</span>
            </div>
            <div>
              <strong>
                {
                  analysis.data.analysis.components.filter(
                    (component) => component.recursive,
                  ).length
                }
              </strong>
              <span>Recursive components</span>
            </div>
          </div>
          <AnalysisNotes envelope={analysis.data.analysis.envelope} />
          <section className="unframed-section">
            <h3>Recursive Components</h3>
            {analysis.data.analysis.components.filter(
              (component) => component.recursive,
            ).length === 0 && (
              <p className="muted">
                No recursive component found in the selected known call graph.
              </p>
            )}
            {analysis.data.analysis.components
              .filter((component) => component.recursive)
              .map((component, index) => (
                <div className="component-row" key={component.id}>
                  <strong>Component {index + 1}</strong>
                  <div>
                    {component.members.map((id) => (
                      <div key={id}>{source(id)}</div>
                    ))}
                  </div>
                </div>
              ))}
          </section>
          <section className="unframed-section">
            <h3>Definition Metrics</h3>
            <div className="analysis-table-scroll">
              <table className="analysis-table">
                <thead>
                  <tr>
                    <th>Definition</th>
                    <th>Callers</th>
                    <th>Callees</th>
                    <th>Call sites</th>
                    <th>Unknown</th>
                    <th>Source branches</th>
                    <th>Unsafe blocks</th>
                  </tr>
                </thead>
                <tbody>
                  {analysis.data.analysis.metrics.map((metric) => (
                    <tr key={metric.definition_id}>
                      <td>{source(metric.definition_id)}</td>
                      <td>{metric.distinct_callers}</td>
                      <td>{metric.distinct_callees}</td>
                      <td>{metric.outgoing_call_sites}</td>
                      <td>{metric.unknown_call_sites}</td>
                      <td>{metric.source_branches}</td>
                      <td>{metric.source_unsafe_blocks}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <details>
              <summary>Metric distributions</summary>
              <div className="analysis-table-scroll">
                <table className="analysis-table">
                  <thead>
                    <tr>
                      <th>Metric</th>
                      <th>Minimum</th>
                      <th>Median</th>
                      <th>p95</th>
                      <th>Maximum</th>
                    </tr>
                  </thead>
                  <tbody>
                    {analysis.data.analysis.distributions.map(
                      (distribution) => (
                        <tr key={distribution.metric}>
                          <td>{distribution.metric.replaceAll("_", " ")}</td>
                          <td>{distribution.minimum}</td>
                          <td>{distribution.median}</td>
                          <td>{distribution.p95}</td>
                          <td>{distribution.maximum}</td>
                        </tr>
                      ),
                    )}
                  </tbody>
                </table>
              </div>
            </details>
          </section>
          {analysis.data.analysis.unknown_frontier.length > 0 && (
            <section className="unframed-section">
              <h3>Unknown Frontiers</h3>
              <ul className="frontier-list">
                {analysis.data.analysis.unknown_frontier.map((frontier) => (
                  <li key={frontier.relation_id}>
                    {source(frontier.source)}
                    <span>{frontier.reason.replaceAll("_", " ")}</span>
                  </li>
                ))}
              </ul>
            </section>
          )}
        </>
      )}
      {definition && (
        <section className="unframed-section">
          <h3>Outgoing Paths</h3>
          <form
            className="analysis-controls"
            onSubmit={(event) => {
              event.preventDefault();
              if (graphRequest) {
                setPathScope(selectionKey);
                setPathRequest({
                  graph: { ...graphRequest, direction: "outgoing" },
                  target: target || null,
                });
              }
            }}
          >
            <label>
              Target
              <select
                aria-label="Path target"
                value={target}
                onChange={(event) => {
                  setTarget(event.target.value);
                  setPathRequest(null);
                }}
              >
                <option value="">Reachable set</option>
                {graph?.nodes.map((node) => (
                  <option key={node.id} value={node.id}>
                    {node.qualified_name}
                  </option>
                ))}
              </select>
            </label>
            <button
              className="text-button"
              type="submit"
              disabled={path.loading}
            >
              <Route size={15} />
              {target ? "Find path" : "Find reachable definitions"}
            </button>
          </form>
          <ErrorNotice error={path.error} />
          {path.loading && <Loading label="Exploring outgoing calls" />}
          {path.data && (
            <div className="path-result">
              <h4>
                {path.data.analysis.outcome === "found"
                  ? "Static Path Found"
                  : path.data.analysis.outcome === "not_found_in_selection"
                    ? "No Path in Complete Selection"
                    : path.data.analysis.outcome === "reachability_complete"
                      ? "Selected Reachable Set"
                      : "Reachability Unknown"}
              </h4>
              {path.data.analysis.path.length > 0 && (
                <ol className="path-steps">
                  {path.data.analysis.path.map((id) => (
                    <li key={id}>{source(id)}</li>
                  ))}
                </ol>
              )}
              {path.data.analysis.path.length === 0 &&
                path.data.analysis.reachable.length > 0 && (
                  <ul className="path-steps">
                    {path.data.analysis.reachable.map((id) => (
                      <li key={id}>{source(id)}</li>
                    ))}
                  </ul>
                )}
              <AnalysisNotes envelope={path.data.analysis.envelope} />
            </div>
          )}
        </section>
      )}
    </div>
  );
}
