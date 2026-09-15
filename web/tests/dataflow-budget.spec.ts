import { expect, test } from "@playwright/test";
import type {
  CompilerFlowPage,
  CompilerImportSummary,
  CompilerLocalEffects,
  ReachingDefinitions,
} from "../src/api/types";
import { coverage, mockApi } from "./fixtures";

const imported: CompilerImportSummary = {
  id: "compiler-import:budget-fixture",
  snapshot_id: "snapshot:two",
  context_id: "context:host",
  compiler: {
    adapter: "typed-fixture",
    adapter_version: "1",
    release: "fixture",
    commit_hash: "a".repeat(40),
    commit_date: "2026-09-15",
    host: "x86_64-unknown-linux-gnu",
    llvm_version: "fixture",
  },
  phase: "runtime_optimized",
  body_count: 1,
  mapped_count: 1,
  coverage,
};
const empty: CompilerLocalEffects = {
  defs: [],
  uses: [],
  moves: [],
  storage_live: [],
  storage_dead: [],
  unknown_effects: [],
};
const span = {
  status: "unavailable" as const,
  reason: "Synthetic UI contract fixture",
};
const compiler: CompilerFlowPage = {
  snapshot_id: imported.snapshot_id,
  context_id: imported.context_id,
  import_id: imported.id,
  definition_id: "definition:main",
  compiler: imported.compiler,
  phase: imported.phase,
  input_manifest_hash: "fixture-input",
  panic_strategy: "unwind",
  offset: 0,
  total_blocks: 1,
  next_offset: null,
  coverage,
  body: {
    body_id: "mir:fixture",
    def_path: "demo::main",
    kind: "function",
    span,
    argument_count: 1,
    locals: [
      {
        index: 0,
        role: "return",
        names: [],
        type_display: "u32",
        source_scope: 0,
        span,
      },
      {
        index: 1,
        role: "argument",
        names: ["input"],
        type_display: "u32",
        source_scope: 0,
        span,
      },
    ],
    source_scopes: [{ index: 0, parent: null, span, inlined_def_path: null }],
    blocks: [
      {
        index: 0,
        is_cleanup: false,
        statements: [
          {
            index: 0,
            kind: "assign",
            source_scope: 0,
            span,
            locals: { ...empty, defs: [0], uses: [1] },
          },
        ],
        terminator: {
          kind: "return",
          source_scope: 0,
          span,
          locals: { ...empty, uses: [0] },
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

for (const width of [1440, 390]) {
  test(`converged but withheld dataflow never claims zero reachable blocks at ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await mockApi(page);
    await page.route("**/v1/compiler?*", (route) =>
      route.fulfill({ json: [imported] }),
    );
    await page.route("**/v1/compiler/bodies/*", (route) =>
      route.fulfill({ json: compiler }),
    );
    let analysis: ReachingDefinitions = {
      body_id: compiler.body.body_id,
      fixed_point: true,
      iterations: 7,
      reachable_blocks: [],
      definitions: [],
      uses: [],
      unknown_memory_effects: [],
      envelope: {
        algorithm_version: "mir-reaching-definitions-v1",
        assumptions: ["Whole-local assignments only"],
        coverage: {
          ...coverage,
          status: "partial",
          limitations: [
            "Reaching-definition response exceeds the byte budget; claims withheld",
          ],
        },
        truncated: true,
        deadline_reached: false,
        cancelled: false,
      },
    };
    await page.route("**/v1/compiler/bodies/*/dataflow?*", (route) => {
      const params = new URL(route.request().url()).searchParams;
      expect(params.get("snapshot_id")).toBe(imported.snapshot_id);
      expect(params.get("context_id")).toBe(imported.context_id);
      expect(params.get("import_id")).toBe(imported.id);
      return route.fulfill({
        json: {
          api_version: "1",
          snapshot_id: imported.snapshot_id,
          context_id: imported.context_id,
          analysis,
        },
      });
    });
    await page.goto("/?view=flow#token=test-token");
    await expect(page).toHaveURL(/snapshot=snapshot%3Atwo/);
    if (width === 390)
      await page.getByRole("button", { name: "Show graph or flow" }).click();
    await page.getByLabel("Flow phase").selectOption(imported.id);
    await page.getByRole("button", { name: "Analyze locals" }).click();
    const panel = page.getByRole("region", { name: "Whole-local dataflow" });
    await expect(
      panel.getByText("Fixed point reached", { exact: true }),
    ).toHaveClass("badge partial");
    await expect(panel).toContainText("reachable-block records unavailable");
    await expect(panel).toContainText("Dataflow records were withheld");
    await expect(panel).not.toContainText("0 reachable blocks");
    await expect(panel).not.toContainText("Unknown memory effects (0)");
    await expect(panel).toContainText("Unknown memory effects (unavailable)");
    await expect(panel.getByLabel("Dataflow local")).toHaveCount(0);
    await expect(panel.locator(".dataflow-table")).toHaveCount(0);
    await panel
      .getByText("Dataflow assumptions and coverage", { exact: true })
      .click();
    await expect(panel).toContainText(
      "response exceeds the byte budget; claims withheld",
    );
    await panel.screenshot({
      path: test.info().outputPath("withheld-dataflow.png"),
    });

    analysis = {
      ...analysis,
      envelope: {
        ...analysis.envelope,
        coverage: {
          ...coverage,
          status: "partial",
          limitations: ["Reaching-definition output exceeds the fact budget"],
        },
      },
      reachable_blocks: [0],
      definitions: [{ local: 1, point: { kind: "entry" } }],
      uses: [
        {
          local: 1,
          point: { kind: "statement", block: 0, index: 0 },
          reaching: [{ local: 1, point: { kind: "entry" } }],
          may_be_uninitialized: false,
          possibly_changed_by_unknown_memory: false,
        },
      ],
    };
    await page.getByRole("button", { name: "Analyze locals" }).click();
    await expect(panel).toContainText("1 returned reachable block");
    await expect(panel).toContainText(
      "Unknown memory effects (0 returned records)",
    );
    await expect(panel).toContainText("Partial dataflow result");
    await expect(panel.locator(".dataflow-table tbody tr")).toHaveCount(1);
    await expect(panel).not.toContainText("Dataflow records were withheld");
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  });
}
