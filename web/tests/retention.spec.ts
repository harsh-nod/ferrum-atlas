import { expect, test, type Page, type Route } from "@playwright/test";
import { mockApi } from "./fixtures";
import type { Bookmark } from "../src/state";

const storage = "ferrum-atlas.bookmarks";
const legacy: Bookmark = {
  id: "old-bookmark",
  label: "fixture::main",
  note: "Preserve this revision",
  location: {
    snapshot: "snapshot:two",
    context: "context:host",
    definition: "definition:main",
    edge: "",
    view: "explore",
    depth: 2,
    direction: "both",
  },
};
async function open(page: Page) {
  await page.goto("/#token=test-token");
  await expect(page.getByTestId("source-content")).toContainText("fn main()");
}
async function respond(route: Route, overrides = {}) {
  const url = new URL(route.request().url());
  await route.fulfill({
    json: {
      snapshot_id: decodeURIComponent(url.pathname.split("/")[3]),
      ...route.request().postDataJSON(),
      retained: url.pathname.endsWith("/pin"),
      ...overrides,
    },
  });
}

test("bookmark names survive reload, pin exact scope, and unpin only that bookmark", async ({
  page,
}) => {
  await mockApi(page);
  const requests: { path: string; name: string; context_id: string }[] = [];
  await page.route("**/v1/snapshots/*/*", async (route) => {
    expect(route.request().headers().authorization).toBe("Bearer test-token");
    requests.push({
      path: new URL(route.request().url()).pathname,
      ...route.request().postDataJSON(),
    });
    await respond(route);
  });
  await open(page);
  await page.getByLabel("Review note", { exact: true }).fill("Pinned note");
  await page
    .getByRole("button", { name: "Pin selection", exact: true })
    .click();
  await expect(page.locator(".bookmark-row small")).toHaveText(
    "Snapshot retained",
  );
  const first = requests[0];
  expect(first.path).toBe("/v1/snapshots/snapshot%3Atwo/pin");
  expect(first.context_id).toBe("context:host");
  expect(first.name).toMatch(/^browser-[a-f0-9-]{36}-[a-f0-9-]{36}$/);
  await page.reload();
  await expect(page.locator(".bookmark-row small")).toHaveText(
    "Snapshot retained",
  );
  await expect(page.getByLabel("Review note", { exact: true })).toHaveValue(
    "Pinned note",
  );
  expect(
    requests
      .filter((item) => item.path.endsWith("/pin"))
      .every((item) => item.name === first.name),
  ).toBeTruthy();
  await page.getByLabel("Expansion depth").selectOption("3");
  await page
    .getByRole("button", { name: "Pin selection", exact: true })
    .click();
  await expect(page.locator(".bookmark-row small")).toHaveText([
    "Snapshot retained",
    "Snapshot retained",
  ]);
  const second = requests.at(-1)!;
  expect(second.name).not.toBe(first.name);
  await page
    .getByRole("button", { name: "Unpin selection", exact: true })
    .click();
  await expect(page.locator(".bookmark-row")).toHaveCount(1);
  expect(requests.at(-1)).toEqual({
    ...second,
    path: "/v1/snapshots/snapshot%3Atwo/unpin",
  });
  const saved = await page.evaluate(
    (key) => localStorage.getItem(key),
    storage,
  );
  expect(saved).toContain(first.name);
  expect(saved).not.toContain(second.name);
  expect(saved).not.toContain("retained");
  expect(saved).not.toContain("test-token");
});

for (const code of [429, 503]) {
  test(`legacy bookmarks fail visibly without false retention on HTTP ${code}, then retry`, async ({
    page,
  }) => {
    await mockApi(page);
    await page.addInitScript(
      ({ storage, legacy }) => {
        localStorage.setItem(storage, JSON.stringify([legacy]));
      },
      { storage, legacy },
    );
    let failing = true;
    const names: string[] = [];
    await page.route("**/v1/snapshots/*/*", async (route) => {
      names.push(route.request().postDataJSON().name);
      if (failing)
        return route.fulfill({
          status: code,
          json: {
            code: "retention_unavailable",
            message: "Retention unavailable",
            correlation_id: "pin-test",
          },
        });
      await respond(route);
    });
    await open(page);
    await expect(page.locator(".bookmark-row small")).toHaveText(
      "Snapshot not confirmed retained",
    );
    await expect(page.getByRole("alert")).toContainText(
      "1 reading-trail snapshot(s) not confirmed retained",
    );
    await expect(page.getByLabel("Review note", { exact: true })).toHaveValue(
      legacy.note,
    );
    failing = false;
    await page
      .getByRole("button", { name: "Retry snapshot retention" })
      .click();
    await expect(page.locator(".bookmark-row small")).toHaveText(
      "Snapshot retained",
    );
    expect(new Set(names).size).toBe(1);
  });
}

