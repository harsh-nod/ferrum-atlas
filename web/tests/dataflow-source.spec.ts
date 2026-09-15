import { expect, test, type Page } from "@playwright/test";
import { EditorState } from "@codemirror/state";
import type {
  CompilerBody,
  CompilerFlowPage,
  CompilerImportSummary,
  CompilerLocalEffects,
  CompilerSourceMapping,
  ReachingDefinitions,
  SourceWindow,
} from "../src/api/types";
import { dataflowSource } from "../src/compiler-source";
import { byteToCharacter } from "../src/state";
import { coverage, mockApi, snapshots } from "./fixtures";

const text =
  "// caf\u00e9 \u{1f600}\r\nfn main(input: i32) {\r\n    let value = input;\r\n    let result = produce(value);\r\n    consume(result);\r\n}\r\n";
function span(fragment: string): CompilerSourceMapping {
  const start = text.indexOf(fragment);
  if (start < 0) throw new Error("Fixture fragment missing");
  return {
    status: "exact",
    path: "src/main.rs",
    start_byte: Buffer.byteLength(text.slice(0, start)),
    end_byte: Buffer.byteLength(text.slice(0, start + fragment.length)),
  };
}
const empty: CompilerLocalEffects = {
  defs: [],
  uses: [],
  moves: [],
  storage_live: [],
  storage_dead: [],
  unknown_effects: [],
};
const body: CompilerBody = {
  body_id: "body:main",
  def_path: "demo::main",
  kind: "function",
  span: span("fn main(input: i32)"),
  argument_count: 1,
  source_scopes: [],
  locals: [
    {
      index: 1,
      role: "argument",
      names: ["input"],
      type_display: "i32",
      source_scope: 0,
      span: span("input: i32"),
    },
    {
      index: 4,
      role: "temporary",
      names: [],
      type_display: "i32",
      source_scope: 0,
      span: span("consume(result);"),
    },
  ],
  blocks: [
    {
      index: 0,
      is_cleanup: false,
      statements: [
        {
          index: 0,
          kind: "Assign",
          source_scope: 0,
          span: span("let value = input;"),
          locals: { ...empty, defs: [2], uses: [1] },
        },
      ],
      terminator: {
        kind: "Call",
        source_scope: 0,
        span: span("let result = produce(value);"),
        locals: { ...empty, uses: [2], unknown_effects: ["call"] },
        normal_return_defs: [3],
        successors: [{ target: 1, kind: "normal", switch_value: null }],
        unwind: { kind: "continue" },
        call_target: null,
        assert_expected: null,
      },
    },
    {
      index: 1,
      is_cleanup: false,
      statements: [
        {
          index: 0,
          kind: "Assign",
          source_scope: 0,
          span: span("consume(result);"),
          locals: { ...empty, uses: [3] },
        },
        {
          index: 1,
          kind: "Assign",
          source_scope: 0,
          span: {
            status: "unavailable",
            reason: "Macro expansion source unavailable",
          },
          locals: { ...empty, uses: [4] },
        },
        {
          index: 2,
          kind: "Assign",
          source_scope: 0,
          span: {
            status: "exact",
            path: "generated/other.rs",
            start_byte: 0,
            end_byte: 5,
          },
          locals: { ...empty, uses: [5] },
        },
      ],
      terminator: {
        kind: "Return",
        source_scope: 0,
        span: span("}"),
        locals: empty,
        normal_return_defs: [],
        successors: [],
        unwind: null,
        call_target: null,
        assert_expected: null,
      },
    },
  ],
};
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
  phase: "optimized_mir",
  body_count: 1,
  mapped_count: 1,
  coverage,
};
const analysis: ReachingDefinitions = {
  envelope: {
    algorithm_version: "mir-reaching-definitions-v1",
    coverage,
    truncated: false,
    cancelled: false,
    deadline_reached: false,
    assumptions: [],
  },
  body_id: body.body_id,
  fixed_point: true,
  iterations: 2,
  reachable_blocks: [0, 1],
  definitions: [],
  unknown_memory_effects: [],
  uses: [
    {
      local: 1,
      point: { kind: "statement", block: 0, index: 0 },
      reaching: [{ local: 1, point: { kind: "entry" } }],
      may_be_uninitialized: false,
      possibly_changed_by_unknown_memory: false,
    },
    {
      local: 2,
      point: { kind: "terminator", block: 0 },
      reaching: [
        { local: 2, point: { kind: "statement", block: 0, index: 0 } },
      ],
      may_be_uninitialized: false,
      possibly_changed_by_unknown_memory: false,
    },
    {
      local: 3,
      point: { kind: "statement", block: 1, index: 0 },
      reaching: [
        { local: 3, point: { kind: "normal_return", block: 0, target: 1 } },
      ],
      may_be_uninitialized: false,
      possibly_changed_by_unknown_memory: true,
    },
    {
      local: 4,
      point: { kind: "statement", block: 1, index: 1 },
      reaching: [{ local: 4, point: { kind: "entry" } }],
      may_be_uninitialized: true,
      possibly_changed_by_unknown_memory: true,
    },
    {
      local: 5,
      point: { kind: "statement", block: 1, index: 2 },
      reaching: [],
      may_be_uninitialized: true,
      possibly_changed_by_unknown_memory: false,
    },
    {
      local: 6,
      point: { kind: "terminator", block: 77 },
      reaching: [],
      may_be_uninitialized: true,
      possibly_changed_by_unknown_memory: false,
    },
  ],
};
function flow(
  id = imported.id,
  snapshot = imported.snapshot_id,
): CompilerFlowPage {
  return {
    snapshot_id: snapshot,
    context_id: imported.context_id,
    import_id: id,
    definition_id: "definition:main",
    compiler,
    phase: "optimized_mir",
    input_manifest_hash: "input:fixture",
    panic_strategy: "unwind",
    body,
    offset: 0,
    total_blocks: 2,
    next_offset: null,
    coverage,
  };
}
function source(snapshot: string, binding = false): SourceWindow {
  const prefix = text.slice(0, text.indexOf("fn main"));
  return {
    snapshot_id: snapshot,
    file_id: "file:main",
    path: "src/main.rs",
    content_hash: "content:exact",
    text: binding ? prefix : text.slice(prefix.length),
    start_line: binding ? 1 : 2,
    start_byte: binding ? 0 : Buffer.byteLength(prefix),
    total_lines: 7,
    truncated: true,
    encoding: "utf-8",
  };
}
async function setup(page: Page) {
  await mockApi(page);
  await page.route("**/v1/compiler?*", (route) =>
    route.fulfill({ json: [imported, { ...imported, id: "compiler:two" }] }),
  );
  await page.route("**/v1/compiler/bodies/*", (route) => {
    const p = new URL(route.request().url()).searchParams;
    return route.fulfill({
      json: flow(p.get("import_id")!, p.get("snapshot_id")!),
    });
  });
  await page.route("**/v1/compiler/bodies/*/dataflow?*", (route) => {
    const p = new URL(route.request().url()).searchParams;
    return route.fulfill({
      json: {
        api_version: "1",
        snapshot_id: p.get("snapshot_id"),
        context_id: p.get("context_id"),
        analysis,
      },
    });
  });
  await page.route("**/v1/source/*", (route) => {
    const p = new URL(route.request().url()).searchParams;
    return route.fulfill({
      json: source(p.get("snapshot_id")!, p.get("lines") === "1"),
    });
  });
}
async function open(page: Page, mobile = false) {
  await page.goto("/?view=flow#token=test-token");
  await expect(page).toHaveURL(/snapshot=snapshot%3Atwo/);
  if (mobile)
    await page.getByRole("button", { name: "Show graph or flow" }).click();
  await page.getByLabel("Flow phase").selectOption(imported.id);
  await page.getByRole("button", { name: "Analyze locals" }).click();
  await expect(page.locator(".dataflow-table tbody tr")).toHaveCount(6);
}

