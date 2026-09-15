import { expect, test, type Page } from "@playwright/test";
import { definitions, evidence, graph, mockApi, snapshots } from "./fixtures";
import type { SourceMaintainability } from "../src/api/types";

const span = definitions[0].span;
const report: SourceMaintainability = {
  envelope: {
    algorithm_version: "source-maintainability/test",
    coverage: {
      status: "partial",
      reasons: [],
      limitations: ["Spelled syntax only; macro output not counted"],
    },
    assumptions: ["Not a universal quality score"],
    truncated: false,
    cancelled: false,
    deadline_reached: false,
  },
  input_digest: "metrics-input:test",
  definition_id: definitions[0].id,
  span,
  body_span: span,
  visited_nodes: 25,
  visited_tokens: 40,
  metrics: {
    source_lines: 5,
    lexical_tokens: 30,
    max_nesting: 2,
    unsafe_boundaries: 1,
  },
  locations_truncated: false,
  code_lines: Array.from({ length: 30 }, () => span),
  nesting_sites: [{ kind: "if", depth: 1, span }],
  unsafe_sites: [
    {
      kind: "unsafe_block",
      keyword_span: span,
      boundary_span: span,
      enclosing_boundary: null,
    },
  ],
  unknowns: [{ span, reason: "Generated origin unknown" }],
};
const result = (analysis = report) => ({
  api_version: "1",
  snapshot_id: snapshots[0].id,
  context_id: snapshots[0].context.id,
  analysis,
});

test("metric source jumps preserve exact selected bytes after UTF-8 and CRLF prefixes", async ({
  page,
}) => {
  await mockApi(page);
  const text =
    '// \u03bb heading\r\nfn main() { let text = "\u03bb";\r\n unsafe { operation(); }\r\n}\r\n';
  const start = new TextEncoder().encode(
    text.slice(0, text.indexOf("unsafe")),
  ).length;
  const keyword = { ...span, start, end: start + 6 };
  await page.route("**/v1/analysis/maintainability/**", (route) =>
    route.fulfill({
      json: result({
        ...report,
        unsafe_sites: [{ ...report.unsafe_sites[0], keyword_span: keyword }],
      }),
    }),
  );
  let requestedOffset: string | null = null;
  await page.route("**/v1/source/**", (route) => {
    requestedOffset = new URL(route.request().url()).searchParams.get("offset");
    return route.fulfill({
      json: {
        snapshot_id: snapshots[0].id,
        file_id: span.file_id,
        path: "src/main.rs",
        content_hash: "content:unicode-fixture",
        text,
        start_line: 1,
        total_lines: 5,
        start_byte: 0,
        truncated: false,
        encoding: "utf-8",
      },
    });
  });
  const panel = await open(page);
  await panel
    .getByRole("button", { name: "Measure source", exact: true })
    .click();
  await panel.getByLabel("Source locations").selectOption("unsafe");
  await panel.getByRole("button", { name: /unsafe_block/ }).click();
  await expect(page.locator(".cm-content")).toContainText("operation");
  expect(requestedOffset).toBe(String(start));
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const modulePath = "/node_modules/@codemirror/view/dist/index.js";
        const { EditorView } = await import(modulePath);
        const view = EditorView.findFromDOM(
          document.querySelector(".cm-editor"),
        );
        return view?.state.sliceDoc(
          view.state.selection.main.from,
          view.state.selection.main.to,
        );
      }),
    )
    .toBe("unsafe");
});
async function open(page: Page) {
  await page.route("**/v1/definitions/**", async (route) => {
    const url = new URL(route.request().url());
    const definition = definitions.find(
      (item) => item.id === decodeURIComponent(url.pathname.split("/").at(-1)!),
    );
    if (!definition) return route.fallback();
    await route.fulfill({
      json: {
        snapshot_id: url.searchParams.get("snapshot_id"),
        definition: { ...definition, body_span: definition.span },
        evidence: [evidence],
        coverage: graph.coverage,
      },
    });
  });
  await page.goto(
    "/?snapshot=snapshot%3Atwo&context=context%3Ahost&definition=definition%3Amain&view=analysis#token=test-token",
  );
  return page.getByRole("region", { name: "Source maintainability" });
}

