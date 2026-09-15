import { expect, test, type Page } from "@playwright/test";
import type {
  CompilerFlowPage,
  CompilerImportSummary,
  CompilerLocalEffects,
  CompilerSourceMapping,
} from "../src/api/types";
import { coverage, mockApi, relations } from "./fixtures";

const identity = {
  adapter: "reviewed-fixture",
  adapter_version: "1",
  release: "nightly-test",
  commit_hash: "c".repeat(40),
  commit_date: "2026-09-01",
  host: "x86_64-unknown-linux-gnu",
  llvm_version: "test",
};
const imported: CompilerImportSummary = {
  id: "compiler:one",
  snapshot_id: "snapshot:two",
  context_id: "context:host",
  compiler: identity,
  phase: "optimized_mir",
  body_count: 1,
  mapped_count: 1,
  coverage,
};
const locals: CompilerLocalEffects = {
  defs: [0],
  uses: [1],
  moves: [],
  storage_live: [],
  storage_dead: [],
  unknown_effects: ["pointer_aliasing"],
};
const mapping: Extract<CompilerSourceMapping, { status: "exact" }> = {
  status: "exact",
  path: "src/main.rs",
  start_byte: relations[0].span.start,
  end_byte: relations[0].span.end,
};
function flow(offset = 0): CompilerFlowPage {
  return {
    snapshot_id: "snapshot:two",
    context_id: "context:host",
    import_id: imported.id,
    definition_id: "definition:main",
    compiler: identity,
    phase: "optimized_mir",
    input_manifest_hash: "input:exact",
    panic_strategy: "unwind",
    offset,
    total_blocks: 201,
    next_offset: offset === 0 ? 200 : null,
    coverage,
    body: {
      body_id: "body:main",
      def_path: "demo::main",
      kind: "function",
      span: mapping,
      argument_count: 1,
      locals: [
        {
          index: 0,
          role: "return",
          names: [],
          type_display: "u128",
          source_scope: 0,
          span: mapping,
        },
      ],
      source_scopes: [],
      blocks: [
        {
          index: offset,
          is_cleanup: offset > 0,
          statements: [
            {
              index: 0,
              kind: "Assign",
              source_scope: 0,
              span: { ...mapping, path: "generated/other.rs" },
              locals,
            },
          ],
          terminator: {
            kind: "SwitchInt",
            source_scope: 0,
            span: mapping,
            locals,
            normal_return_defs: [2],
            successors: [
              {
                target: 200,
                kind: "switch_value",
                switch_value: "340282366920938463463374607431768211455",
              },
            ],
            unwind: { kind: "cleanup", target: 200 },
            call_target: { kind: "indirect", reason: "Function pointer" },
            assert_expected: null,
          },
        },
      ],
    },
  };
}
async function open(page: Page) {
  await page.goto("/?view=flow#token=test-token");
  await expect(page).toHaveURL(/snapshot=snapshot%3Atwo/);
}

test("no compiler artifact keeps source phase available without claiming compiler CFG", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/v1/compiler?*", (route) => route.fulfill({ json: [] }));
  await open(page);
  await expect(page.getByLabel("Flow phase")).toHaveValue("");
  await expect(
    page.getByText("No compiler artifact imported for this snapshot."),
  ).toBeVisible();
  await expect(page.locator(".flow-view .phase-label")).toHaveText(
    "Phase: source",
  );
});

for (const width of [1440, 390]) {
  test(`compiler phase retains provenance, bounded pages, and exact source mappings at ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await mockApi(page);
    await page.route("**/v1/compiler?*", (route) =>
      route.fulfill({ json: [imported] }),
    );
    const offsets: number[] = [];
    await page.route("**/v1/compiler/bodies/*", async (route) => {
      const parameters = new URL(route.request().url()).searchParams;
      expect(parameters.get("import_id")).toBe(imported.id);
      expect(parameters.get("snapshot_id")).toBe("snapshot:two");
      expect(parameters.get("context_id")).toBe("context:host");
      expect(parameters.get("limit")).toBe("200");
      const offset = Number(parameters.get("offset"));
      offsets.push(offset);
      await route.fulfill({ json: flow(offset) });
    });
    await open(page);
    if (width === 390)
      await page.getByRole("button", { name: "Show graph or flow" }).click();
    await page.getByLabel("Flow phase").selectOption(imported.id);
    const provenance = page.locator(".compiler-flow-view > details").first();
    await expect(provenance).not.toHaveAttribute("open", "");
    await expect(
      page.locator(".compiler-flow-view > .analysis-section-heading .badge"),
    ).toHaveText("partial");
    await expect(
      page.getByRole("heading", { name: "Compiler Basic Blocks" }),
    ).toBeVisible();
    await expect(
      page.locator(".compiler-block-table tr").nth(1),
    ).toBeInViewport();
    await expect(page.locator(".compiler-block-table")).toContainText(
      "340282366920938463463374607431768211455",
    );
    await expect(page.locator(".compiler-block-table")).toContainText(
      "Unwind: cleanup to bb200",
    );
    await expect(page.locator(".compiler-block-table")).toContainText(
      "Normal-return defs: _2",
    );
    await page.locator(".compiler-block-table summary").click();
    await expect(page.locator(".compiler-statements")).toContainText(
      "generated/other.rs",
    );
    await expect(page.locator(".compiler-statements button")).toHaveCount(0);
    await expect(
      page.locator(".compiler-block-table .analysis-source"),
    ).toHaveCount(1);
    await page
      .getByRole("button", { name: "Next compiler block window" })
      .click();
    await expect(
      page.locator(".compiler-block-table td strong").first(),
    ).toHaveText("bb200");
    await expect(
      page.getByRole("button", { name: "Next compiler block window" }),
    ).toBeDisabled();
    await page
      .getByRole("button", { name: "Previous compiler block window" })
      .click();
    await expect(
      page.locator(".compiler-block-table td strong").first(),
    ).toHaveText("bb0");
    expect(
      offsets.filter(
        (value, index) => index === 0 || value !== offsets[index - 1],
      ),
    ).toEqual([0, 200, 0]);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBeTruthy();
    await page.screenshot({
      path: test.info().outputPath("compiler-flow.png"),
    });
    await page.locator(".compiler-block-table .analysis-source").click();
    if (width === 390)
      await page
        .getByRole("button", { name: "Show source", exact: true })
        .click();
    await expect(page.locator(".cm-activeLine")).toContainText("handle();");
  });
}

test("an unavailable selected compiler body never silently substitutes source flow", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/v1/compiler?*", (route) =>
    route.fulfill({ json: [imported] }),
  );
  await page.route("**/v1/compiler/bodies/*", (route) =>
    route.fulfill({
      status: 404,
      json: {
        code: "not_found",
        message: "No compiler body mapped to this definition",
        correlation_id: "compiler-test",
      },
    }),
  );
  await open(page);
  await page.getByLabel("Flow phase").selectOption(imported.id);
  await expect(page.locator(".compiler-flow-view [role=alert]")).toContainText(
    "No compiler body mapped",
  );
  await expect(page.locator(".flow-view .phase-label")).toHaveCount(0);
  await expect(page.getByLabel("Flow phase")).toHaveValue(imported.id);
  await page.getByLabel("Flow phase").selectOption("");
  await expect(page.locator(".flow-view .phase-label")).toHaveText(
    "Phase: source",
  );
});