test("structured points bind only supported effects and normal-return edges", () => {
  expect(dataflowSource(body, analysis.uses[0], "use")).toEqual(
    span("let value = input;"),
  );
  expect(dataflowSource(body, analysis.uses[1], "use")).toEqual(
    span("let result = produce(value);"),
  );
  expect(
    dataflowSource(body, analysis.uses[0].reaching[0], "definition"),
  ).toEqual(span("input: i32"));
  expect(
    dataflowSource(body, analysis.uses[2].reaching[0], "definition"),
  ).toEqual(span("let result = produce(value);"));
  const unsupported = [
    dataflowSource(undefined, analysis.uses[0], "use"),
    dataflowSource(
      body,
      { local: 3, point: { kind: "normal_return", block: 0, target: 99 } },
      "definition",
    ),
    dataflowSource(
      body,
      { local: 9, point: { kind: "statement", block: 0, index: 0 } },
      "definition",
    ),
    dataflowSource(
      body,
      { local: 2, point: { kind: "statement", block: 0, index: 99 } },
      "use",
    ),
    dataflowSource(body, analysis.uses[3].reaching[0], "definition"),
    dataflowSource(body, analysis.uses[5], "use"),
  ];
  expect(unsupported.every((mapping) => mapping.status === "unavailable")).toBe(
    true,
  );
  expect(dataflowSource(body, analysis.uses[3], "use")).toEqual(
    body.blocks[1].statements[1].span,
  );
});

