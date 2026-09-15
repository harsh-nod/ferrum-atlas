import { expect, test, type Page } from "@playwright/test";
import type { Core } from "cytoscape";
import { EditorState } from "@codemirror/state";
import { byteToCharacter, parseLocation } from "../src/state";
import { graphElements } from "../src/graph-model";
import {
  definitions,
  graph,
  mockApi,
  relations,
  snapshots,
  sourceText,
} from "./fixtures";
import type { ObservationWindow, TestOutcome } from "../src/api/types";

async function open(page: Page) {
  await page.goto("/#token=test-token");
  await expect(page.getByTestId("source-content")).toContainText("fn main()");
}

test("byte spans map UTF-8 into CodeMirror character offsets", () => {
  const character = sourceText.indexOf("handle();");
  expect(
    byteToCharacter(
      sourceText,
      Buffer.byteLength(sourceText.slice(0, character)),
    ),
  ).toBe(character);
  expect(byteToCharacter("a\u{1f600}b", 5)).toBe(3);
  expect(byteToCharacter("abc", 999)).toBe(3);
  expect(byteToCharacter("abc", -2)).toBe(0);
});

test("CRLF and bare CR source spans match normalized editor positions", () => {
  for (const source of [
    "fn main() {\r\n    caf\u00e9();\r\n}\r\n",
    "a\rb\rc",
    "\u{1f600}\r\nx",
  ]) {
    const state = EditorState.create({ doc: source });
    const end = byteToCharacter(source, Buffer.byteLength(source));
    expect(end).toBe(state.doc.length);
    expect(() => state.update({ selection: { anchor: end } })).not.toThrow();
    const prefix = source.slice(0, source.indexOf("\r") + 2);
    expect(byteToCharacter(source, Buffer.byteLength(prefix))).toBe(
      prefix.replace(/\r\n?/g, "\n").length,
    );
  }
});

test("deep links bound traversal and preserve the revision pin", () => {
  expect(
    parseLocation(
      "?snapshot=s&context=c&definition=d&edge=e&view=flow&depth=3&direction=incoming",
    ),
  ).toEqual({
    snapshot: "s",
    context: "c",
    definition: "d",
    edge: "e",
    view: "flow",
    depth: 3,
    direction: "incoming",
  });
  expect(
    parseLocation("?view=garbage&depth=999&direction=garbage"),
  ).toMatchObject({ view: "explore", depth: 2, direction: "both" });
});

test("unknown frontiers respect the total canvas node budget", () => {
  const dense = {
    ...graph,
    nodes: Array.from({ length: 200 }, (_, i) => ({
      ...definitions[0],
      id: `definition:${i}`,
    })),
    edges: Array.from({ length: 500 }, (_, i) => ({
      ...relations[1],
      id: `relation:${i}`,
      source: "definition:0",
    })),
  };
  const model = graphElements(dense, dense.edges);
  expect(model.nodes).toBe(200);
  expect(model.sites).toBe(0);
  expect(model.omitted).toBe(500);
  expect(dense.edges).toHaveLength(500);
  expect(
    model.elements.some((item) => item.data.id === "definition:199"),
  ).toBeTruthy();
});

test("opens pinned source and removes the token from the browser URL", async ({
  page,
}) => {
  await mockApi(page);
  await open(page);
  await expect(page).toHaveURL(/snapshot=snapshot%3Atwo/);
  await expect(page).toHaveURL(/context=context%3Ahost/);
  expect(new URL(page.url()).hash).toBe("");
  expect(
    await page.evaluate(() => sessionStorage.getItem("ferrum-atlas.session")),
  ).toBe("test-token");
  await expect(
    page.getByRole("button", { name: "Pin selection", exact: true }),
  ).toBeEnabled();
});

test("canvas pixels and actual edge clicks navigate to call sites", async ({
  page,
}) => {
  await mockApi(page);
  await open(page);
  const canvas = page.getByTestId("graph-canvas");
  await expect(canvas).toHaveAttribute("data-layout-state", "ready");
  await expect(canvas).toHaveAttribute("data-node-count", "3");
  const painted = await canvas.evaluate((element) =>
    Array.from(element.querySelectorAll("canvas")).reduce((sum, canvas) => {
      const context = canvas.getContext("2d");
      if (!context) return sum;
      const pixels = context.getImageData(
        0,
        0,
        canvas.width,
        canvas.height,
      ).data;
      for (let i = 3; i < pixels.length; i += 4) if (pixels[i] > 0) sum++;
      return sum;
    }, 0),
  );
  expect(painted).toBeGreaterThan(500);
  const point = await canvas.evaluate((element) =>
    (element as HTMLElement & { _cyreg: { cy: Core } })._cyreg.cy
      .getElementById("relation:main-handle")
      .renderedMidpoint(),
  );
  const bounds = (await canvas.boundingBox())!;
  const sourceRequest = page.waitForRequest(
    (request) =>
      request.url().includes("/v1/source/") &&
      new URL(request.url()).searchParams.get("offset") ===
        String(relations[0].span.start),
  );
  await page.mouse.click(bounds.x + point.x, bounds.y + point.y);
  await sourceRequest;
  await expect(page).toHaveURL(/edge=relation%3Amain-handle/);
});