test("retention remains pending until the matching response and refuses a mismatched acknowledgement", async ({
  page,
}) => {
  await mockApi(page);
  let finish: (() => void) | undefined;
  await page.route("**/v1/snapshots/*/*", async (route) => {
    await new Promise<void>((resolve) => {
      finish = resolve;
    });
    await respond(route, { snapshot_id: "snapshot:wrong" });
  });
  await open(page);
  await page
    .getByRole("button", { name: "Pin selection", exact: true })
    .click();
  await expect(page.locator(".bookmark-row small")).toHaveText(
    "Retention pending",
  );
  await expect(
    page.getByRole("button", { name: "Unpin selection", exact: true }),
  ).toBeDisabled();
  await expect(page.getByLabel("Review note", { exact: true })).toBeDisabled();
  await expect.poll(() => Boolean(finish)).toBeTruthy();
  finish!();
  await expect(page.locator(".bookmark-row small")).toHaveText(
    "Snapshot not confirmed retained",
  );
  await expect(page.getByRole("alert")).toContainText(
    "did not match the selection",
  );
});

test("failed unpin keeps the bookmark recoverable and never claims confirmed retention", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/v1/snapshots/*/unpin", (route) =>
    route.fulfill({
      status: 503,
      json: {
        code: "busy",
        message: "Retention store busy",
        correlation_id: "unpin-test",
      },
    }),
  );
  await open(page);
  await page
    .getByRole("button", { name: "Pin selection", exact: true })
    .click();
  await expect(page.locator(".bookmark-row small")).toHaveText(
    "Snapshot retained",
  );
  await page
    .getByRole("button", { name: "Unpin selection", exact: true })
    .click();
  await expect(page.locator(".bookmark-row small")).toHaveText(
    "Snapshot not confirmed retained",
  );
  await expect(page.getByRole("alert")).toContainText("Retention store busy");
  await page.getByRole("button", { name: "Retry snapshot retention" }).click();
  await expect(page.locator(".bookmark-row small")).toHaveText(
    "Snapshot retained",
  );
});

test("storage failure does not create an untracked server pin", async ({
  page,
}) => {
  await mockApi(page);
  await page.addInitScript((key) => {
    const original = Storage.prototype.setItem;
    Storage.prototype.setItem = function (name, value) {
      if (name === key)
        throw new DOMException("Storage quota exceeded", "QuotaExceededError");
      original.call(this, name, value);
    };
  }, storage);
  let pins = 0;
  page.on("request", (request) => {
    if (request.url().endsWith("/pin")) pins++;
  });
  await open(page);
  await page
    .getByRole("button", { name: "Pin selection", exact: true })
    .click();
  await expect(page.getByRole("alert")).toContainText("Storage quota exceeded");
  await expect(page.locator(".bookmark-row")).toHaveCount(0);
  expect(pins).toBe(0);
});

test("a full trail preserves all forty server pin identities without silent eviction", async ({
  page,
}) => {
  await mockApi(page);
  await page.addInitScript(
    ({ storage, legacy }) => {
      localStorage.setItem(
        storage,
        JSON.stringify(
          Array.from({ length: 40 }, (_, index) => ({
            ...legacy,
            id: `saved-${index}`,
            location: {
              ...legacy.location,
              definition: `definition:saved-${index}`,
            },
          })),
        ),
      );
    },
    { storage, legacy },
  );
  await open(page);
  await expect(page.locator(".bookmark-row .retained")).toHaveCount(40);
  await page
    .getByRole("button", { name: "Pin selection", exact: true })
    .click();
  await expect(page.getByRole("alert")).toContainText("Reading trail is full");
  await expect(page.locator(".bookmark-row")).toHaveCount(40);
  const names = await page.evaluate(
    (key) =>
      JSON.parse(localStorage.getItem(key)!).map(
        (item: Bookmark) => item.retentionName,
      ),
    storage,
  );
  expect(new Set(names).size).toBe(40);
  await page
    .getByRole("button", {
      name: `Remove trail pin ${legacy.label}`,
      exact: true,
    })
    .first()
    .click();
  await expect(page.locator(".bookmark-row")).toHaveCount(39);
  await page
    .getByRole("button", { name: "Pin selection", exact: true })
    .click();
  await expect(page.locator(".bookmark-row .retained")).toHaveCount(40);
});

test("mobile keyboard pinning keeps retention status and navigation usable", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await mockApi(page);
  await open(page);
  const pin = page.getByRole("button", { name: "Pin selection", exact: true });
  await pin.focus();
  await page.keyboard.press("Enter");
  await expect(
    page.getByRole("button", { name: "Unpin selection", exact: true }),
  ).toBeEnabled();
  await page.getByRole("button", { name: "Scope and bookmarks" }).click();
  await expect(page.locator(".bookmark-row small")).toHaveText(
    "Snapshot retained",
  );
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBeTruthy();
  await page.screenshot({
    path: test.info().outputPath("retained-mobile.png"),
  });
  await page.locator(".bookmark-row").focus();
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("source-content")).toContainText("fn main()");
});