test("compiler byte ranges select exact UTF-8 and CRLF source-window text", () => {
  const window = source(imported.snapshot_id);
  for (const [site, kind, expected] of [
    [analysis.uses[0], "use", "let value = input;"],
    [analysis.uses[0].reaching[0], "definition", "input: i32"],
    [
      analysis.uses[2].reaching[0],
      "definition",
      "let result = produce(value);",
    ],
  ] as const) {
    const mapping = dataflowSource(body, site, kind);
    expect(mapping.status).toBe("exact");
    if (mapping.status !== "exact") throw new Error("exact mapping required");
    const state = EditorState.create({ doc: window.text }).update({
      selection: {
        anchor: byteToCharacter(
          window.text,
          mapping.start_byte - window.start_byte,
        ),
        head: byteToCharacter(
          window.text,
          mapping.end_byte - window.start_byte,
        ),
      },
    }).state;
    expect(
      state.sliceDoc(state.selection.main.from, state.selection.main.to),
    ).toBe(expected);
  }
});

test("MIR point lookup uses recorded indices rather than array positions", () => {
  const sparse = structuredClone(body);
  sparse.blocks[0].index = 70;
  sparse.blocks[0].statements[0].index = 9;
  sparse.blocks[0].terminator.successors[0].target = 83;
  sparse.blocks[1].index = 83;
  sparse.blocks.reverse();
  expect(
    dataflowSource(
      sparse,
      { local: 1, point: { kind: "statement", block: 70, index: 9 } },
      "use",
    ),
  ).toEqual(span("let value = input;"));
  expect(
    dataflowSource(
      sparse,
      { local: 3, point: { kind: "normal_return", block: 70, target: 83 } },
      "definition",
    ),
  ).toEqual(span("let result = produce(value);"));
  sparse.blocks[1].terminator.successors[0].kind = "unwind";
  expect(
    dataflowSource(
      sparse,
      { local: 3, point: { kind: "normal_return", block: 70, target: 83 } },
      "definition",
    ).status,
  ).toBe("unavailable");
});