test("relationship table exposes unknown boundaries and provenance", async ({
  page,
}) => {
  await mockApi(page);
  await open(page);
  await page.getByRole("button", { name: "Relationship table" }).click();
  await expect(page.getByRole("table")).toBeVisible();
  await page
    .getByRole("row")
    .filter({ hasText: "? callback" })
    .getByRole("button", { name: "Call site" })
    .click();
  await expect(
    page.getByText("Unknown boundary", { exact: true }),
  ).toBeVisible();
  await expect(page).toHaveURL(/edge=relation%3Acallback/);
  await expect(page.locator(".inspector-panel")).toContainText(
    "reviewed-fixture:1",
  );
  await page.getByLabel("Unknown targets").uncheck();
  await expect(
    page.getByRole("row").filter({ hasText: "? callback" }),
  ).toHaveCount(0);
});

test("search and browser history preserve the selected definition", async ({
  page,
}) => {
  await mockApi(page);
  await open(page);
  await page.getByRole("searchbox", { name: "Search symbols" }).fill("handle");
  await expect(page.locator(".symbol-row")).toHaveCount(1);
  await page.locator(".symbol-row").click();
  await expect(page).toHaveURL(/definition=definition%3Ahandle/);
  await page.getByRole("button", { name: "Flow", exact: true }).click();
  await expect(page.getByText("Phase: source")).toBeVisible();
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Explore", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  await expect(page).toHaveURL(/definition=definition%3Ahandle/);
});

test("bookmarks retain notes and export revision-pinned reading trails", async ({
  page,
}) => {
  await mockApi(page);
  await open(page);
  await page
    .getByLabel("Review note", { exact: true })
    .fill("Callback ownership remains open.");
  await page
    .getByRole("button", { name: "Pin selection", exact: true })
    .click();
  await page.reload();
  await expect(page.getByLabel("Review note", { exact: true })).toHaveValue(
    "Callback ownership remains open.",
  );
  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export reading trail" }).click();
  expect((await download).suggestedFilename()).toBe(
    "ferrum-atlas-reading-trail.json",
  );
  const saved = await page.evaluate(() =>
    localStorage.getItem("ferrum-atlas.bookmarks"),
  );
  expect(saved).toContain("snapshot:two");
  expect(saved).not.toContain("test-token");
  await page.getByLabel("Expansion depth").selectOption("3");
  await expect(
    page.getByRole("button", { name: "Pin selection", exact: true }),
  ).toBeVisible();
});

test("traversal settings survive reload", async ({ page }) => {
  await mockApi(page);
  await open(page);
  await page.getByLabel("Expansion depth").selectOption("3");
  await page.getByLabel("Relationship direction").selectOption("incoming");
  await page.reload();
  await expect(page.getByLabel("Expansion depth")).toHaveValue("3");
  await expect(page.getByLabel("Relationship direction")).toHaveValue(
    "incoming",
  );
});

test("unavailable snapshot, context, and edge pins never substitute source", async ({
  page,
}) => {
  await mockApi(page, {
    graph: {
      ...graph,
      edges: [],
      page: { truncated: true, next_cursor: null },
    },
  });
  await page.goto(
    "/?snapshot=snapshot:gone&context=context:host#token=test-token",
  );
  await expect(page.getByRole("alert")).toContainText(
    "pinned snapshot is unavailable",
  );
  await expect(page.getByTestId("source-content")).toHaveCount(0);
  await page.goto("/?snapshot=snapshot:two&context=context:wrong");
  await expect(page.getByRole("alert")).toContainText("does not match");
  await expect(page.getByTestId("source-content")).toHaveCount(0);
  await page.goto(
    "/?snapshot=snapshot:two&context=context:host&definition=definition:main&edge=relation:main-handle",
  );
  await expect(page.getByRole("alert")).toContainText(
    "pinned call site is unavailable",
  );
  await expect(page.getByTestId("source-content")).toHaveCount(0);
});

