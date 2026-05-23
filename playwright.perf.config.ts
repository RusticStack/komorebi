import { defineConfig, devices } from "@playwright/test";

const perfPort = Number(process.env.KOMOREBI_PERF_PORT ?? 1431);
const baseURL = `http://127.0.0.1:${perfPort}`;

export default defineConfig({
  forbidOnly: Boolean(process.env.CI),
  fullyParallel: false,
  outputDir: "test-results/perf",
  reporter: [["list"]],
  testDir: "perf",
  timeout: 120_000,
  use: {
    ...devices["Desktop Chrome"],
    baseURL,
    screenshot: "off",
    trace: "off",
    video: "off",
  },
  webServer: {
    command: `pnpm vite --mode e2e --host 127.0.0.1 --port ${perfPort} --strictPort`,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
    url: baseURL,
  },
  workers: 1,
});
