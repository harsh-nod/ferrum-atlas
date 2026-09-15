import { expect, test, type Page } from "@playwright/test";
import type {
  AnalysisResponse,
  CompilerComplexity,
  CompilerFlowPage,
  CompilerImportSummary,
} from "../src/api/types";
import {
  coverage,
  definitions,
  mockApi,
  relations,
  snapshots,
} from "./fixtures";

const compiler = {
  adapter: "fixture",
  adapter_version: "1",
  release: "fixture-nightly",
  commit_hash: "c".repeat(40),
  commit_date: "2026-09-01",
  host: "x86_64-unknown-linux-gnu",
  llvm_version: "test",
};
const imported: CompilerImportSummary = {
  id: "compiler:one",
  snapshot_id: snapshots[0].id,
  context_id: snapshots[0].context.id,
  compiler,
  phase: "runtime_optimized",
  body_count: 1,
  mapped_count: 1,
  coverage,
};
const mapping = {
  status: "exact" as const,
  path: "src/main.rs",
  start_byte: relations[0].span.start,
  end_byte: relations[0].span.end,
};
function flow(importId = imported.id): CompilerFlowPage {
  return {
    snapshot_id: imported.snapshot_id,
    context_id: imported.context_id,
    import_id: importId,
    definition_id: definitions[0].id,
    compiler,
    phase: imported.phase,
    input_manifest_hash: "manifest:fixture",
    panic_strategy: "unwind",
    offset: 0,
    total_blocks: 1,
    next_offset: null,
    coverage,
    body: {
      body_id: "body:main",
      def_path: "demo::main",
      kind: "function",
      span: mapping,
      argument_count: 0,
      locals: [],
      source_scopes: [],
      blocks: [
        {
          index: 0,
          is_cleanup: false,
          statements: [],
          terminator: {
            kind: "return",
            span: mapping,
            source_scope: 0,
            locals: {
              defs: [],
              uses: [],
              moves: [],
              storage_live: [],
              storage_dead: [],
              unknown_effects: [],
            },
            normal_return_defs: [],
            successors: [],
            unwind: null,
            call_target: null,
            assert_expected: null,
          },
        },
      ],
    },
  };
}
function measure(): CompilerComplexity {
  return {
    envelope: {
      algorithm_version: "compiler-cyclomatic-v1",
      coverage: {
        ...coverage,
        limitations: [
          "Structural measure, not execution feasibility, termination, or code quality",
        ],
      },
      truncated: false,
      cancelled: false,
      deadline_reached: false,
      assumptions: [
        "Only entry-reachable runtime successor edges participate",
        "A synthetic exit connects terminal and external unwind alternatives",
      ],
    },
    input_digest: "input:complexity-fixture",
    definition_id: definitions[0].id,
    import_id: imported.id,
    body_id: "body:main",
    compiler,
    phase: imported.phase,
    input_manifest_hash: "manifest:fixture",
    panic_strategy: "unwind",
    body_span: mapping,
    formula: "E - N + 2P",
    metrics: {
      nodes: 2,
      edges: 1,
      components: 1,
      cyclomatic: 1,
      runtime_blocks: 1,
      runtime_edges: 0,
      synthetic_exit_edges: 1,
      excluded_imaginary_edges: 0,
    },
    locations_truncated: false,
    reachable_blocks: [0],
    unreachable_blocks: [],
    exit_sites: [{ block: 0, kind: "return", span: mapping }],
  };
}
function response(analysis = measure()): AnalysisResponse<CompilerComplexity> {
  return {
    api_version: "1",
    snapshot_id: imported.snapshot_id,
    context_id: imported.context_id,
    analysis,
  };
}
async function setup(page: Page, report = response()) {
  const requests: URL[] = [];
  await mockApi(page);
  await page.route("**/v1/compiler?*", (route) =>
    route.fulfill({ json: [imported, { ...imported, id: "compiler:two" }] }),
  );
  await page.route("**/v1/compiler/bodies/*", (route) =>
    route.fulfill({
      json: flow(new URL(route.request().url()).searchParams.get("import_id")!),
    }),
  );
  await page.route("**/v1/compiler/bodies/*/complexity?*", (route) => {
    requests.push(new URL(route.request().url()));
    return route.fulfill({ json: report });
  });
  return requests;
}
async function open(page: Page, mobile = false) {
  await page.goto("/?view=flow#token=test-token");
  await expect(page).toHaveURL(/snapshot=snapshot%3Atwo/);
  if (mobile)
    await page.getByRole("button", { name: "Show graph or flow" }).click();
  await page.getByLabel("Flow phase").selectOption(imported.id);
  await expect(
    page.getByRole("button", { name: "Measure CFG", exact: true }),
  ).toBeEnabled();
}