test("loading and authentication failures remain distinct from empty results", async ({
  page,
}) => {
  await mockApi(page, { sourceDelay: 700 });
  await page.goto("/#token=test-token");
  await expect(page.getByText("Loading source", { exact: true })).toBeVisible();
  await expect(page.getByTestId("source-content")).toBeVisible();
  await page.goto("/#token=wrong-token");
  await expect(page.getByRole("alert").first()).toContainText("not authorized");
  await page.getByLabel("New session token").fill("test-token");
  await page.getByRole("button", { name: "Reconnect" }).click();
  await expect(page.getByTestId("source-content")).toBeVisible();
});

test("changes, evidence and health display pinned API results", async ({
  page,
}) => {
  await mockApi(page);
  await open(page);
  await page.getByRole("button", { name: "Changes", exact: true }).click();
  await expect(page.getByText("1 changed definitions")).toBeVisible();
  await expect(
    page.getByText("Correspondence: exact path and signature"),
  ).toBeVisible();
  await expect(page.getByTestId("source-content")).toHaveCount(2);
  await page.getByRole("button", { name: "Evidence", exact: true }).click();
  await expect(page.locator(".main-workspace")).toContainText(
    "Selected host configuration",
  );
  await page.getByRole("button", { name: "Health", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Index Health" }),
  ).toBeVisible();
  await expect(page.locator(".main-workspace")).toContainText(
    "x86_64-unknown-linux-gnu",
  );
});

test("change baselines never select a different repository", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/v1/snapshots", (route) =>
    route.fulfill({
      json: [
        snapshots[0],
        {
          ...snapshots[0],
          id: "snapshot:foreign",
          repository_id: "repository:foreign",
        },
        snapshots[1],
      ],
    }),
  );
  await open(page);
  await page.getByRole("button", { name: "Changes", exact: true }).click();
  await expect(page.getByLabel("Base snapshot")).toHaveValue("snapshot:one");
  await expect(page.getByLabel("Base snapshot").locator("option")).toHaveCount(
    2,
  );
});

test("failed provenance stays an error rather than an empty evidence result", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/v1/evidence/**", (route) =>
    route.fulfill({
      status: 503,
      json: {
        code: "unavailable_evidence",
        message: "Unavailable fixture evidence",
        correlation_id: "request:unavailable",
      },
    }),
  );
  await page.goto(
    "/?snapshot=snapshot:two&context=context:host&definition=definition:main&edge=relation:main-handle&view=evidence#token=test-token",
  );
  await expect(
    page.locator(".main-workspace").getByRole("alert"),
  ).toContainText("Unavailable fixture evidence");
  await expect(
    page.getByText("No provenance records for this selection."),
  ).toHaveCount(0);
});

test("all eight outcomes and independent-clock decimal counters remain distinct", async ({
  page,
}) => {
  await mockApi(page);
  const outcomes: TestOutcome[] = [
    "pass",
    "fail",
    "expected_fail",
    "unexpected_pass",
    "timeout",
    "infrastructure_error",
    "not_run",
    "unknown",
  ];
  const window: ObservationWindow = {
    summary: {
      id: "observation:fixture",
      snapshot_id: snapshots[0].id,
      artifact: {
        sha256: "a".repeat(64),
        source_id: snapshots[0].source_id,
        context_id: snapshots[0].context.id,
        producer: "Observation fixture",
      },
      test_count: 8,
      event_count: 2,
      clock_domains: ["cpu-clock", "device-clock"],
      limitations: ["Synthetic display fixture"],
    },
    tests: outcomes.map((outcome) => ({
      name: outcome,
      outcome,
      elapsed_ns: "9007199254740993",
      timeout_ns: outcome === "timeout" ? "1000000000" : null,
      reason: "Recorded reason",
      definition_ids: [definitions[0].id],
    })),
    streams: ["cpu", "device"].map((id) => ({
      id,
      process_or_device: id + "-process",
      thread_or_hart: "7",
      clock_domain: id + "-clock",
      timestamp_unit: id === "cpu" ? "ns" : "cycles",
      events: [
        {
          sequence: "18446744073709551615",
          timestamp: id === "cpu" ? "9007199254740993" : "2",
          kind: "enter",
          definition_id: definitions[0].id,
          correlation_id: "recorded-correlation",
          loss_count: "9007199254740993",
        },
      ],
    })),
    offset: 0,
    next_offset: null,
    truncated: true,
  };
  await page.route("**/v1/observations**", (route) =>
    route.fulfill({
      json:
        new URL(route.request().url()).pathname === "/v1/observations"
          ? [window.summary]
          : window,
    }),
  );
  await open(page);
  await page.getByRole("button", { name: "Evidence", exact: true }).click();
  await page.getByRole("button", { name: /Observation fixture/ }).click();
  const rows = page.locator(".observation-table").first().locator("tbody tr");
  await expect(rows).toHaveCount(8);
  for (let i = 0; i < outcomes.length; i++)
    await expect(rows.nth(i).locator("td").nth(1)).toHaveText(
      outcomes[i].replaceAll("_", " "),
    );
  await expect(page.getByText("Bounded observation window")).toBeVisible();
  await expect(page.locator(".trace-stream").first()).toContainText(
    "18446744073709551615",
  );
  await expect(page.locator(".trace-stream").last()).toContainText(
    "device-clock",
  );
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  const link = page.locator(".observation-source").first();
  const href = await link.getAttribute("href");
  expect(href).toContain("snapshot=snapshot%3Atwo");
  expect(href).toContain("context=context%3Ahost");
});

