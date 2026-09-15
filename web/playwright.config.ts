import { defineConfig } from "@playwright/test";

const port = Number(process.env.ATLAS_WEB_TEST_PORT ?? "4173");
if (!Number.isInteger(port) || port < 1024 || port > 65535)
  throw new Error("ATLAS_WEB_TEST_PORT must be an integer in 1024..65535");
const baseURL = `http://127.0.0.1:${port}`;

export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  workers: 3,
  timeout: 30_000,
  expect: { timeout: 10_000 },
  reporter: [["list"], ["html", { open: "never" }]],
  use: {
    baseURL,
    viewport: { width: 1440, height: 900 },
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  webServer: {
    command: `npm run dev -- --port ${port} --strictPort`,
    url: baseURL,
    reuseExistingServer: false,
    timeout: 30_000,
  },
});
