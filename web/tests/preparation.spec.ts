import { expect, test, type Page } from "@playwright/test";
import { mockApi } from "./fixtures";

async function harness(page: Page) {
  await page.route("**/client-harness", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: "<!doctype html><title>Client contract</title>",
    }),
  );
  await page.goto("/client-harness");
  await page.evaluate(() =>
    sessionStorage.setItem("ferrum-atlas.session", "test-token"),
  );
}

test("concurrent fact queries share one scope preparation and metadata/jobs/pins never prepare", async ({
  page,
}) => {
  await harness(page);
  const events: string[] = [];
  await page.route("**/v1/**", async (route) => {
    const url = new URL(route.request().url());
    expect(route.request().headers().authorization).toBe("Bearer test-token");
    if (url.pathname.endsWith("/prepare")) {
      events.push("prepare");
      expect(route.request().method()).toBe("POST");
      expect(route.request().postData()).toBeNull();
      expect(url.searchParams.get("context_id")).toBe("c");
      await route.fulfill({
        json: { snapshot_id: "s", context_id: "c", ready: true },
      });
      return;
    }
    events.push(url.pathname);
    await route.fulfill({ json: {} });
  });
  await page.evaluate(async () => {
    const module = "/src/api/client.ts";
    const { request } = await import(module);
    await Promise.all([
      request(
        "/source/f?snapshot_id=s&context_id=c",
        new AbortController().signal,
      ),
      request(
        "/definitions/d?snapshot_id=s&context_id=c",
        new AbortController().signal,
      ),
      request("/graph/path", new AbortController().signal, {
        graph: { snapshot_id: "s", context_id: "c" },
        target: "d",
      }),
    ]);
    for (const path of [
      "/search",
      "/flow/d",
      "/bodies/d",
      "/evidence/e",
      "/compiler",
      "/observations",
      "/queries/impact",
      "/analysis",
    ])
      await request(
        `${path}?snapshot_id=s&context_id=c`,
        new AbortController().signal,
      );
    for (const path of ["/capabilities", "/snapshots", "/snapshots/s", "/jobs"])
      await request(path, new AbortController().signal);
    await request("/snapshots/other/pin", new AbortController().signal, {
      context_id: "other",
      name: "test",
    });
    await request("/snapshots/other/unpin", new AbortController().signal, {
      context_id: "other",
      name: "test",
    });
    await request(
      "/analysis/state-machine/d/reviews",
      new AbortController().signal,
      {
        selection: {
          snapshot_id: "s",
          context_id: "c",
          enum_path: "State",
          state_place: "state",
        },
        review: { input_digest: "inference:test" },
      },
    );
  });
  expect(events[0]).toBe("prepare");
  expect(events.filter((item) => item === "prepare")).toHaveLength(1);
  expect(events).toContain("/v1/source/f");
  expect(events).toContain("/v1/graph/path");
});

test("successful deadline-partial responses invalidate readiness without retrying or concealing the result", async ({
  page,
}) => {
  await harness(page);
  let prepares = 0;
  const responses = [
    { work: { deadline_reached: true }, items: [] },
    { analysis: { envelope: { deadline_reached: true } } },
    { comparison: { envelope: { deadline_reached: true } } },
    { work: { deadline_reached: false }, items: ["ready"] },
    { work: { deadline_reached: false }, items: ["ready"] },
  ];
  let facts = 0;
  await page.route("**/v1/**", async (route) => {
    if (new URL(route.request().url()).pathname.endsWith("/prepare")) {
      prepares++;
      return route.fulfill({
        json: { snapshot_id: "s", context_id: "c", ready: true },
      });
    }
    await route.fulfill({ json: responses[facts++] });
  });
  for (let i = 0; i < responses.length; i++) {
    const result = await page.evaluate(async () => {
      const module = "/src/api/client.ts";
      const { request } = await import(module);
      return request(
        "/search?snapshot_id=s&context_id=c",
        new AbortController().signal,
      );
    });
    expect(result).toEqual(responses[i]);
    expect(facts).toBe(i + 1);
    expect(prepares).toBe(Math.min(i + 1, 4));
  }
});

test("diff resolves both context pins and trace comparisons prepare both independent scopes", async ({
  page,
}) => {
  await harness(page);
  const prepared: string[] = [];
  const facts: string[] = [];
  await page.route("**/v1/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname.endsWith("/prepare")) {
      const snapshot_id = url.pathname.split("/")[3];
      const context_id = url.searchParams.get("context_id");
      prepared.push(`${snapshot_id}:${context_id}`);
      return route.fulfill({ json: { snapshot_id, context_id, ready: true } });
    }
    if (url.pathname.startsWith("/v1/snapshots/")) {
      const id = url.pathname.split("/")[3];
      return route.fulfill({ json: { id, context: { id: `context-${id}` } } });
    }
    facts.push(url.pathname);
    expect(prepared.length).toBe(facts.length === 1 ? 2 : 4);
    await route.fulfill({ json: {} });
  });
  await page.evaluate(async () => {
    const module = "/src/api/client.ts";
    const { request } = await import(module);
    await request("/diff", new AbortController().signal, {
      before: "a",
      after: "b",
    });
    await request("/traces/compare", new AbortController().signal, {
      before_snapshot_id: "a",
      before_context_id: "trace-a",
      after_snapshot_id: "b",
      after_context_id: "trace-b",
    });
  });
  expect(prepared.sort()).toEqual([
    "a:context-a",
    "a:trace-a",
    "b:context-b",
    "b:trace-b",
  ]);
});

