import { useState } from "react";
import { ArrowLeft, ArrowRight, Workflow } from "lucide-react";
import type {
  AnalysisResponse,
  Definition,
  MirPoint,
  ReachingDefinitions,
  Snapshot,
} from "../api/types";
import { params, request } from "../api/client";
import { useResource } from "../state";
import { CoverageNotice, ErrorNotice, IconButton, Loading } from "./common";

function point(value: MirPoint) {
  switch (value.kind) {
    case "entry":
      return "entry";
    case "statement":
      return `bb${value.block}:${value.index}`;
    case "terminator":
      return `bb${value.block}:terminator`;
    case "normal_return":
      return `bb${value.block} -> bb${value.target}:return`;
  }
}

export function DataflowView({
  snapshot,
  definition,
  importId,
}: {
  snapshot: Snapshot;
  definition: Definition;
  importId: string;
}) {
  const scope = `${snapshot.id}:${snapshot.context.id}:${definition.id}:${importId}`;
  const [run, setRun] = useState({ scope: "", count: 0 });
  const [window, setWindow] = useState({ scope: "", local: "", offset: 0 });
  const selected =
    window.scope === scope ? window : { scope, local: "", offset: 0 };
  const result = useResource<AnalysisResponse<ReachingDefinitions>>(
    run.scope === scope ? `${scope}:${run.count}` : "",
    (signal) =>
      request(
        `/compiler/bodies/${encodeURIComponent(definition.id)}/dataflow` +
          params({
            snapshot_id: snapshot.id,
            context_id: snapshot.context.id,
            import_id: importId,
          }),
        signal,
      ),
  );
  const data = result.data?.analysis;
  const partial = Boolean(
    data &&
    (data.envelope.truncated ||
      data.envelope.deadline_reached ||
      data.envelope.cancelled),
  );
  const recordsWithheld = Boolean(
    partial &&
    data &&
    !data.reachable_blocks.length &&
    !data.definitions.length &&
    !data.uses.length,
  );
  const uses =
    data?.uses.filter(
      (use) => !selected.local || use.local === Number(selected.local),
    ) ?? [];
  const locals = [...new Set(data?.uses.map((use) => use.local))].sort(
    (a, b) => a - b,
  );
  return (
    <section className="dataflow-view" aria-label="Whole-local dataflow">
      <div className="analysis-section-heading">
        <h3>Whole-local Dataflow</h3>
        <button
          type="button"
          className="analysis-source"
          disabled={result.loading}
          onClick={() => {
            setRun({ scope, count: run.count + 1 });
            setWindow({ scope, local: "", offset: 0 });
          }}
        >
          <Workflow size={15} /> Analyze locals
        </button>
      </div>
      {result.loading && <Loading label="Analyzing compiler locals" />}
      <ErrorNotice error={result.error} />
      {data && (
        <>
          <div className="analysis-section-heading">
            <span
              className={`badge ${data.fixed_point && !partial ? "complete" : "partial"}`}
            >
              {data.fixed_point
                ? "Fixed point reached"
                : "Fixed point unavailable"}
            </span>
            <span>
              {data.iterations} iterations /{" "}
              {recordsWithheld
                ? "reachable-block records unavailable"
                : `${data.reachable_blocks.length} returned reachable block${data.reachable_blocks.length === 1 ? "" : "s"}`}
            </span>
          </div>
          <details>
            <summary>Dataflow assumptions and coverage</summary>
            <p className="mono">{data.envelope.algorithm_version}</p>
            <CoverageNotice coverage={data.envelope.coverage} />
            <ul>
              {data.envelope.assumptions.map((assumption) => (
                <li key={assumption}>{assumption}</li>
              ))}
            </ul>
          </details>
          {partial && (
            <p role="status" className="badge partial">
              Partial dataflow result
            </p>
          )}
          {recordsWithheld && (
            <p className="muted">
              Dataflow records were withheld; reachability and memory-effect
              totals are unavailable.
            </p>
          )}
          {data.fixed_point && !recordsWithheld && (
            <>
              <label className="dataflow-local">
                Local
                <select
                  aria-label="Dataflow local"
                  value={selected.local}
                  onChange={(event) =>
                    setWindow({ scope, local: event.target.value, offset: 0 })
                  }
                >
                  <option value="">All locals</option>
                  {locals.map((local) => (
                    <option key={local} value={local}>
                      _{local}
                    </option>
                  ))}
                </select>
              </label>
              {uses.length === 0 ? (
                <p className="muted">No tracked uses in the returned result.</p>
              ) : (
                <>
                  <div className="analysis-table-scroll">
                    <table className="analysis-table dataflow-table">
                      <thead>
                        <tr>
                          <th>Local / use</th>
                          <th>Reaching whole-local definitions</th>
                          <th>Uncertainty</th>
                        </tr>
                      </thead>
                      <tbody>
                        {uses
                          .slice(selected.offset, selected.offset + 100)
                          .map((use, index) => (
                            <tr
                              key={`${use.local}:${point(use.point)}:${index}`}
                            >
                              <td>
                                <strong>_{use.local}</strong>
                                <br />
                                <code>{point(use.point)}</code>
                              </td>
                              <td>
                                {use.reaching.length
                                  ? use.reaching
                                      .slice(0, 100)
                                      .map((definition) => (
                                        <div key={point(definition.point)}>
                                          <code>{point(definition.point)}</code>
                                        </div>
                                      ))
                                  : "No tracked definition"}
                                {use.reaching.length > 100 && (
                                  <p className="badge partial">
                                    100 of {use.reaching.length} definitions
                                    shown
                                  </p>
                                )}
                              </td>
                              <td>
                                {use.may_be_uninitialized && (
                                  <p className="badge partial">
                                    Tracking gap on a path
                                  </p>
                                )}
                                {use.possibly_changed_by_unknown_memory && (
                                  <p className="badge partial">
                                    Unknown memory effects
                                  </p>
                                )}
                                {!use.may_be_uninitialized &&
                                  !use.possibly_changed_by_unknown_memory &&
                                  "None in this abstract domain"}
                              </td>
                            </tr>
                          ))}
                      </tbody>
                    </table>
                  </div>
                  <div className="compiler-pagination">
                    <span>
                      {selected.offset + 1}-
                      {Math.min(selected.offset + 100, uses.length)} of{" "}
                      {uses.length} uses
                    </span>
                    <IconButton
                      label="Previous dataflow uses"
                      disabled={selected.offset === 0}
                      onClick={() =>
                        setWindow({
                          ...selected,
                          offset: Math.max(0, selected.offset - 100),
                        })
                      }
                    >
                      <ArrowLeft size={15} />
                    </IconButton>
                    <IconButton
                      label="Next dataflow uses"
                      disabled={selected.offset + 100 >= uses.length}
                      onClick={() =>
                        setWindow({
                          ...selected,
                          offset: selected.offset + 100,
                        })
                      }
                    >
                      <ArrowRight size={15} />
                    </IconButton>
                  </div>
                </>
              )}
            </>
          )}
          <details>
            <summary>
              Unknown memory effects (
              {recordsWithheld
                ? "unavailable"
                : `${data.unknown_memory_effects.length} returned records`}
              )
            </summary>
            <ul>
              {data.unknown_memory_effects
                .slice(0, 100)
                .map((effect, index) => (
                  <li key={index}>
                    <code>{point(effect.point)}</code>:{" "}
                    {effect.effects.join(", ")}
                  </li>
                ))}
            </ul>
            {data.unknown_memory_effects.length > 100 && (
              <p className="badge partial">100 effects shown</p>
            )}
          </details>
        </>
      )}
    </section>
  );
}
