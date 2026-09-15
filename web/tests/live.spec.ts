import { test, expect } from "@playwright/test";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import { createServer } from "node:net";
import { randomBytes, createHash } from "node:crypto";
import type {
  Snapshot,
  Definition,
  GraphResponse,
  SourceWindow,
  DiffResponse,
  ObservationBundle,
  ObservationSummary,
  ObservationWindow,
  JobRecord,
} from "../src/api/types";

test("real capture, HTTP, source graph, observations, edit and restart", async ({
  page,
}, testInfo) => {
  test.setTimeout(90_000);
  const root = resolve(import.meta.dirname, "../..");
  const binary = join(root, "target/debug/atlas");
  const temp = await mkdtemp(join(tmpdir(), "atlas-live-"));
  const workspace = join(temp, "project");
  const store = join(temp, "store");
  const token = randomBytes(32).toString("hex");
  const tokenFile = join(temp, "session");
  const socket = createServer();
  await new Promise<void>((done) => socket.listen(0, "127.0.0.1", done));
  const port = (socket.address() as { port: number }).port;
  await new Promise<void>((done) => socket.close(() => done()));
  const base = `http://127.0.0.1:${port}`;
  let server: ChildProcess | undefined;
  let logs = "";
  const run = (args: string[]) =>
    JSON.parse(
      execFileSync(binary, ["--store", store, ...args], {
        encoding: "utf8",
        timeout: 30_000,
        stdio: ["ignore", "pipe", "pipe"],
      }),
    );
  const api = async <T>(path: string, body?: unknown): Promise<T> => {
    const response = await fetch(base + "/v1" + path, {
      headers: {
        Authorization: `Bearer ${token}`,
        ...(body ? { "Content-Type": "application/json" } : {}),
      },
      ...(body ? { method: "POST", body: JSON.stringify(body) } : {}),
    });
    expect(response.ok, `${response.status}: ${path}`).toBeTruthy();
    return response.json() as Promise<T>;
  };
  const start = async () => {
    server = spawn(
      binary,
      [
        "--store",
        store,
        "serve",
        "--enable-jobs",
        "--listen",
        `127.0.0.1:${port}`,
        "--web-dir",
        join(root, "web/dist"),
        "--token-file",
        tokenFile,
      ],
      { stdio: ["ignore", "pipe", "pipe"] },
    );
    server.stdout?.on("data", (data) => {
      logs += data.toString();
    });
    server.stderr?.on("data", (data) => {
      logs += data.toString();
    });
    await expect
      .poll(
        async () => {
          if (server?.exitCode !== null)
            throw new Error(logs.replaceAll(token, "[redacted]"));
          try {
            return (
              await fetch(base + "/v1/capabilities", {
                headers: { Authorization: `Bearer ${token}` },
              })
            ).status;
          } catch {
            return 0;
          }
        },
        { timeout: 10_000 },
      )
      .toBe(200);
  };
  const stop = async () => {
    if (server && server.exitCode === null && server.signalCode === null) {
      const child = server;
      const stopped = new Promise<void>((done) =>
        child.once("exit", () => done()),
      );
      child.kill("SIGINT");
      const timer = setTimeout(() => child.kill("SIGKILL"), 3000);
      await stopped;
      clearTimeout(timer);
    }
    server = undefined;
  };
  try {
    await mkdir(join(workspace, "src"), { recursive: true });
    await writeFile(tokenFile, token, { mode: 0o600 });
    await writeFile(
      join(workspace, "Cargo.toml"),
      "[package]\nname='live_pilot'\nversion='0.1.0'\nedition='2021'\n",
    );
    const original =
      "pub mod engine;\r\npub fn entry(value: u32) -> u32 { engine::step(value) }\r\npub fn callback(value: u32, operation: fn(u32) -> u32) -> u32 { operation(value) }\r\n";
    await writeFile(join(workspace, "src/lib.rs"), original);
    await writeFile(
      join(workspace, "src/engine.rs"),
      "pub fn step(value: u32) -> u32 { leaf(value) }\nfn leaf(value: u32) -> u32 { value + 1 }\n",
    );
    run(["init", "--workspace", workspace]);
    const before = run([
      "index",
      "--target",
      "x86_64-unknown-linux-gnu",
      "--no-default-features",
    ]) as Snapshot;
    let compilerImport: { id: string; mapped_count: number } | undefined;
    if (process.env.ATLAS_RUSTC) {
      const compilerBundle = join(temp, "compiler.json");
      execFileSync(
        process.env.ATLAS_RUSTC,
        [
          "--trusted-local",
          "--root",
          workspace,
          "--source",
          "src/lib.rs",
          "--crate-name",
          "live_pilot",
          "--output",
          compilerBundle,
        ],
        { timeout: 30_000, stdio: ["ignore", "pipe", "pipe"] },
      );
      compilerImport = run([
        "import-compiler",
        "--snapshot",
        before.id,
        "--bundle",
        compilerBundle,
      ]);
      expect(compilerImport!.mapped_count).toBeGreaterThanOrEqual(4);
    }
    await start();
    const prepared = await api<{ ready: boolean; snapshot_id: string }>(
      `/snapshots/${encodeURIComponent(before.id)}/prepare?context_id=${encodeURIComponent(before.context.id)}`,
      {},
    );
    expect(prepared.ready).toBe(true);
    expect(prepared.snapshot_id).toBe(before.id);
    const pin = (snapshot: Snapshot) =>
      new URLSearchParams({
        snapshot_id: snapshot.id,
        context_id: snapshot.context.id,
      });
    const definitions = await api<{ items: Definition[] }>(
      "/search?" + pin(before),
    );
    const entry = definitions.items.find(
      (definition) => definition.name === "entry",
    )!;
    const callback = definitions.items.find(
      (definition) => definition.name === "callback",
    )!;
    expect(entry).toBeDefined();
    const graph = await api<GraphResponse>("/graph/neighborhood", {
      snapshot_id: before.id,
      context_id: before.context.id,
      definition_id: entry.id,
      depth: 2,
      direction: "outgoing",
      max_nodes: 200,
      max_edges: 500,
    });
    expect(graph.nodes.map((node) => node.name)).toEqual(
      expect.arrayContaining(["entry", "step", "leaf"]),
    );
    const indirect = await api<GraphResponse>("/graph/neighborhood", {
      snapshot_id: before.id,
      context_id: before.context.id,
      definition_id: callback.id,
      depth: 2,
      direction: "outgoing",
      max_nodes: 200,
      max_edges: 500,
    });
    expect(
      indirect.edges.some((edge) => edge.target.kind === "unknown"),
    ).toBeTruthy();
    const source = await api<SourceWindow>(
      `/source/${encodeURIComponent(entry.file_id)}?${pin(before)}`,
    );
    expect(source.text).toBe(original);
    expect(source.text).toContain("\r\n");
    expect((await fetch(base + "/v1/snapshots")).status).toBe(401);
    expect(
      (
        await fetch(base + "/v1/snapshots", {
          headers: {
            Authorization: `Bearer ${token}`,
            Origin: "https://example.invalid",
          },
        })
      ).status,
    ).toBe(403);
    const mismatch = new URLSearchParams({
      snapshot_id: before.id,
      context_id: "context:wrong",
    });
    expect(
      (
        await fetch(
          base + `/v1/definitions/${encodeURIComponent(entry.id)}?${mismatch}`,
          { headers: { Authorization: `Bearer ${token}` } },
        )
      ).status,
    ).toBe(409);

    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto(
      `${base}/?snapshot=${encodeURIComponent(before.id)}&context=${encodeURIComponent(before.context.id)}&definition=${encodeURIComponent(entry.id)}&depth=2&direction=outgoing#token=${token}`,
    );
    await expect(page.getByTestId("graph-canvas")).toHaveAttribute(
      "data-layout-state",
      "ready",
    );
    await expect(page.locator(".cm-content")).toContainText("pub fn entry");
    await page
      .getByRole("button", { name: "Pin selection", exact: true })
      .click();
    await expect(page.locator(".bookmark-entry .retained")).toHaveText(
      "Snapshot retained",
    );
    expect(
      run(["pins"]).some(
        (pin: { name: string; snapshot_id: string }) =>
          pin.name.startsWith("bookmark-pin:") && pin.snapshot_id === before.id,
      ),
    ).toBe(true);
    await page
      .getByRole("button", { name: "Unpin selection", exact: true })
      .click();
    await expect(page.locator(".bookmark-entry")).toHaveCount(0);
    expect(
      run(["pins"]).some((pin: { name: string }) =>
        pin.name.startsWith("bookmark-pin:"),
      ),
    ).toBe(false);
    expect(page.url()).not.toContain(token);
    const pixels = await page
      .getByTestId("graph-canvas")
      .evaluate((element) => {
        let count = 0;
        for (const canvas of element.querySelectorAll("canvas")) {
          const context = canvas.getContext("2d");
          if (!context || !canvas.width || !canvas.height) continue;
          const data = context.getImageData(
            0,
            0,
            canvas.width,
            canvas.height,
          ).data;
          for (let i = 0; i < data.length; i += 4)
            if (
              data[i + 3] > 30 &&
              Math.min(data[i], data[i + 1], data[i + 2]) < 210
            )
              count++;
        }
        return count;
      });
    expect(pixels).toBeGreaterThan(100);
    const midpoint = await page
      .getByTestId("graph-canvas")
      .evaluate((element) => {
        const cy = (
          element as unknown as { _cyreg: { cy: import("cytoscape").Core } }
        )._cyreg.cy;
        const edge = cy.edges()[0];
        return { ...edge.renderedMidpoint(), id: edge.id() };
      });
    const bounds = (await page.getByTestId("graph-canvas").boundingBox())!;
    await page.mouse.click(bounds.x + midpoint.x, bounds.y + midpoint.y);
    await expect
      .poll(() => new URL(page.url()).searchParams.get("edge"))
      .toBe(midpoint.id);
    await page.screenshot({
      path: testInfo.outputPath("live-desktop.png"),
      fullPage: true,
    });
    await page.reload();
    await expect(page.getByTestId("graph-canvas")).toHaveAttribute(
      "data-layout-state",
      "ready",
    );
    expect(new URL(page.url()).searchParams.get("edge")).toBe(midpoint.id);
    expect(errors).toEqual([]);

    if (compilerImport) {
      await page.getByRole("button", { name: "Flow", exact: true }).click();
      await page.getByLabel("Flow phase").selectOption(compilerImport.id);
      await expect(
        page.getByRole("heading", { name: "Compiler Basic Blocks" }),
      ).toBeVisible();
      await expect(page.locator(".compiler-block-table")).toContainText(
        "return",
      );
      await page.getByRole("button", { name: "Analyze locals" }).click();
      await expect(
        page.getByText("Fixed point reached", { exact: true }),
      ).toBeVisible();
      await expect(page.locator(".dataflow-table")).toContainText(
        "Unknown memory effects",
      );
      await page.screenshot({
        path: testInfo.outputPath("live-compiler-flow.png"),
        fullPage: true,
      });
      await page.getByRole("button", { name: "Explore", exact: true }).click();
    }

    const artifact = join(temp, "artifact");
    await writeFile(artifact, "controlled observation fixture; never executed");
    const bundle: ObservationBundle = {
      schema_version: 1,
      snapshot_id: before.id,
      artifact: {
        sha256: createHash("sha256")
          .update("controlled observation fixture; never executed")
          .digest("hex"),
        source_id: before.source_id,
        context_id: before.context.id,
        producer: "Controlled observation fixture",
      },
      tests: [
        {
          name: "entry fixture",
          outcome: "timeout",
          elapsed_ns: "9007199254740993",
          timeout_ns: "1000000000",
          reason: "Fixture timeout",
          definition_ids: [entry.id],
        },
      ],
      streams: ["cpu", "device"].map((id, index) => ({
        id,
        process_or_device: id,
        thread_or_hart: "0",
        clock_domain: id + "-clock",
        timestamp_unit: index ? "cycles" : "ns",
        events: [
          {
            sequence: "1",
            timestamp: index ? "2" : "18446744073709551615",
            kind: "enter",
            definition_id: entry.id,
            correlation_id: null,
            loss_count: "3",
          },
        ],
      })),
      limitations: [
        "Synthetic observations test import and display, not actual run behavior.",
      ],
    };
    const bundlePath = join(temp, "bundle.json");
    await writeFile(bundlePath, JSON.stringify(bundle));
    const observation = run([
      "import-evidence",
      "--snapshot",
      before.id,
      "--artifact",
      artifact,
      "--bundle",
      bundlePath,
    ]) as ObservationSummary;
    const observationPage = await api<ObservationWindow>(
      `/observations/${encodeURIComponent(observation.id)}?${pin(before)}&limit=200`,
    );
    expect(observationPage.tests[0].elapsed_ns).toBe("9007199254740993");
    expect(observationPage.streams[0].events[0].timestamp).toBe(
      "18446744073709551615",
    );
    expect(observationPage.summary.clock_domains).toEqual([
      "cpu-clock",
      "device-clock",
    ]);
    await page.getByRole("button", { name: "Evidence", exact: true }).click();
    await page
      .getByRole("button", { name: /Controlled observation fixture/ })
      .click();
    await expect(
      page.getByText("9007199254740993", { exact: true }),
    ).toBeVisible();

    await writeFile(
      join(workspace, "src/engine.rs"),
      "pub fn step(value: u32) -> u32 { leaf(value) + 7 }\nfn leaf(value: u32) -> u32 { value + 1 }\n",
    );
    const queued = await api<JobRecord>("/jobs", {
      profile: "default",
      level: "semantic",
      priority: "foreground",
    });
    const events = await fetch(`${base}/v1/jobs/${queued.id}/events`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    expect(events.headers.get("content-type")).toContain("text/event-stream");
    const progress = await events.text();
    expect(progress).toContain('"status":"succeeded"');
    const completed = await api<JobRecord>(`/jobs/${queued.id}`);
    expect(completed.status).toBe("succeeded");
    expect(completed.snapshot_id).toBeTruthy();
    const after = await api<Snapshot>(`/snapshots/${completed.snapshot_id}`);
    const diff = await api<DiffResponse>("/diff", {
      before: before.id,
      after: after.id,
    });
    expect(
      diff.changes.some(
        (change) =>
          change.after?.name === "step" &&
          change.changed_fields.includes("body"),
      ),
    ).toBeTruthy();
    await stop();
    await start();
    expect((await api<JobRecord>(`/jobs/${queued.id}`)).status).toBe(
      "succeeded",
    );
    const restartedSource = await api<SourceWindow>(
      `/source/${encodeURIComponent(entry.file_id)}?${pin(before)}`,
    );
    expect(restartedSource.text).toBe(original);
    expect((await api<Snapshot[]>("/snapshots")).length).toBe(2);
    await page.goto(
      `${base}/?snapshot=${encodeURIComponent(before.id)}&context=${encodeURIComponent(before.context.id)}&definition=${encodeURIComponent(callback.id)}&depth=2&direction=outgoing`,
    );
    await expect(page.getByTestId("graph-canvas")).toHaveAttribute(
      "data-layout-state",
      "ready",
    );
    await page.evaluate(() =>
      localStorage.setItem(
        "ferrum-atlas.bookmarks",
        JSON.stringify([
          {
            id: "missing-history",
            label: "Absent snapshot",
            note: "",
            location: {
              snapshot: "analysis:missing",
              context: "context:missing",
              definition: "definition:missing",
              edge: "",
              view: "explore",
              depth: 2,
              direction: "both",
            },
          },
        ]),
      ),
    );
    await page.reload();
    await expect(page.locator(".bookmark-entry")).toContainText(
      "Snapshot not confirmed retained",
    );
    await page
      .getByRole("button", { name: "Remove trail pin Absent snapshot" })
      .click();
    await expect(page.locator(".bookmark-entry")).toHaveCount(0);
    await page.setViewportSize({ width: 390, height: 844 });
    await page.screenshot({
      path: testInfo.outputPath("live-mobile.png"),
      fullPage: true,
    });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBeTruthy();
    expect(errors).toEqual([]);
  } finally {
    await stop();
    await rm(temp, { recursive: true, force: true });
  }
});