for (const width of [1440, 390])
  test(`source metrics have bounded source navigation at ${width}`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await mockApi(page);
    let calls = 0;
    await page.route("**/v1/analysis/maintainability/**", async (route) => {
      calls++;
      const url = new URL(route.request().url());
      expect(url.searchParams.get("snapshot_id")).toBe(snapshots[0].id);
      expect(url.searchParams.get("context_id")).toBe(snapshots[0].context.id);
      await route.fulfill({ json: result() });
    });
    const panel = await open(page);
    await expect(panel).toBeVisible();
    expect(calls).toBe(0);
    await panel
      .getByRole("button", { name: "Measure source", exact: true })
      .press("Enter");
    await expect(panel.locator("dd")).toHaveText(["5", "30", "2", "1"]);
    await panel.getByLabel("Source locations").selectOption("lines");
    await expect(panel.locator(".maintainability-sites li")).toHaveCount(25);
    await panel.getByRole("button", { name: "Next metric locations" }).click();
    await expect(panel.locator(".maintainability-sites li")).toHaveCount(5);
    await panel.getByLabel("Source locations").selectOption("unsafe");
    await expect(
      panel.getByRole("button", { name: /unsafe_block/ }),
    ).toBeVisible();
    expect(
      await panel.evaluate(
        (element) => element.scrollWidth <= element.clientWidth,
      ),
    ).toBe(true);
    await panel.screenshot({
      path: testInfo.outputPath("maintainability.png"),
    });
    await panel.getByRole("button", { name: /unsafe_block/ }).press("Enter");
    await expect(page).toHaveURL(/view=explore/);
    await expect(page.locator(".cm-editor")).toBeVisible();
  });

test("incomplete metrics withhold counts, failed requests retry, wrong scopes reject", async ({
  page,
}) => {
  await mockApi(page);
  let attempt = 0;
  await page.route("**/v1/analysis/maintainability/**", async (route) => {
    if (++attempt === 1)
      return route.fulfill({
        status: 503,
        json: {
          code: "unavailable",
          message: "Metric fixture unavailable",
          correlation_id: "fixture",
        },
      });
    if (attempt === 2)
      return route.fulfill({
        json: result({
          ...report,
          metrics: null,
          envelope: {
            ...report.envelope,
            truncated: true,
            deadline_reached: true,
          },
        }),
      });
    return route.fulfill({
      json: { ...result(), context_id: "context:wrong" },
    });
  });
  const panel = await open(page);
  const measure = panel.getByRole("button", {
    name: "Measure source",
    exact: true,
  });
  await measure.click();
  await expect(panel.getByText("Metric fixture unavailable")).toBeVisible();
  await measure.click();
  await expect(
    panel.getByText("Counts withheld: source traversal is incomplete."),
  ).toBeVisible();
  await expect(panel.locator("dd")).toHaveCount(0);
  await expect(
    panel.getByText("Deadline reached", { exact: true }),
  ).toBeVisible();
  await measure.click();
  await expect(
    panel.getByText("Metric response does not match this selection."),
  ).toBeVisible();
  await expect(panel.locator("dd")).toHaveCount(0);
});

for (const changed of ["definition", "snapshot", "context"] as const)
  test(`${changed} navigation aborts in-flight metrics and never displays a prior selection`, async ({
    page,
  }) => {
    await mockApi(page);
    let release!: () => void;
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    let started!: () => void;
    const requested = new Promise<void>((resolve) => {
      started = resolve;
    });
    await page.route("**/v1/analysis/maintainability/**", async (route) => {
      started();
      await held;
      await route.fulfill({ json: result() }).catch(() => {});
    });
    const panel = await open(page);
    await panel
      .getByRole("button", { name: "Measure source", exact: true })
      .click();
    await requested;
    const failed = page.waitForEvent("requestfailed", (request) =>
      request.url().includes("/analysis/maintainability/"),
    );
    await page.evaluate((changed) => {
      const url = new URL(location.href);
      url.searchParams.set(
        changed,
        changed === "definition"
          ? "definition:handle"
          : changed === "snapshot"
            ? "snapshot:one"
            : "context:wrong",
      );
      history.pushState(null, "", url);
      dispatchEvent(new PopStateEvent("popstate"));
    }, changed);
    if (changed === "context") await expect(panel).toHaveCount(0);
    else
      await expect(
        panel.getByRole("button", { name: "Measure source", exact: true }),
      ).toBeEnabled();
    await failed;
    release();
    await expect(panel.locator("dd")).toHaveCount(0);
    await expect(panel.getByText("Measuring selected source")).toHaveCount(0);
  });
