import { expect, test, type Page } from "@playwright/test";
import { definitions, graph, mockApi, snapshots } from "./fixtures";
import type {
  AnalysisEnvelope,
  GraphAnalysis,
  JobRecord,
  ObservationSummary,
  ObservationWindow,
  TraceComparison,
} from "../src/api/types";

const envelope: AnalysisEnvelope = {
  algorithm_version: "selected-call-graph-v1",
  coverage: graph.coverage,
  truncated: false,
  cancelled: false,
  deadline_reached: false,
  assumptions: ["Selected graph only, not the whole repository."],
};
const analysis: GraphAnalysis = {
  envelope,
  selected_nodes: 2,
  selected_call_sites: 2,
  components: [
    { id: "component:1", members: [definitions[0].id], recursive: true },
  ],
  component_links: [],
  metrics: definitions.map((definition) => ({
    definition_id: definition.id,
    distinct_callers: 1,
    distinct_callees: 1,
    incoming_call_sites: 1,
    outgoing_call_sites: 2,
    unknown_call_sites: 1,
    source_lines: 4,
    source_branches: 1,
    source_returns: 0,
    source_awaits: 0,
    source_unsafe_blocks: 0,
    declared_public: false,
  })),
  distributions: [
    { metric: "distinct_callers", minimum: 0, median: 1, p95: 1, maximum: 1 },
  ],
  unknown_frontier: [],
};
async function open(page: Page, view: string) {
  await page.goto(`/?view=${view}#token=test-token`);
  await expect(page).toHaveURL(/snapshot=snapshot%3Atwo/);
}

for (const viewport of [
  { width: 1440, height: 900 },
  { width: 390, height: 844 },
]) {
  test(`selected analysis and keyboard path workflow at ${viewport.width}`, async ({
    page,
  }) => {
    await page.setViewportSize(viewport);
    await mockApi(page);
    let captured: unknown;
    await page.route("**/v1/graph/analysis", async (route) => {
      const body = route.request().postDataJSON();
      expect(body.snapshot_id).toBe("snapshot:two");
      expect(body.context_id).toBe("context:host");
      await route.fulfill({
        json: {
          api_version: "1",
          snapshot_id: body.snapshot_id,
          context_id: body.context_id,
          analysis,
        },
      });
    });
    await page.route("**/v1/graph/path", async (route) => {
      captured = route.request().postDataJSON();
      await route.fulfill({
        json: {
          api_version: "1",
          snapshot_id: "snapshot:two",
          context_id: "context:host",
          analysis: {
            envelope,
            outcome: "found",
            path: definitions.map((definition) => definition.id),
            relations: ["relation:call"],
            reachable: definitions.map((definition) => definition.id),
            unknown_frontier: [],
          },
        },
      });
    });
    await open(page, "analysis");
    await expect(
      page.getByRole("heading", { name: "Selected Graph Analysis" }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Recursive Components" }),
    ).toBeVisible();
    const target = page.getByLabel("Path target");
    await target.selectOption(definitions[1].id);
    await target.press("Tab");
    await page
      .getByRole("button", { name: "Find path", exact: true })
      .press("Enter");
    await expect(
      page.getByRole("heading", { name: "Static Path Found" }),
    ).toBeVisible();
    expect(captured).toMatchObject({
      target: definitions[1].id,
      graph: {
        snapshot_id: "snapshot:two",
        context_id: "context:host",
        direction: "outgoing",
        max_nodes: 200,
        max_edges: 500,
      },
    });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBeTruthy();
    await page.screenshot({
      path: test.info().outputPath("analysis.png"),
      fullPage: true,
    });
    await page.locator(".path-steps button").last().click();
    await expect(page).toHaveURL(/definition=definition%3Ahandle/);
    await expect(page).toHaveURL(/view=explore/);
  });
}

test("partial negative is explicitly unknown and stale paths clear on target change", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/v1/graph/analysis", (route) =>
    route.fulfill({
      json: {
        api_version: "1",
        snapshot_id: "snapshot:two",
        context_id: "context:host",
        analysis,
      },
    }),
  );
  await page.route("**/v1/graph/path", (route) =>
    route.fulfill({
      json: {
        api_version: "1",
        snapshot_id: "snapshot:two",
        context_id: "context:host",
        analysis: {
          envelope: { ...envelope, truncated: true },
          outcome: "unknown",
          path: [],
          relations: [],
          reachable: [definitions[0].id],
          unknown_frontier: [],
        },
      },
    }),
  );
  await open(page, "analysis");
  await page
    .getByRole("button", { name: "Find reachable definitions" })
    .click();
  await expect(
    page.getByRole("heading", { name: "Reachability Unknown" }),
  ).toBeVisible();
  await page.getByLabel("Path target").selectOption(definitions[1].id);
  await expect(
    page.getByRole("heading", { name: "Reachability Unknown" }),
  ).toHaveCount(0);
});

