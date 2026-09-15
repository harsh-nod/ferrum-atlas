import { useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  ChartNoAxesCombined,
  FileCode2,
} from "lucide-react";
import type {
  AnalysisResponse,
  Definition,
  Snapshot,
  SourceMaintainability,
  Span,
} from "../api/types";
import { params, request } from "../api/client";
import { useResource } from "../state";
import { ErrorNotice, Loading } from "./common";
import { AnalysisNotes } from "./AnalysisNotes";

type Kind = "nesting" | "unsafe" | "lines" | "unknowns";
const pageSize = 25;

export function MaintainabilityView({
  snapshot,
  definition,
  onSpan,
}: {
  snapshot: Snapshot;
  definition: Definition;
  onSpan: (span: Span) => void;
}) {
  const [attempt, setAttempt] = useState(0);
  const [kind, setKind] = useState<Kind>("nesting");
  const [offset, setOffset] = useState(0);
  const resource = useResource<AnalysisResponse<SourceMaintainability>>(
    attempt
      ? JSON.stringify([
          snapshot.id,
          snapshot.context.id,
          definition.id,
          attempt,
        ])
      : "",
    (signal) =>
      request(
        `/analysis/maintainability/${encodeURIComponent(definition.id)}${params({ snapshot_id: snapshot.id, context_id: snapshot.context.id })}`,
        signal,
      ),
  );
  const scoped =
    !resource.data ||
    (resource.data.snapshot_id === snapshot.id &&
      resource.data.context_id === snapshot.context.id &&
      resource.data.analysis.definition_id === definition.id);
  const report = scoped ? resource.data?.analysis : undefined;
  const sites: { span: Span; label: string }[] = !report
    ? []
    : kind === "nesting"
      ? report.nesting_sites.map((site) => ({
          span: site.span,
          label: `${site.kind} / depth ${site.depth}`,
        }))
      : kind === "unsafe"
        ? report.unsafe_sites.map((site) => ({
            span: site.keyword_span,
            label: `${site.kind}${site.enclosing_boundary ? " / nested unsafe boundary" : " / outer unsafe boundary"}`,
          }))
        : kind === "lines"
          ? report.code_lines.map((span, index) => ({
              span,
              label: `Code line ${index + 1}`,
            }))
          : report.unknowns.map((site) => ({
              span: site.span,
              label: site.reason,
            }));
  return (
    <section
      className="unframed-section maintainability"
      aria-label="Source maintainability"
    >
      <h3>Source Maintainability</h3>
      <button
        type="button"
        className="analysis-source"
        disabled={resource.loading}
        onClick={() => {
          setAttempt((value) => value + 1);
          setOffset(0);
        }}
      >
        <ChartNoAxesCombined size={16} /> Measure source
      </button>
      <ErrorNotice error={resource.error} />
      {!scoped && (
        <p className="notice" role="alert">
          Metric response does not match this selection.
        </p>
      )}
      {resource.loading && <Loading label="Measuring selected source" />}
      {report && (
        <>
          {report.metrics ? (
            <dl className="maintainability-counts">
              <div>
                <dt>Noncomment code lines</dt>
                <dd>{report.metrics.source_lines}</dd>
              </div>
              <div>
                <dt>Lexical tokens</dt>
                <dd>{report.metrics.lexical_tokens}</dd>
              </div>
              <div>
                <dt>Maximum syntax nesting</dt>
                <dd>{report.metrics.max_nesting}</dd>
              </div>
              <div>
                <dt>Unsafe boundaries</dt>
                <dd>{report.metrics.unsafe_boundaries}</dd>
              </div>
            </dl>
          ) : (
            <p className="notice" role="status">
              Counts withheld: source traversal is incomplete.
            </p>
          )}
          <AnalysisNotes envelope={report.envelope} />
          <details>
            <summary>Input identity</summary>
            <p className="mono">{report.input_digest}</p>
          </details>
          {report.locations_truncated && (
            <p className="notice" role="status">
              Source locations are truncated; completed counts remain separate.
            </p>
          )}
          <label className="maintainability-kind">
            Source locations
            <select
              value={kind}
              onChange={(event) => {
                setKind(event.target.value as Kind);
                setOffset(0);
              }}
            >
              <option value="nesting">Syntax nesting</option>
              <option value="unsafe">Unsafe boundaries</option>
              <option value="lines">Noncomment code lines</option>
              <option value="unknowns">Unknown boundaries</option>
            </select>
          </label>
          <ul className="maintainability-sites">
            {sites.slice(offset, offset + pageSize).map((site, index) => (
              <li key={offset + index}>
                <button
                  type="button"
                  className="analysis-source"
                  onClick={() => onSpan(site.span)}
                  title={`Open source bytes ${site.span.start}..${site.span.end}`}
                >
                  <FileCode2 size={14} />
                  <span>{site.label}</span>
                  <span className="mono">
                    {site.span.start}..{site.span.end}
                  </span>
                </button>
              </li>
            ))}
          </ul>
          {!sites.length && <p className="muted">No returned locations.</p>}
          {sites.length > pageSize && (
            <nav
              className="maintainability-paging"
              aria-label="Metric source pages"
            >
              <button
                type="button"
                className="icon-button"
                aria-label="Previous metric locations"
                title="Previous metric locations"
                disabled={offset === 0}
                onClick={() => setOffset(Math.max(0, offset - pageSize))}
              >
                <ArrowLeft size={16} />
              </button>
              <span>
                {offset + 1}-{Math.min(offset + pageSize, sites.length)} /{" "}
                {sites.length}
              </span>
              <button
                type="button"
                className="icon-button"
                aria-label="Next metric locations"
                title="Next metric locations"
                disabled={offset + pageSize >= sites.length}
                onClick={() => setOffset(offset + pageSize)}
              >
                <ArrowRight size={16} />
              </button>
            </nav>
          )}
        </>
      )}
    </section>
  );
}
