import { useState } from "react";
import { ArrowLeft, ArrowRight, ChartNoAxesCombined } from "lucide-react";
import type {
  AnalysisResponse,
  CompilerComplexity,
  CompilerFlowPage,
  Definition,
  Snapshot,
  SourceWindow,
  Span,
} from "../api/types";
import { params, request } from "../api/client";
import { useResource } from "../state";
import { AnalysisNotes } from "./AnalysisViews";
import { CompilerSourceLink } from "./CompilerSourceLink";
import { ErrorNotice, IconButton, Loading } from "./common";

export function CompilerComplexityView({
  snapshot,
  definition,
  importId,
  flow,
  source,
  onSpan,
}: {
  snapshot: Snapshot;
  definition: Definition;
  importId: string;
  flow: CompilerFlowPage;
  source?: SourceWindow;
  onSpan: (span: Span) => void;
}) {
  const scope = `${snapshot.id}:${snapshot.context.id}:${definition.id}:${importId}:${flow.body.body_id}`;
  const [run, setRun] = useState({ scope: "", count: 0 });
  const [window, setWindow] = useState({ scope: "", offset: 0 });
  const offset = window.scope === scope ? window.offset : 0;
  const result = useResource<AnalysisResponse<CompilerComplexity>>(
    run.scope === scope ? `${scope}:${run.count}` : "",
    (signal) =>
      request(
        `/compiler/bodies/${encodeURIComponent(definition.id)}/complexity` +
          params({
            snapshot_id: snapshot.id,
            context_id: snapshot.context.id,
            import_id: importId,
          }),
        signal,
      ),
  );
  const value = result.data?.analysis;
  const matches =
    value &&
    result.data?.snapshot_id === snapshot.id &&
    result.data.context_id === snapshot.context.id &&
    value.definition_id === definition.id &&
    value.import_id === importId &&
    value.body_id === flow.body.body_id &&
    value.phase === flow.phase &&
    value.input_manifest_hash === flow.input_manifest_hash &&
    value.panic_strategy === flow.panic_strategy &&
    (Object.keys(flow.compiler) as (keyof typeof flow.compiler)[]).every(
      (key) => value.compiler[key] === flow.compiler[key],
    );
  const data = matches ? value : undefined;
  const metrics = data?.metrics;
  const locationsAvailable = Boolean(
    data &&
    !data.locations_truncated &&
    !data.envelope.truncated &&
    !data.envelope.cancelled &&
    !data.envelope.deadline_reached,
  );
  return (
    <section
      className="compiler-complexity-view unframed-section"
      aria-label="Compiler CFG complexity"
    >
      <div className="analysis-section-heading">
        <h3>Compiler CFG Complexity</h3>
        <button
          type="button"
          className="analysis-source"
          disabled={result.loading}
          onClick={() => {
            setRun({ scope, count: run.count + 1 });
            setWindow({ scope, offset: 0 });
          }}
        >
          <ChartNoAxesCombined size={15} /> Measure CFG
        </button>
      </div>
      {result.loading && <Loading label="Measuring compiler CFG" />}
      <ErrorNotice error={result.error} />
      {result.data && !matches && (
        <ErrorNotice
          error={
            new Error(
              "CFG measurement does not match the selected compiler body and scope",
            )
          }
        />
      )}
      {data && (
        <>
          <p>
            <code>{data.formula}</code>{" "}
            <span className={`badge ${data.envelope.coverage.status}`}>
              {data.envelope.coverage.status}
            </span>
          </p>
          <details className="compiler-measurement-notes" open={!metrics}>
            <summary>Coverage and assumptions</summary>
            <AnalysisNotes envelope={data.envelope} />
          </details>
          <details>
            <summary>CFG measurement provenance</summary>
            <dl className="context-list">
              <dt>Phase</dt>
              <dd>{data.phase}</dd>
              <dt>Compiler</dt>
              <dd>
                {data.compiler.release} / {data.compiler.host}
              </dd>
              <dt>Compiler commit</dt>
              <dd className="mono">{data.compiler.commit_hash}</dd>
              <dt>Adapter</dt>
              <dd>
                {data.compiler.adapter} {data.compiler.adapter_version}
              </dd>
              <dt>Input digest</dt>
              <dd className="mono">{data.input_digest}</dd>
              <dt>Input manifest</dt>
              <dd className="mono">{data.input_manifest_hash}</dd>
              <dt>Import</dt>
              <dd className="mono">{data.import_id}</dd>
              <dt>Body</dt>
              <dd className="mono">{data.body_id}</dd>
              <dt>Panic strategy</dt>
              <dd>{data.panic_strategy}</dd>
              <dt>Body source</dt>
              <dd>
                <CompilerSourceLink
                  mapping={data.body_span}
                  source={source}
                  snapshotId={snapshot.id}
                  fileId={definition.file_id}
                  onSpan={onSpan}
                  label="Show measured CFG body source"
                />
              </dd>
            </dl>
          </details>
          {metrics ? (
            <dl
              className="context-list compiler-complexity-metrics"
              aria-label="CFG structural metrics"
            >
              <dt>Cyclomatic</dt>
              <dd>{metrics.cyclomatic}</dd>
              <dt title="Nodes include the synthetic exit">Nodes (N)</dt>
              <dd>{metrics.nodes}</dd>
              <dt title="Edges include synthetic exit edges">Edges (E)</dt>
              <dd>{metrics.edges}</dd>
              <dt>Components (P)</dt>
              <dd>{metrics.components}</dd>
              <dt title="Entry-reachable runtime blocks">Runtime blocks</dt>
              <dd>{metrics.runtime_blocks}</dd>
              <dt title="Runtime successor edges">Runtime edges</dt>
              <dd>{metrics.runtime_edges}</dd>
              <dt title="Synthetic exit edges">Exit edges</dt>
              <dd>{metrics.synthetic_exit_edges}</dd>
              <dt title="Excluded imaginary edges">Excluded edges</dt>
              <dd>{metrics.excluded_imaginary_edges}</dd>
            </dl>
          ) : (
            <p role="status" className="muted">
              CFG metrics unavailable; no numeric result was returned.
            </p>
          )}
          {!locationsAvailable ? (
            <p role="status" className="muted">
              CFG location records unavailable; exit-site and unreachable-block
              totals are not shown.
            </p>
          ) : (
            <>
              <dl className="context-list">
                <dt>Returned entry-reachable blocks</dt>
                <dd>{data.reachable_blocks.length}</dd>
                <dt>Returned unreachable blocks</dt>
                <dd>{data.unreachable_blocks.length}</dd>
              </dl>
              <h3>CFG Exit Sites</h3>
              {data.exit_sites.length === 0 ? (
                <p className="muted">No exit-site records returned.</p>
              ) : (
                <>
                  <div className="analysis-table-scroll">
                    <table className="analysis-table compiler-exit-table">
                      <thead>
                        <tr>
                          <th>Block</th>
                          <th>Exit kind</th>
                          <th>Source</th>
                        </tr>
                      </thead>
                      <tbody>
                        {data.exit_sites
                          .slice(offset, offset + 100)
                          .map((site, index) => (
                            <tr
                              key={`${site.block}:${site.kind}:${offset + index}`}
                            >
                              <td>
                                <code>bb{site.block}</code>
                              </td>
                              <td>{site.kind.replaceAll("_", " ")}</td>
                              <td>
                                <CompilerSourceLink
                                  mapping={site.span}
                                  source={source}
                                  snapshotId={snapshot.id}
                                  fileId={definition.file_id}
                                  onSpan={onSpan}
                                  label={`Show CFG exit source at bb${site.block} (${site.kind})`}
                                />
                              </td>
                            </tr>
                          ))}
                      </tbody>
                    </table>
                  </div>
                  <div className="compiler-pagination">
                    <span>
                      {offset + 1}-
                      {Math.min(offset + 100, data.exit_sites.length)} of{" "}
                      {data.exit_sites.length} exit sites
                    </span>
                    <IconButton
                      label="Previous CFG exit sites"
                      disabled={offset === 0}
                      onClick={() =>
                        setWindow({ scope, offset: Math.max(0, offset - 100) })
                      }
                    >
                      <ArrowLeft size={15} />
                    </IconButton>
                    <IconButton
                      label="Next CFG exit sites"
                      disabled={offset + 100 >= data.exit_sites.length}
                      onClick={() => setWindow({ scope, offset: offset + 100 })}
                    >
                      <ArrowRight size={15} />
                    </IconButton>
                  </div>
                </>
              )}
            </>
          )}
        </>
      )}
    </section>
  );
}
