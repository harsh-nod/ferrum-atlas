import type {
  CompilerBody,
  CompilerSourceMapping,
  LocalDefinition,
} from "./api/types";

export function dataflowSource(
  body: CompilerBody | undefined,
  site: LocalDefinition,
  kind: "use" | "definition",
): CompilerSourceMapping {
  const unavailable = (reason: string): CompilerSourceMapping => ({
    status: "unavailable",
    reason,
  });
  if (!body)
    return unavailable("Compiler body does not match this dataflow result");
  const { point, local } = site;
  if (point.kind === "entry") {
    const argument = body.locals.find(
      (candidate) => candidate.index === local && candidate.role === "argument",
    );
    return kind === "definition" && argument
      ? argument.span
      : unavailable("Entry source mapping unavailable for this local");
  }
  const block = body.blocks.find(
    (candidate) => candidate.index === point.block,
  );
  if (!block)
    return unavailable("Compiler block not in the loaded source window");
  if (point.kind === "normal_return") {
    // A returned value is defined at the call, not at its successor's first use.
    return kind === "definition" &&
      block.terminator.normal_return_defs.includes(local) &&
      block.terminator.successors.some(
        (edge) => edge.kind === "normal" && edge.target === point.target,
      )
      ? block.terminator.span
      : unavailable("Normal-return source mapping unavailable for this local");
  }
  const operation =
    point.kind === "statement"
      ? block.statements.find((statement) => statement.index === point.index)
      : block.terminator;
  if (!operation)
    return unavailable("Compiler statement source mapping unavailable");
  const locals =
    kind === "definition"
      ? operation.locals.defs
      : [...operation.locals.uses, ...operation.locals.moves];
  return locals.includes(local)
    ? operation.span
    : unavailable("Compiler local effect does not match this dataflow point");
}
