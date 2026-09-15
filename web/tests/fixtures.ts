import type { Page } from "@playwright/test";
import type {
  Capabilities,
  Coverage,
  Definition,
  DefinitionDetail,
  DiffResponse,
  Evidence,
  FunctionFlow,
  GraphResponse,
  QueryResponse,
  Relation,
  Snapshot,
  SourceWindow,
} from "../src/api/types";

export const sourceText =
  "// UTF-8: caf\u00e9\nfn main() {\n    handle();\n}\n\nfn handle() {\n    callback();\n}\n";
const byte = (character: number) =>
  Buffer.byteLength(sourceText.slice(0, character));
const span = (value: string) => {
  const start = sourceText.indexOf(value);
  return {
    file_id: "file:main",
    start: byte(start),
    end: byte(start + value.length),
  };
};
export const coverage: Coverage = {
  status: "partial",
  reasons: [{ reason: "indirect_target_unknown", count: 1 }],
  limitations: [
    "An unresolved callback remains. Runtime paths are not exhaustive.",
  ],
};
export const evidence: Evidence = {
  id: "evidence:resolver",
  basis: "semantic",
  producer: "reviewed-fixture:1",
  inputs: ["source:two", "context:host"],
  assumptions: ["Selected host configuration"],
  limitations: ["Indirect targets remain unknown"],
};
export const definitions: Definition[] = ["main", "handle"].map((name) => ({
  id: `definition:${name}`,
  context_id: "context:host",
  file_id: "file:main",
  name,
  qualified_name: `demo::${name}`,
  kind: "function",
  parent_id: null,
  signature: `fn ${name}()`,
  signature_hash: `signature:${name}`,
  body_hash: `body:${name}`,
  span: span(`fn ${name}()`),
  body_span: null,
  cfg: [],
  cfg_status: "active",
  visibility: "private",
  metrics: { lines: 3, branches: 0, returns: 0, awaits: 0, unsafe_blocks: 0 },
}));
export const relations: Relation[] = [
  {
    id: "relation:main-handle",
    source: "definition:main",
    target: { kind: "resolved", id: "definition:handle" },
    kind: "calls",
    span: span("handle();"),
    evidence_id: evidence.id,
  },
  {
    id: "relation:callback",
    source: "definition:handle",
    target: {
      kind: "unknown",
      reason: "indirect_target_unknown",
      label: "callback",
    },
    kind: "calls",
    span: span("callback();"),
    evidence_id: evidence.id,
  },
];
export const snapshots: Snapshot[] = ["two", "one"].map((revision) => ({
  id: `snapshot:${revision}`,
  source_id: `source:${revision}`,
  repository_id: "repository:demo",
  revision,
  created_at: revision === "two" ? "1789387200" : "1789300800",
  file_count: 1,
  definition_count: 2,
  relation_count: 2,
  producer: "reviewed-fixture:1",
  fact_digest: `facts:${revision}`,
  coverage: structuredClone(coverage),
  context: {
    id: "context:host",
    name: "host-default",
    target: "x86_64-unknown-linux-gnu",
    features: [],
    default_features: true,
    cfg: {},
    crates: [
      {
        name: "demo",
        root_file: "src/main.rs",
        edition: "2021",
        dependencies: {},
      },
    ],
    manifest_digest: "manifest:demo",
    trust: "read_only",
    coverage: { status: "complete", reasons: [], limitations: [] },
  },
}));
export const graph: GraphResponse = {
  api_version: "1",
  snapshot_id: snapshots[0].id,
  context_id: snapshots[0].context.id,
  nodes: definitions,
  edges: relations,
  coverage: structuredClone(coverage),
  page: { truncated: false, next_cursor: null },
  work: { deadline_reached: false, elapsed_ms: 3 },
};

