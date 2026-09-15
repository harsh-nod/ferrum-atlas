import { useState } from "react";
import { ArrowLeft, ArrowRight, FileCode2 } from "lucide-react";
import type {
  CompilerFlowPage,
  CompilerLocalEffects,
  CompilerSourceMapping,
  Definition,
  Snapshot,
  SourceWindow,
  Span,
} from "../api/types";
import { params, request } from "../api/client";
import { useResource } from "../state";
import { CoverageNotice, ErrorNotice, IconButton, Loading } from "./common";

function effects(value: CompilerLocalEffects) {
  return (
    (["defs", "uses", "moves", "storage_live", "storage_dead"] as const)
      .filter((key) => value[key].length > 0)
      .map(
        (key) =>
          `${key.replaceAll("_", " ")}: ${value[key]
            .slice(0, 200)
            .map((index) => `_${index}`)
            .join(
              ", ",
            )}${value[key].length > 200 ? ` (200 of ${value[key].length})` : ""}`,
      )
      .join("; ") || "No recorded local effects"
  );
}

export function CompilerFlowView({
  snapshot,
  definition,
  importId,
  onSpan,
}: {
  snapshot: Snapshot;
  definition?: Definition;
  importId: string;
  onSpan: (span: Span) => void;
}) {
  const scope = `${snapshot.id}:${snapshot.context.id}:${definition?.id}:${importId}`;
  const [window, setWindow] = useState({ scope: "", offset: 0 });
  const [openBlocks, setOpenBlocks] = useState(new Set<number>());
  const [localsOpen, setLocalsOpen] = useState(false);
  const offset = window.scope === scope ? window.offset : 0;
  const pin = { snapshot_id: snapshot.id, context_id: snapshot.context.id };
  const page = useResource<CompilerFlowPage>(
    definition ? `${scope}:${offset}` : "",
    (signal) =>
      request(
        `/compiler/bodies/${encodeURIComponent(definition!.id)}` +
          params({ ...pin, import_id: importId, offset, limit: 200 }),
        signal,
      ),
  );
  const source = useResource<SourceWindow>(
    definition
      ? `${snapshot.id}:${snapshot.context.id}:${definition.file_id}:compiler-source-path`
      : "",
    (signal) =>
      request(
        `/source/${encodeURIComponent(definition!.file_id)}` +
          params({ ...pin, offset: 0, lines: 1 }),
        signal,
      ),
  );
  const anchor = (mapping: CompilerSourceMapping) => {
    if (mapping.status === "unavailable")
      return <span className="muted">{mapping.reason}</span>;
    if (
      source.data?.path === mapping.path &&
      source.data.file_id === definition?.file_id
    )
      return (
        <button
          type="button"
          className="analysis-source"
          onClick={() =>
            onSpan({
              file_id: source.data!.file_id,
              start: mapping.start_byte,
              end: mapping.end_byte,
            })
          }
        >
          <FileCode2 size={13} />
          <span>
            {mapping.path}:{mapping.start_byte}..{mapping.end_byte}
          </span>
        </button>
      );
    return (
      <code>
        {mapping.path}:{mapping.start_byte}..{mapping.end_byte}
      </code>
    );
  };
  return (
    <div className="document-view compiler-flow-view">
      {!definition && <p className="muted">No definition selected.</p>}
      {page.loading && <Loading label="Loading compiler flow" />}
      <ErrorNotice error={page.error} />
      <ErrorNotice error={source.error} />
      {page.data && (
        <>
          <div className="analysis-section-heading">
            <h3>{page.data.phase}</h3>
            <span className={`badge ${page.data.coverage.status}`}>
              {page.data.coverage.status}
            </span>
          </div>
          <details>
            <summary>Compiler provenance and limitations</summary>
            <p>
              {page.data.compiler.release} / {page.data.compiler.host}
            </p>
            <dl className="context-list">
              <dt>Compiler commit</dt>
              <dd className="mono">{page.data.compiler.commit_hash}</dd>
              <dt>Adapter</dt>
              <dd>
                {page.data.compiler.adapter}{" "}
                {page.data.compiler.adapter_version}
              </dd>
              <dt>Input manifest</dt>
              <dd className="mono">{page.data.input_manifest_hash}</dd>
              <dt>Import</dt>
              <dd className="mono">{page.data.import_id}</dd>
              <dt>Panic strategy</dt>
              <dd>{page.data.panic_strategy}</dd>
              <dt>Body</dt>
              <dd className="mono">{page.data.body.def_path}</dd>
            </dl>
            <CoverageNotice coverage={page.data.coverage} />
          </details>
          <div className="analysis-section-heading">
            <h3>Compiler Basic Blocks</h3>
            <span>
              {page.data.body.blocks.length ? offset + 1 : 0}-
              {Math.min(
                offset + page.data.body.blocks.length,
                page.data.total_blocks,
              )}{" "}
              of {page.data.total_blocks}
            </span>
          </div>
          <div className="analysis-table-scroll">
            <table className="analysis-table compiler-block-table">
              <thead>
                <tr>
                  <th>Block</th>
                  <th>Terminator / successors</th>
                  <th>Local effects / source</th>
                </tr>
              </thead>
              <tbody>
                {page.data.body.blocks.map((block) => (
                  <tr key={block.index}>
                    <td>
                      <strong>bb{block.index}</strong>
                      {block.is_cleanup && (
                        <p className="badge partial">cleanup</p>
                      )}
                    </td>
                    <td>
                      <strong>{block.terminator.kind}</strong>
                      <ul>
                        {block.terminator.successors
                          .slice(0, 200)
                          .map((successor, index) => (
                            <li key={index}>
                              <code>bb{successor.target}</code>{" "}
                              {successor.kind.replaceAll("_", " ")}
                              {successor.switch_value !== null && (
                                <span className="mono">
                                  {" "}
                                  = {successor.switch_value}
                                </span>
                              )}
                            </li>
                          ))}
                      </ul>
                      {block.terminator.successors.length > 200 && (
                        <p className="muted">
                          200 of {block.terminator.successors.length} successors
                          shown.
                        </p>
                      )}
                      {block.terminator.unwind && (
                        <p>
                          Unwind: {block.terminator.unwind.kind}
                          {block.terminator.unwind.kind === "cleanup"
                            ? ` to bb${block.terminator.unwind.target}`
                            : block.terminator.unwind.kind === "terminate"
                              ? ` (${block.terminator.unwind.reason})`
                              : ""}
                        </p>
                      )}
                      {block.terminator.call_target && (
                        <p>
                          {block.terminator.call_target.kind ===
                          "function_definition"
                            ? block.terminator.call_target.def_path
                            : `Indirect target: ${block.terminator.call_target.reason}`}
                        </p>
                      )}
                      {block.terminator.normal_return_defs.length > 0 && (
                        <p>
                          Normal-return defs:{" "}
                          {block.terminator.normal_return_defs
                            .slice(0, 200)
                            .map((index) => `_${index}`)
                            .join(", ")}
                        </p>
                      )}
                    </td>
                    <td>
                      <p>{effects(block.terminator.locals)}</p>
                      {block.terminator.locals.unknown_effects.length > 0 && (
                        <p className="compiler-unknown">
                          Unknown:{" "}
                          {block.terminator.locals.unknown_effects
                            .map((effect) => effect.replaceAll("_", " "))
                            .join(", ")}
                        </p>
                      )}
                      {anchor(block.terminator.span)}
                      <details
                        onToggle={(event) => {
                          const isOpen = event.currentTarget.open;
                          setOpenBlocks((previous) => {
                            const next = new Set(previous);
                            if (isOpen) next.add(block.index);
                            else next.delete(block.index);
                            return next;
                          });
                        }}
                      >
                        <summary>
                          Statements ({block.statements.length})
                        </summary>
                        {openBlocks.has(block.index) && (
                          <ol className="compiler-statements">
                            {block.statements.slice(0, 200).map((statement) => (
                              <li key={statement.index}>
                                <strong>{statement.kind}</strong>
                                <p>{effects(statement.locals)}</p>
                                {statement.locals.unknown_effects.length >
                                  0 && (
                                  <p>
                                    Unknown:{" "}
                                    {statement.locals.unknown_effects.join(
                                      ", ",
                                    )}
                                  </p>
                                )}
                                {anchor(statement.span)}
                              </li>
                            ))}
                          </ol>
                        )}
                        {block.statements.length > 200 && (
                          <p className="muted">
                            200 of {block.statements.length} statements shown.
                          </p>
                        )}
                      </details>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="compiler-pagination">
            <IconButton
              label="Previous compiler block window"
              disabled={offset === 0}
              onClick={() =>
                setWindow({ scope, offset: Math.max(0, offset - 200) })
              }
            >
              <ArrowLeft size={15} />
            </IconButton>
            <IconButton
              label="Next compiler block window"
              disabled={page.data.next_offset === null}
              onClick={() =>
                setWindow({ scope, offset: page.data!.next_offset! })
              }
            >
              <ArrowRight size={15} />
            </IconButton>
          </div>
          <details
            onToggle={(event) => setLocalsOpen(event.currentTarget.open)}
          >
            <summary>Compiler locals ({page.data.body.locals.length})</summary>
            {localsOpen && (
              <div className="analysis-table-scroll">
                <table className="analysis-table">
                  <thead>
                    <tr>
                      <th>Local</th>
                      <th>Role / names</th>
                      <th>Compiler type display</th>
                      <th>Source</th>
                    </tr>
                  </thead>
                  <tbody>
                    {page.data.body.locals.slice(0, 200).map((local) => (
                      <tr key={local.index}>
                        <td>_{local.index}</td>
                        <td>
                          {local.role} / {local.names.join(", ")}
                        </td>
                        <td>
                          <code>{local.type_display}</code>
                        </td>
                        <td>{anchor(local.span)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
            {page.data.body.locals.length > 200 && (
              <p className="muted">
                200 of {page.data.body.locals.length} locals shown.
              </p>
            )}
          </details>
        </>
      )}
    </div>
  );
}