for (const width of [1440, 390]) {
  test(`dataflow use and definition jumps are exact and keyboard-accessible at ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await setup(page);
    const requests: number[] = [];
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (
        url.pathname.startsWith("/v1/source/") &&
        url.searchParams.get("lines") === "200"
      )
        requests.push(Number(url.searchParams.get("offset")));
    });
    await open(page, width === 390);
    const panel = page.getByRole("region", { name: "Whole-local dataflow" });
    await expect(
      panel.getByRole("button", { name: /^Show source for/ }),
    ).toHaveCount(6);
    await expect(panel).toContainText("Macro expansion source unavailable");
    await expect(panel).toContainText(
      "Entry source mapping unavailable for this local",
    );
    await expect(panel).toContainText("Source file binding unavailable");
    await expect(panel).toContainText(
      "Compiler block not in the loaded source window",
    );
    await panel
      .getByRole("button", { name: /^Show source for use _1/ })
      .focus();
    await page.keyboard.press("Tab");
    await expect(
      panel.getByRole("button", { name: /^Show source for definition _1/ }),
    ).toBeFocused();
    for (const [label, fragment, key] of [
      ["Show source for use _3 at bb1:0:", "consume(result);", "Enter"],
      [
        "Show source for definition _3 at bb0 -> bb1:return:",
        "let result = produce(value);",
        "Space",
      ],
      ["Show source for definition _1 at entry:", "input: i32", "Enter"],
    ]) {
      if (width === 390)
        await page.getByRole("button", { name: "Show graph or flow" }).click();
      const button = panel.getByRole("button", {
        name: new RegExp(`^${label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`),
      });
      await button.focus();
      await expect(button).toBeFocused();
      await page.keyboard.press(key);
      const mapping = span(fragment);
      if (mapping.status !== "exact") throw new Error("exact mapping required");
      await expect.poll(() => requests.at(-1)).toBe(mapping.start_byte);
      await expect(page.locator(".cm-activeLine")).toContainText(fragment);
      await expect(page.getByTestId("source-content")).toBeVisible();
    }
    if (width === 390)
      await page.getByRole("button", { name: "Show graph or flow" }).click();
    await panel
      .getByRole("button", { name: /^Show source for use _1/ })
      .scrollIntoViewIfNeeded();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: test.info().outputPath(`dataflow-source-${width}.png`),
    });
  });
}

test("a late dataflow source jump cannot overwrite a newer definition jump", async ({
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
  const mapping = span("consume(result);");
  if (mapping.status !== "exact") throw new Error("exact mapping required");
  await page.route("**/v1/source/*", async (route) => {
    const p = new URL(route.request().url()).searchParams;
    if (Number(p.get("offset")) !== mapping.start_byte) return route.fallback();
    requested();
    await pending;
    await route
      .fulfill({
        json: {
          ...source(imported.snapshot_id),
          text: "fn stale_response() {}",
          path: "stale.rs",
        },
      })
      .catch(() => {});
  });
  await open(page);
  await page.getByRole("button", { name: /^Show source for use _3/ }).click();
  await received;
  const cancelled = page.waitForEvent("requestfailed", {
    predicate: (request) =>
      new URL(request.url()).searchParams.get("offset") ===
      String(mapping.start_byte),
  });
  await page
    .getByRole("button", { name: /^Show source for definition _3/ })
    .click();
  await cancelled;
  await expect(page.locator(".cm-activeLine")).toContainText(
    "let result = produce(value);",
  );
  release();
  await expect(page.getByTestId("source-content")).not.toContainText(
    "stale_response",
  );
  await expect(page.locator(".source-pane .panel-heading")).toContainText(
    "src/main.rs",
  );
});

test("late dataflow results cannot attach source links to another compiler import", async ({
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
  await page.route("**/v1/compiler/bodies/*/dataflow?*", async (route) => {
    const p = new URL(route.request().url()).searchParams;
    if (p.get("import_id") !== imported.id)
      return route.fulfill({
        json: {
          api_version: "1",
          snapshot_id: imported.snapshot_id,
          context_id: imported.context_id,
          analysis: { ...analysis, uses: [analysis.uses[2]] },
        },
      });
    requested();
    await pending;
    await route
      .fulfill({
        json: {
          api_version: "1",
          snapshot_id: imported.snapshot_id,
          context_id: imported.context_id,
          analysis,
        },
      })
      .catch(() => {});
  });
  await page.goto("/?view=flow#token=test-token");
  await page.getByLabel("Flow phase").selectOption(imported.id);
  await page.getByRole("button", { name: "Analyze locals" }).click();
  await received;
  const cancelled = page.waitForEvent("requestfailed", {
    predicate: (request) =>
      request.url().includes("/dataflow?") &&
      new URL(request.url()).searchParams.get("import_id") === imported.id,
  });
  await page.getByLabel("Flow phase").selectOption("compiler:two");
  await cancelled;
  await expect(
    page.getByRole("button", { name: "Analyze locals" }),
  ).toBeEnabled();
  await expect(page.locator(".dataflow-table")).toHaveCount(0);
  await page.getByRole("button", { name: "Analyze locals" }).click();
  await expect(page.locator(".dataflow-table tbody tr")).toHaveCount(1);
  release();
  await expect(
    page.getByRole("button", { name: /^Show source for use _1/ }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: /^Show source for use _3/ }).click();
  await expect(page.locator(".cm-activeLine")).toContainText(
    "consume(result);",
  );
});

test("mismatched dataflow body and stale source snapshot withhold navigation", async ({
  page,
}) => {
  await setup(page);
  await page.route("**/v1/compiler/bodies/*/dataflow?*", (route) =>
    route.fulfill({
      json: {
        api_version: "1",
        snapshot_id: imported.snapshot_id,
        context_id: imported.context_id,
        analysis: { ...analysis, body_id: "body:unrelated" },
      },
    }),
  );
  await open(page);
  await expect(page.locator(".dataflow-table")).toContainText(
    "Compiler body does not match this dataflow result",
  );
  await expect(
    page.getByRole("button", { name: /^Show source for/ }),
  ).toHaveCount(0);
  for (const mismatch of [
    { snapshot_id: "snapshot:other" },
    { context_id: "context:other" },
  ]) {
    await page.route("**/v1/compiler/bodies/*/dataflow?*", (route) =>
      route.fulfill({
        json: {
          api_version: "1",
          snapshot_id: imported.snapshot_id,
          context_id: imported.context_id,
          analysis,
          ...mismatch,
        },
      }),
    );
    await page.getByRole("button", { name: "Analyze locals" }).click();
    await expect(page.locator(".dataflow-table")).toContainText(
      "Compiler body does not match this dataflow result",
    );
    await expect(
      page.getByRole("button", { name: /^Show source for/ }),
    ).toHaveCount(0);
  }
  await page.route("**/v1/source/*", (route) =>
    route.fulfill({ json: source("snapshot:one", true) }),
  );
  await page.reload();
  await page.getByLabel("Flow phase").selectOption(imported.id);
  await expect(page.locator(".compiler-block-table")).toContainText(
    "Source file binding unavailable",
  );
  await expect(
    page.locator(".compiler-block-table button.compiler-source-link"),
  ).toHaveCount(0);
});

test("switching snapshots cancels a pending dataflow source selection", async ({
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
  const mapping = span("consume(result);");
  if (mapping.status !== "exact") throw new Error("exact mapping required");
  await page.route("**/v1/source/*", async (route) => {
    const p = new URL(route.request().url()).searchParams;
    if (p.get("snapshot_id") === "snapshot:one")
      return route.fulfill({
        json: {
          ...source("snapshot:one"),
          text: "fn restored_snapshot() {}",
          start_byte: 0,
          start_line: 1,
        },
      });
    if (Number(p.get("offset")) !== mapping.start_byte) return route.fallback();
    requested();
    await pending;
    await route
      .fulfill({
        json: { ...source("snapshot:two"), text: "fn stale_snapshot() {}" },
      })
      .catch(() => {});
  });
  await open(page);
  await page.getByRole("button", { name: /^Show source for use _3/ }).click();
  await received;
  const cancelled = page.waitForEvent("requestfailed", {
    predicate: (request) =>
      new URL(request.url()).searchParams.get("offset") ===
      String(mapping.start_byte),
  });
  await page
    .getByLabel("Snapshot", { exact: true })
    .selectOption("snapshot:one");
  await cancelled;
  await expect(page).toHaveURL(/snapshot=snapshot%3Aone/);
  await expect(page.getByTestId("source-content")).toContainText(
    "restored_snapshot",
  );
  release();
  await expect(page.getByTestId("source-content")).not.toContainText(
    "stale_snapshot",
  );
  await expect(page.locator(".dataflow-table")).toHaveCount(0);
});