test("mobile search keeps typing visible and drawers contain keyboard focus", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await mockApi(page);
  await open(page);
  const search = page.getByRole("searchbox", { name: "Search symbols" });
  await search.fill("handle");
  await expect(page.locator(".scope-panel")).not.toBeVisible();
  await search.press("Enter");
  await expect(
    page.getByRole("dialog", { name: "Scope and reading trail" }),
  ).toBeVisible();
  await expect(page.locator(".symbol-row")).toHaveCount(1);
  await page.keyboard.press("Shift+Tab");
  expect(
    await page
      .locator(".scope-panel")
      .evaluate((panel) => panel.contains(document.activeElement)),
  ).toBe(true);
  await page.keyboard.press("Escape");
  await expect(search).toBeFocused();
});

for (const viewport of [
  { width: 1440, height: 900 },
  { width: 1024, height: 768 },
  { width: 390, height: 844 },
  { width: 720, height: 450 },
]) {
  test(`workspace framing at ${viewport.width}x${viewport.height}`, async ({
    page,
  }, info) => {
    await page.setViewportSize(viewport);
    await mockApi(page);
    await open(page);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth + 1,
      ),
    ).toBe(true);
    await page.screenshot({
      path: info.outputPath("source.png"),
      fullPage: true,
    });
    if (viewport.width <= 760)
      await page.getByRole("button", { name: "Show graph or flow" }).click();
    await expect(page.getByTestId("graph-canvas")).toHaveAttribute(
      "data-layout-state",
      "ready",
    );
    const bounds = await page.getByTestId("graph-canvas").boundingBox();
    expect(bounds!.width).toBeGreaterThan(200);
    expect(bounds!.height).toBeGreaterThan(80);
    const visible = await page
      .getByTestId("graph-canvas")
      .evaluate((element) => {
        const cy = (element as HTMLElement & { _cyreg: { cy: Core } })._cyreg
          .cy;
        return cy.nodes().map((node) => node.renderedPosition());
      });
    for (const point of visible) {
      expect(point.x).toBeGreaterThan(0);
      expect(point.y).toBeGreaterThan(0);
      expect(point.x).toBeLessThan(bounds!.width);
      expect(point.y).toBeLessThan(bounds!.height);
    }
    await page.screenshot({
      path: info.outputPath("graph.png"),
      fullPage: true,
    });
    if (viewport.width <= 1150) {
      await page
        .getByRole("button", { name: "Evidence inspector", exact: true })
        .click();
      await expect(page.locator(".inspector-panel")).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(page.locator(".inspector-panel")).not.toBeVisible();
    }
  });
}

test("rapid selection changes discard outdated source responses", async ({
  page,
}) => {
  await mockApi(page);
  let releaseOld: (() => void) | undefined;
  let oldRequested: (() => void) | undefined;
  const receivedOld = new Promise<void>((resolve) => {
    oldRequested = resolve;
  });
  const pendingOld = new Promise<void>((resolve) => {
    releaseOld = resolve;
  });
  await page.route("**/v1/source/**", async (route) => {
    const offset = Number(
      new URL(route.request().url()).searchParams.get("offset"),
    );
    if (offset === definitions[0].span.start) {
      oldRequested?.();
      await pendingOld;
      await route
        .fulfill({
          json: {
            snapshot_id: "snapshot:two",
            file_id: "file:main",
            path: "old-response.rs",
            content_hash: "content:old",
            text: "fn stale_response() {}",
            start_line: 1,
            total_lines: 1,
            start_byte: 0,
            truncated: false,
            encoding: "utf-8",
          },
        })
        .catch(() => {});
      return;
    }
    await route.fallback();
  });
  await page.goto("/#token=test-token");
  await receivedOld;
  await page.locator(".symbol-row").filter({ hasText: "handle" }).click();
  await expect(page.getByTestId("source-content")).toContainText("fn handle()");
  releaseOld?.();
  await expect(page.getByTestId("source-content")).not.toContainText(
    "stale_response",
  );
  await expect(page).toHaveURL(/definition=definition%3Ahandle/);
});
