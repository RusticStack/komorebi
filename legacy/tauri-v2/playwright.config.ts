import { defineConfig, devices } from "@playwright/test";

const e2ePort = Number(process.env.KOMOREBI_E2E_PORT ?? 1430);
const baseURL = `http://127.0.0.1:${e2ePort}`;

export default defineConfig({
  forbidOnly: Boolean(process.env.CI),
  fullyParallel: false,
  outputDir: "test-results/e2e",
  reporter: [
    ["list"],
    ["html", { open: "never", outputFolder: "playwright-report" }],
  ],
  testDir: "e2e",
  use: {
    ...devices["Desktop Chrome"],
    baseURL,
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
    video: "retain-on-failure",
  },
  webServer: {
    command: `pnpm vite --mode e2e --host 127.0.0.1 --port ${e2ePort} --strictPort`,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
    url: baseURL,
  },
});