test("jobs queue pins current context, exposes stages and cancels explicitly", async ({
  page,
}) => {
  await mockApi(page);
  const records: JobRecord[] = [];
  let submitted = 0;
  await page.route("**/v1/jobs", async (route) => {
    if (route.request().method() === "POST") {
      submitted++;
      const request = route.request().postDataJSON();
      expect(request).toMatchObject({
        profile: "host-default",
        level: "syntax",
        priority: "foreground",
        context: { id: "context:host", trust: "read_only" },
      });
      const record: JobRecord = {
        id: "job:one",
        request,
        status: "running",
        created_ms: "9007199254740993",
        snapshot_id: null,
        message: null,
        events: [
          {
            sequence: 1,
            timestamp_ms: "9007199254740993",
            stage: "capture",
            status: "running",
          },
        ],
      };
      records.push(record);
      await route.fulfill({ json: record });
    } else await route.fulfill({ json: records });
  });
  await page.route("**/v1/jobs/job%3Aone/cancel", async (route) => {
    records[0].status = "cancelled";
    await route.fulfill({ json: records[0] });
  });
  await open(page, "health");
  await expect(page.getByText("No analysis jobs recorded.")).toBeVisible();
  expect(submitted).toBe(0);
  await page.getByLabel("Job analysis level").selectOption("syntax");
  await page.getByLabel("Job priority").selectOption("foreground");
  await page.getByRole("button", { name: "Queue analysis" }).click();
  await expect(page.locator(".job-row .badge")).toHaveText("running");
  await page.locator(".job-row summary").click();
  await expect(page.locator(".job-events")).toContainText(
    "9007199254740993 ms",
  );
  await page.getByRole("button", { name: "Cancel job job:one" }).click();
  await expect(page.locator(".job-row .badge")).toHaveText("cancelled");
  expect(submitted).toBe(1);
});

test("disabled jobs are a quiet unavailable state, not a false empty queue", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/v1/jobs", (route) =>
    route.fulfill({
      status: 501,
      json: {
        code: "unsupported_capability",
        message: "Jobs disabled",
        correlation_id: "test",
      },
    }),
  );
  await open(page, "health");
  await expect(
    page.getByText("Analysis jobs are unavailable on this server."),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Queue analysis" }),
  ).toBeDisabled();
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("trace comparison preserves distinct artifacts, wide positions and clock domains", async ({
  page,
}) => {
  await mockApi(page);
  const summary = (snapshotId: string): ObservationSummary => ({
    id: `observation:${snapshotId}`,
    snapshot_id: snapshotId,
    artifact: {
      sha256: snapshotId.endsWith("one") ? "a".repeat(64) : "b".repeat(64),
      source_id: "source",
      context_id: "context:host",
      producer: "synthetic comparison fixture",
    },
    test_count: 0,
    event_count: 2,
    clock_domains: ["device-clock"],
    limitations: ["Synthetic fixture, not a real execution"],
  });
  await page.route("**/v1/observations?*", async (route) => {
    const id = new URL(route.request().url()).searchParams.get("snapshot_id")!;
    await route.fulfill({ json: [summary(id)] });
  });
  await page.route("**/v1/observations/observation*", async (route) => {
    const id = new URL(route.request().url()).searchParams.get("snapshot_id")!;
    const body: ObservationWindow = {
      summary: summary(id),
      tests: [],
      streams: [
        {
          id: "cpu",
          process_or_device: "device",
          thread_or_hart: "0",
          clock_domain: "device-clock",
          timestamp_unit: "cycles",
          events: [],
        },
      ],
      offset: 0,
      next_offset: null,
      truncated: false,
    };
    await route.fulfill({ json: body });
  });
  await page.route("**/v1/traces/compare", async (route) => {
    expect(route.request().postDataJSON()).toMatchObject({
      before_snapshot_id: "snapshot:one",
      after_snapshot_id: "snapshot:two",
      before_stream_id: "cpu",
      after_stream_id: "cpu",
      max_events: 200,
      max_anchors: 200,
    });
    const result: TraceComparison = {
      envelope,
      before_artifact: summary("snapshot:one").artifact,
      after_artifact: summary("snapshot:two").artifact,
      anchor: "packet:17",
      compared_events: 2,
      matching_prefix_events: 1,
      ambiguous_anchors: [],
      first_observed_divergence: {
        reason: "event_kind",
        before: {
          stream_id: "cpu",
          sequence: "9007199254740993",
          timestamp: "18014398509481987",
          clock_domain: "cpu-clock",
          timestamp_unit: "ns",
        },
        after: {
          stream_id: "cpu",
          sequence: "9007199254740997",
          timestamp: "18014398509481988",
          clock_domain: "device-clock",
          timestamp_unit: "cycles",
        },
        before_kind: "queue_switch",
        after_kind: "external_effect",
      },
      before_loss_count: "0",
      after_loss_count: "18446744073709551615",
      before_interval: "7",
      after_interval: "70",
      before_clock_domain: "cpu-clock",
      after_clock_domain: "device-clock",
      before_timestamp_unit: "ns",
      after_timestamp_unit: "cycles",
    };
    await route.fulfill({
      json: {
        before: "snapshot:one",
        after: "snapshot:two",
        before_context: "context:host",
        after_context: "context:host",
        comparison: result,
      },
    });
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await open(page, "evidence");
  await page.getByRole("button", { name: "Compare traces" }).click();
  await expect(
    page.getByRole("heading", { name: "First Observed Divergence" }),
  ).toBeVisible();
  await expect(page.locator(".trace-comparison-result")).toContainText(
    "9007199254740993",
  );
  await expect(page.locator(".trace-comparison-result")).toContainText(
    "18446744073709551615",
  );
  await expect(page.locator(".trace-comparison-result")).toContainText(
    "cpu-clock / ns",
  );
  await expect(page.locator(".trace-comparison-result")).toContainText(
    "device-clock / cycles",
  );
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBeTruthy();
  await page.getByLabel("Trace comparison window size").fill("100");
  await expect(
    page.getByRole("heading", { name: "First Observed Divergence" }),
  ).toHaveCount(0);
});