export async function mockApi(
  page: Page,
  options: { graph?: GraphResponse; sourceDelay?: number } = {},
) {
  await page.route("**/v1/**", async (route) => {
    const url = new URL(route.request().url());
    if (route.request().headers().authorization !== "Bearer test-token")
      return route.fulfill({
        status: 401,
        json: {
          code: "not_authorized",
          message: "A session token is required",
          correlation_id: "request:test",
        },
      });
    const current =
      snapshots.find(
        (item) => item.id === url.searchParams.get("snapshot_id"),
      ) ?? snapshots[0];
    if (url.pathname === "/v1/capabilities") {
      const body: Capabilities = {
        api_version: "1",
        schema_version: 1,
        analysis_levels: ["syntax", "semantic"],
        graph_families: ["calls"],
        trust_modes: ["read_only"],
        limitations: ["Compiler CFG not captured"],
      };
      return route.fulfill({ json: body });
    }
    if (url.pathname === "/v1/snapshots")
      return route.fulfill({ json: snapshots });
    if (/^\/v1\/snapshots\/[^/]+\/prepare$/.test(url.pathname)) {
      return route.fulfill({
        json: {
          snapshot_id: decodeURIComponent(url.pathname.split("/")[3]),
          context_id: url.searchParams.get("context_id"),
          ready: true,
        },
      });
    }
    if (/^\/v1\/snapshots\/[^/]+\/(unpin|pin)$/.test(url.pathname)) {
      const body = route.request().postDataJSON();
      return route.fulfill({
        json: {
          snapshot_id: decodeURIComponent(url.pathname.split("/")[3]),
          context_id: body.context_id,
          name: body.name,
          retained: url.pathname.endsWith("/pin"),
        },
      });
    }
    if (url.pathname.startsWith("/v1/snapshots/")) {
      const id = decodeURIComponent(url.pathname.split("/").at(-1)!);
      const selected = snapshots.find((item) => item.id === id);
      return selected
        ? route.fulfill({ json: selected })
        : route.fulfill({
            status: 404,
            json: {
              code: "unknown_snapshot",
              message: "Snapshot unavailable",
              correlation_id: "request:snapshot",
            },
          });
    }
    if (url.pathname === "/v1/search") {
      const query = url.searchParams.get("q") ?? "";
      const body: QueryResponse<Definition> = {
        api_version: "1",
        snapshot_id: current.id,
        context_id: current.context.id,
        items: definitions.filter((item) => item.name.startsWith(query)),
        coverage: graph.coverage,
        page: graph.page,
        work: graph.work,
      };
      return route.fulfill({ json: body });
    }
    if (url.pathname.startsWith("/v1/definitions/")) {
      const id = decodeURIComponent(url.pathname.split("/").at(-1)!);
      const definition = definitions.find((item) => item.id === id);
      if (!definition)
        return route.fulfill({
          status: 404,
          json: {
            code: "not_found",
            message: "Definition unavailable",
            correlation_id: "request:def",
          },
        });
      const body: DefinitionDetail = {
        snapshot_id: current.id,
        definition,
        evidence: [evidence],
        coverage: graph.coverage,
      };
      return route.fulfill({ json: body });
    }
    if (url.pathname.startsWith("/v1/source/")) {
      if (options.sourceDelay)
        await new Promise((resolve) =>
          setTimeout(resolve, options.sourceDelay),
        );
      const body: SourceWindow = {
        snapshot_id: current.id,
        file_id: "file:main",
        path: "src/main.rs",
        content_hash: "content:fixture",
        text: sourceText,
        start_line: 1,
        total_lines: sourceText.split("\n").length,
        start_byte: 0,
        truncated: false,
        encoding: "utf-8",
      };
      return route.fulfill({ json: body });
    }
    if (url.pathname === "/v1/graph/neighborhood") {
      const body = route.request().postDataJSON();
      return route.fulfill({
        json: {
          ...(options.graph ?? graph),
          snapshot_id: body.snapshot_id,
          context_id: body.context_id,
        },
      });
    }
    if (url.pathname.startsWith("/v1/evidence/"))
      return route.fulfill({ json: evidence });
    if (url.pathname.startsWith("/v1/flow/")) {
      const body: FunctionFlow = {
        definition_id: "definition:main",
        phase: "source",
        points: [{ kind: "call", label: "handle()", span: relations[0].span }],
        coverage: graph.coverage,
      };
      return route.fulfill({ json: body });
    }
    if (url.pathname === "/v1/diff") {
      const input = route.request().postDataJSON();
      const body: DiffResponse = {
        before: input.before,
        after: input.after,
        context_changed: false,
        changes: [
          {
            kind: "modified",
            before: definitions[0],
            after: { ...definitions[0], body_hash: "body:changed" },
            correspondence: "exact path and signature",
            changed_fields: ["body"],
          },
        ],
        impact_candidates: [definitions[1]],
        coverage: graph.coverage,
        truncated: false,
      };
      return route.fulfill({ json: body });
    }
    if (url.pathname === "/v1/observations") return route.fulfill({ json: [] });
    if (url.pathname === "/v1/compiler") return route.fulfill({ json: [] });
    return route.fulfill({
      status: 404,
      json: {
        code: "unsupported",
        message: "Fixture endpoint unavailable",
        correlation_id: "request:fixture",
      },
    });
  });
}