test("failed or mismatched preparation prevents facts, preserves HTTP errors, and allows a fresh retry", async ({
  page,
}) => {
  await harness(page);
  let mode = 503;
  let facts = 0,
    prepares = 0;
  await page.route("**/v1/**", async (route) => {
    if (new URL(route.request().url()).pathname.endsWith("/prepare")) {
      prepares++;
      if (mode > 0)
        return route.fulfill({
          status: mode,
          json: {
            code: "preparation_failure",
            message: "Snapshot verification failed",
            correlation_id: "verify-test",
          },
        });
      return route.fulfill({
        json: {
          snapshot_id: mode === -1 ? "wrong" : "s",
          context_id: "c",
          ready: true,
        },
      });
    }
    facts++;
    await route.fulfill({ json: {} });
  });
  for (const status of [403, 409, 429, 503, 408, -1]) {
    mode = status;
    const failure = await page.evaluate(async () => {
      const module = "/src/api/client.ts";
      const { request } = await import(module);
      try {
        await request(
          "/search?snapshot_id=s&context_id=c",
          new AbortController().signal,
        );
        return null;
      } catch (error) {
        const value = error as {
          code: string;
          status: number;
          message: string;
        };
        return {
          code: value.code,
          status: value.status,
          message: value.message,
        };
      }
    });
    expect(failure?.code).toBe(
      status === -1 ? "invalid_response" : "preparation_failure",
    );
    expect(failure?.status).toBe(status === -1 ? 0 : status);
    expect(failure?.message).not.toContain("test-token");
    expect(facts).toBe(0);
  }
  mode = 0;
  await page.evaluate(async () => {
    const module = "/src/api/client.ts";
    const { request } = await import(module);
    await request(
      "/search?snapshot_id=s&context_id=c",
      new AbortController().signal,
    );
  });
  expect(prepares).toBe(7);
  expect(facts).toBe(1);
});

test("one cancelled consumer preserves shared preparation, last cancellation aborts it, and retry starts fresh", async ({
  page,
}) => {
  await harness(page);
  const result = await page.evaluate(async () => {
    const module = "/src/api/client.ts";
    const { request } = await import(module);
    let preparations = 0,
      aborted = 0,
      facts = 0;
    const releases: (() => void)[] = [];
    window.fetch = (input, init) => {
      const url = new URL(String(input), location.origin);
      if (!url.pathname.endsWith("/prepare")) {
        facts++;
        return Promise.resolve(Response.json({}));
      }
      preparations++;
      return new Promise<Response>((resolve, reject) => {
        releases.push(() =>
          resolve(
            Response.json({
              snapshot_id: url.pathname.split("/")[3],
              context_id: url.searchParams.get("context_id"),
              ready: true,
            }),
          ),
        );
        init!.signal!.addEventListener(
          "abort",
          () => {
            aborted++;
            reject(init!.signal!.reason);
          },
          { once: true },
        );
      });
    };
    const a = new AbortController(),
      b = new AbortController();
    const first = request(
      "/source/f?snapshot_id=s&context_id=c",
      a.signal,
    ).catch((error: Error) => error.name);
    const second = request(
      "/definitions/d?snapshot_id=s&context_id=c",
      b.signal,
    );
    await Promise.resolve();
    a.abort();
    const cancelled = await first;
    const abortsWhileShared = aborted;
    releases[0]();
    await second;
    const last = new AbortController();
    const third = request(
      "/search?snapshot_id=next&context_id=c",
      last.signal,
    ).catch((error: Error) => error.name);
    await Promise.resolve();
    last.abort();
    await third;
    const retry = request(
      "/search?snapshot_id=next&context_id=c",
      new AbortController().signal,
    );
    await Promise.resolve();
    releases.at(-1)!();
    await retry;
    return { preparations, aborted, facts, cancelled, abortsWhileShared };
  });
  expect(result).toEqual({
    preparations: 3,
    aborted: 1,
    facts: 2,
    cancelled: "AbortError",
    abortsWhileShared: 0,
  });
});

