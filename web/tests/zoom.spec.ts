import { chromium, expect, test } from "@playwright/test";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { mockApi } from "./fixtures";

test("genuine Chromium 200 percent browser zoom preserves source and navigation", async () => {
  const profile = await mkdtemp(join(tmpdir(), "atlas-zoom-"));
  await mkdir(join(profile, "Default"));
  // Chromium's default storage partition key is "x"; zoom factors are 1.2^level.
  await writeFile(
    join(profile, "Default", "Preferences"),
    JSON.stringify({
      partition: { default_zoom_level: { x: Math.log(2) / Math.log(1.2) } },
    }),
  );
  const context = await chromium.launchPersistentContext(profile, {
    channel: "chromium",
    headless: true,
    viewport: null,
    args: ["--window-size=1440,1000"],
  });
  try {
    const page = context.pages()[0];
    await mockApi(page);
    await page.goto("http://127.0.0.1:4173/#token=test-token");
    await expect(page.getByTestId("source-content")).toContainText("fn main()");
    const zoom = await page.evaluate(() => ({
      dpr: devicePixelRatio,
      outer: outerWidth,
      inner: innerWidth,
      cssZoom: getComputedStyle(document.body).zoom,
      pageScale: visualViewport?.scale,
    }));
    expect(zoom.dpr).toBeCloseTo(2, 4);
    expect(zoom.outer / zoom.inner).toBeCloseTo(2, 1);
    expect(zoom.cssZoom).toBe("1");
    expect(zoom.pageScale).toBe(1);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBeTruthy();
    await page
      .getByRole("button", { name: "Flow", exact: true })
      .press("Enter");
    await expect(page).toHaveURL(/view=flow/);
    await expect(page.locator(".cm-editor")).toBeVisible();
    const editor = await page.locator(".cm-editor").boundingBox();
    expect(editor!.y).toBeLessThan(await page.evaluate(() => innerHeight));
    await test.info().attach("browser-zoom-metrics", {
      body: JSON.stringify({ ...zoom, editor }),
      contentType: "application/json",
    });
    const session = await context.newCDPSession(page);
    const screenshot = await session.send("Page.captureScreenshot", {
      format: "png",
      captureBeyondViewport: false,
      fromSurface: true,
    });
    await writeFile(
      test.info().outputPath("browser-zoom-200.png"),
      Buffer.from(screenshot.data, "base64"),
    );
  } finally {
    await context.close();
    await rm(profile, { recursive: true, force: true });
  }
});