for (const width of [1440, 390]) {
  test(`CFG measurement is opt-in, scoped, source-linked and keyboard accessible at ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    const requests = await setup(page);
    let dataflowRequests = 0;
    page.on("request", (request) => {
      if (request.url().includes("/dataflow?")) dataflowRequests++;
    });
    await open(page, width === 390);
    expect(requests).toHaveLength(0);
    const panel = page.getByRole("region", { name: "Compiler CFG complexity" });
    await panel
      .getByRole("button", { name: "Measure CFG", exact: true })
      .focus();
    await page.keyboard.press("Enter");
    await expect(panel.getByText("Cyclomatic", { exact: true })).toBeVisible();
    expect(requests).toHaveLength(1);
    expect(requests[0].searchParams.get("snapshot_id")).toBe(
      imported.snapshot_id,
    );
    expect(requests[0].searchParams.get("context_id")).toBe(
      imported.context_id,
    );
    expect(requests[0].searchParams.get("import_id")).toBe(imported.id);
    expect(dataflowRequests).toBe(0);
    await expect(page.locator(".dataflow-view")).toHaveCount(1);
    await expect(page.locator(".dataflow-table")).toHaveCount(0);
    await expect(panel).toContainText("E - N + 2P");
    await expect(panel).toContainText(
      "not execution feasibility, termination, or code quality",
    );
    await expect(panel.locator(".compiler-complexity-metrics dd")).toHaveText([
      "1",
      "2",
      "1",
      "1",
      "1",
      "0",
      "1",
      "0",
    ]);
    await panel
      .getByText("CFG measurement provenance", { exact: true })
      .click();
    await expect(panel).toContainText(compiler.commit_hash);
    await expect(panel).toContainText("input:complexity-fixture");
    await expect(panel).toContainText("manifest:fixture");
    const button = panel.getByRole("button", {
      name: /^Show CFG exit source at bb0/,
    });
    await button.focus();
    await expect(button).toBeFocused();
    await page.keyboard.press("Space");
    await expect(page.locator(".cm-activeLine")).toContainText("handle();");
    await expect(page.getByTestId("source-content")).toBeVisible();
    if (width === 390)
      await page.getByRole("button", { name: "Show graph or flow" }).click();
    await panel
      .getByText("Cyclomatic", { exact: true })
      .scrollIntoViewIfNeeded();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: test.info().outputPath(`compiler-complexity-${width}.png`),
    });
  });
}

test("withheld location arrays do not erase valid metrics or become zero totals", async ({
  page,
}) => {
  const report = measure();
  report.locations_truncated = true;
  report.reachable_blocks = [];
  report.unreachable_blocks = [];
  report.exit_sites = [];
  report.envelope.coverage.limitations.push(
    "Location records withheld by response byte budget",
  );
  await setup(page, response(report));
  await open(page);
  await page.getByRole("button", { name: "Measure CFG", exact: true }).click();
  const panel = page.getByRole("region", { name: "Compiler CFG complexity" });
  await expect(panel.getByText("Cyclomatic", { exact: true })).toBeVisible();
  await expect(panel).toContainText("CFG location records unavailable");
  await expect(panel).not.toContainText("Returned unreachable blocks");
  await expect(panel).not.toContainText("No exit-site records returned");
  await expect(panel.locator(".compiler-exit-table")).toHaveCount(0);
});

test("absent metrics cover both bounded work and graphs without an exit convention", async ({
  page,
}) => {
  const report = measure();
  report.metrics = null;
  report.reachable_blocks = [];
  report.unreachable_blocks = [];
  report.exit_sites = [];
  report.envelope.deadline_reached = true;
  report.envelope.truncated = true;
  await setup(page, response(report));
  await open(page);
  const panel = page.getByRole("region", { name: "Compiler CFG complexity" });
  await panel.getByRole("button", { name: "Measure CFG", exact: true }).click();
  await expect(panel).toContainText("CFG metrics unavailable");
  await expect(panel).toContainText("Deadline reached");
  await expect(panel.locator(".compiler-complexity-metrics")).toHaveCount(0);
  report.envelope.deadline_reached = false;
  report.envelope.truncated = false;
  report.reachable_blocks = [0];
  report.unreachable_blocks = [1];
  report.envelope.coverage.limitations = [
    "No terminal or external unwind endpoint available for synthetic-exit normalization",
  ];
  await page.route("**/v1/compiler/bodies/*/complexity?*", (route) =>
    route.fulfill({ json: response(report) }),
  );
  await panel.getByRole("button", { name: "Measure CFG", exact: true }).click();
  await expect(panel).toContainText("synthetic-exit normalization");
  await expect(panel).toContainText("CFG metrics unavailable");
  await expect(panel).not.toContainText("Deadline reached");
  await expect(panel).toContainText("Returned entry-reachable blocks");
  await expect(panel).toContainText("Returned unreachable blocks");
  await expect(
    panel
      .locator(".context-list")
      .filter({ hasText: "Returned entry-reachable blocks" })
      .locator("dd"),
  ).toHaveText(["1", "1"]);
  await expect(panel).toContainText("No exit-site records returned");
  await expect(panel).not.toContainText("CFG location records unavailable");
  await expect(panel.locator(".compiler-exit-table")).toHaveCount(0);
});

test("exit-site tables are bounded and unavailable source mappings stay explicit", async ({
  page,
}) => {
  const report = measure();
  report.metrics = {
    nodes: 102,
    edges: 201,
    components: 1,
    cyclomatic: 101,
    runtime_blocks: 101,
    runtime_edges: 100,
    synthetic_exit_edges: 101,
    excluded_imaginary_edges: 0,
  };
  report.reachable_blocks = Array.from({ length: 101 }, (_, index) => index);
  report.exit_sites = report.reachable_blocks.map((block) => ({
    block,
    kind: block === 100 ? "return" : "unwind_continue",
    span:
      block === 0
        ? { status: "unavailable", reason: "Generated exit has no source span" }
        : block === 100
          ? { ...mapping, path: "generated/other.rs" }
          : mapping,
  }));
  await setup(page, response(report));
  await open(page);
  await page.getByRole("button", { name: "Measure CFG", exact: true }).click();
  const panel = page.getByRole("region", { name: "Compiler CFG complexity" });
  await expect(panel.locator(".compiler-exit-table tbody tr")).toHaveCount(100);
  await expect(panel).toContainText("Generated exit has no source span");
  await panel.getByRole("button", { name: "Next CFG exit sites" }).click();
  await expect(panel.locator(".compiler-exit-table tbody tr")).toHaveCount(1);
  await expect(panel).toContainText("101-101 of 101 exit sites");
  await expect(panel).toContainText("Source file binding unavailable");
  await expect(
    panel.getByRole("button", { name: "Next CFG exit sites" }),
  ).toBeDisabled();
  await expect(panel.locator(".compiler-exit-table button")).toHaveCount(0);
  await panel.getByRole("button", { name: "Previous CFG exit sites" }).click();
  await expect(panel.locator(".compiler-exit-table tbody tr")).toHaveCount(100);
});

test("mismatched scope, body, import and compiler provenance never publish metrics or links", async ({
  page,
}) => {
  await setup(page);
  await open(page);
  const variants: AnalysisResponse<CompilerComplexity>[] = [
    { ...response(), snapshot_id: "snapshot:other" },
    { ...response(), context_id: "context:other" },
    response({ ...measure(), definition_id: "definition:other" }),
    response({ ...measure(), import_id: "compiler:other" }),
    response({ ...measure(), body_id: "body:other" }),
    response({
      ...measure(),
      compiler: { ...compiler, commit_hash: "d".repeat(40) },
    }),
    response({ ...measure(), phase: "source" }),
    response({ ...measure(), input_manifest_hash: "manifest:other" }),
    response({ ...measure(), panic_strategy: "abort" }),
  ];
  const panel = page.getByRole("region", { name: "Compiler CFG complexity" });
  for (const report of variants) {
    await page.route("**/v1/compiler/bodies/*/complexity?*", (route) =>
      route.fulfill({ json: report }),
    );
    await panel
      .getByRole("button", { name: "Measure CFG", exact: true })
      .click();
    await expect(panel.getByRole("alert")).toContainText(
      "does not match the selected compiler body and scope",
    );
    await expect(panel.locator(".compiler-complexity-metrics")).toHaveCount(0);
    await expect(
      panel.getByRole("button", { name: /^Show CFG exit source/ }),
    ).toHaveCount(0);
  }
});

test("changing imports aborts stale measurements and errors remain retryable", async ({
  page,
}) => {
  await setup(page);
  let release!: () => void;
  let requested!: () => void;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  const received = new Promise<void>((resolve) => {
    requested = resolve;
  });
  await page.route("**/v1/compiler/bodies/*/complexity?*", async (route) => {
    const id = new URL(route.request().url()).searchParams.get("import_id");
    if (id !== imported.id)
      return route.fulfill({
        status: 503,
        json: {
          code: "unavailable",
          message: "Measurement worker unavailable",
          correlation_id: "complexity:test",
        },
      });
    requested();
    await pending;
    await route.fulfill({ json: response() }).catch(() => {});
  });
  await open(page);
  const panel = page.getByRole("region", { name: "Compiler CFG complexity" });
  await panel.getByRole("button", { name: "Measure CFG", exact: true }).click();
  await received;
  await expect(
    panel.getByRole("button", { name: "Measure CFG", exact: true }),
  ).toBeDisabled();
  const cancelled = page.waitForEvent("requestfailed", {
    predicate: (request) => request.url().includes("/complexity?"),
  });
  await page.getByLabel("Flow phase").selectOption("compiler:two");
  await cancelled;
  await expect(
    panel.getByRole("button", { name: "Measure CFG", exact: true }),
  ).toBeEnabled();
  await expect(panel.locator(".compiler-complexity-metrics")).toHaveCount(0);
  release();
  await panel.getByRole("button", { name: "Measure CFG", exact: true }).click();
  await expect(panel.getByRole("alert")).toContainText(
    "Measurement worker unavailable",
  );
  await page.route("**/v1/compiler/bodies/*/complexity?*", (route) =>
    route.fulfill({
      json: response({ ...measure(), import_id: "compiler:two" }),
    }),
  );
  await panel.getByRole("button", { name: "Measure CFG", exact: true }).click();
  await expect(panel.getByText("Cyclomatic", { exact: true })).toBeVisible();
  await expect(panel.getByRole("alert")).toHaveCount(0);
});