test("readiness cache stays at sixteen LRU scopes, invalidates on fact errors and token rotation", async ({
  page,
}) => {
  await harness(page);
  let prepares = 0,
    failFact = false;
  const tokens: string[] = [];
  await page.route("**/v1/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname.endsWith("/prepare")) {
      prepares++;
      tokens.push(route.request().headers().authorization);
      return route.fulfill({
        json: {
          snapshot_id: url.pathname.split("/")[3],
          context_id: "c",
          ready: true,
        },
      });
    }
    await route.fulfill(
      failFact
        ? {
            status: 503,
            json: {
              code: "corrupt_snapshot",
              message: "Corrupt facts",
              correlation_id: "verify-test",
            },
          }
        : { json: {} },
    );
  });
  await page.evaluate(async () => {
    const module = "/src/api/client.ts";
    const { request } = await import(module);
    for (let index = 0; index < 17; index++)
      await request(
        `/search?snapshot_id=s${index}&context_id=c`,
        new AbortController().signal,
      );
    await request(
      "/search?snapshot_id=s0&context_id=c",
      new AbortController().signal,
    );
  });
  expect(prepares).toBe(18);
  failFact = true;
  await page.evaluate(async () => {
    const module = "/src/api/client.ts";
    const { request } = await import(module);
    await request(
      "/search?snapshot_id=s0&context_id=c",
      new AbortController().signal,
    ).catch(() => {});
  });
  failFact = false;
  await page.evaluate(async () => {
    const module = "/src/api/client.ts";
    const { request, storeToken } = await import(module);
    await request(
      "/search?snapshot_id=s0&context_id=c",
      new AbortController().signal,
    );
    storeToken("new-token");
    await request(
      "/search?snapshot_id=s0&context_id=c",
      new AbortController().signal,
    );
  });
  expect(prepares).toBe(20);
  expect(tokens.at(-1)).toBe("Bearer new-token");
});

test("pending verification capacity is bounded and cancellation releases every slot", async ({
  page,
}) => {
  await harness(page);
  const result = await page.evaluate(async () => {
    const module = "/src/api/client.ts";
    const { request } = await import(module);
    let preparations = 0,
      aborted = 0;
    window.fetch = (_input, init) => {
      preparations++;
      return new Promise<Response>((_resolve, reject) => {
        init!.signal!.addEventListener(
          "abort",
          () => {
            aborted++;
            reject(init!.signal!.reason);
          },
          { once: true },
        );
      });
    };
    const controllers = Array.from({ length: 16 }, () => new AbortController());
    const pending = controllers.map((controller, index) =>
      request(
        `/search?snapshot_id=s${index}&context_id=c`,
        controller.signal,
      ).catch((error: Error) => error.name),
    );
    await Promise.resolve();
    const failure = await request(
      "/search?snapshot_id=overflow&context_id=c",
      new AbortController().signal,
    ).catch((error: { code: string }) => error.code);
    controllers.forEach((controller) => controller.abort());
    await Promise.all(pending);
    return { failure, preparations, aborted };
  });
  expect(result).toEqual({
    failure: "prepare_busy",
    preparations: 16,
    aborted: 16,
  });
});

test("verification has an actual abort deadline configured at thirty seconds", async ({
  page,
}) => {
  await harness(page);
  const result = await page.evaluate(async () => {
    const module = "/src/api/client.ts";
    const { request } = await import(module);
    const originalTimeout = AbortSignal.timeout;
    const deadlines: number[] = [];
    AbortSignal.timeout = (milliseconds) => {
      deadlines.push(milliseconds);
      return originalTimeout(20);
    };
    let aborted = false;
    window.fetch = (_input, init) =>
      new Promise<Response>((_resolve, reject) => {
        init!.signal!.addEventListener(
          "abort",
          () => {
            aborted = true;
            reject(init!.signal!.reason);
          },
          { once: true },
        );
      });
    const error = await request(
      "/search?snapshot_id=s&context_id=c",
      new AbortController().signal,
    ).catch((error: Error) => error.name);
    return { error, aborted, deadlines };
  });
  expect(result.error).toBe("TimeoutError");
  expect(result.aborted).toBeTruthy();
  expect(result.deadlines).toEqual([30_000, 30_000]);
});

test("the viewer shows preparation failure instead of fetching or inventing source", async ({
  page,
}) => {
  await mockApi(page);
  let failing = true;
  let factRequests = 0;
  page.on("request", (request) => {
    if (
      /\/v1\/(source|definitions|search|graph)\b/.test(
        new URL(request.url()).pathname,
      )
    )
      factRequests++;
  });
  await page.route("**/v1/snapshots/*/prepare?*", (route) =>
    failing
      ? route.fulfill({
          status: 503,
          json: {
            code: "verification_unavailable",
            message: "Snapshot verification unavailable",
            correlation_id: "prepare-viewer",
          },
        })
      : route.fulfill({
          json: {
            snapshot_id: "snapshot:two",
            context_id: "context:host",
            ready: true,
          },
        }),
  );
  await page.goto(
    "/?snapshot=snapshot%3Atwo&context=context%3Ahost&definition=definition%3Amain#token=test-token",
  );
  await expect(page.getByRole("alert").first()).toContainText(
    "Snapshot verification unavailable",
  );
  await expect(page.getByTestId("source-content")).toHaveCount(0);
  expect(factRequests).toBe(0);
  failing = false;
  await page.reload();
  await expect(page.getByTestId("source-content")).toContainText("fn main()");
  expect(factRequests).toBeGreaterThan(0);
});
