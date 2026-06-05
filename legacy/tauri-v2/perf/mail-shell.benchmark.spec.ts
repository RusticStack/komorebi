import { expect, test, type Page } from "@playwright/test";

const PERFORMANCE_MESSAGE_COUNT = 10_000;

type HeapUsage = {
  usedBytes: number;
  totalBytes: number;
};

type MailShellBenchmarkReport = {
  fixture: {
    messageCount: number;
    attachmentCount: number;
    inlineAttachmentCount: number;
    totalAttachmentBytes: number;
    searchNeedle: string;
    advancedSearchQuery: string;
  };
  measurements: {
    initialLoadMs: number;
    searchLatencyMs: number;
    advancedSearchLatencyMs: number;
    scrollToLastRowMs: number;
    selectLastRowMs: number;
    renderedRowCountAfterScroll: number;
    heapAfterLoad: HeapUsage;
    heapAfterSearch: HeapUsage;
    heapAfterListInteraction: HeapUsage;
  };
};

test("reports mail shell memory, search, and list interaction metrics", async ({
  page,
}, testInfo) => {
  const loadStartedAt = await page.evaluate(() => performance.now());
  await page.goto(`/e2e/?fixture=performance&messages=${PERFORMANCE_MESSAGE_COUNT}`);
  const list = page.getByTestId("message-virtual-list");
  await expect(list).toHaveAttribute(
    "data-message-count",
    String(PERFORMANCE_MESSAGE_COUNT),
    { timeout: 30_000 },
  );
  await expect(
    page.getByRole("button", { name: /Performance message 1\b/ }),
  ).toBeVisible();
  const initialLoadMs = await elapsedSince(page, loadStartedAt);
  const heapAfterLoad = await readHeapUsage(page);

  const fixtureMetadata = await page.evaluate(() => {
    const fixture = window.__KOMOREBI_PERF_FIXTURE__;
    if (!fixture) {
      throw new Error("Performance fixture metadata was not published.");
    }

    return fixture;
  });

  const searchStartedAt = await page.evaluate(() => performance.now());
  await page.getByRole("searchbox").fill(fixtureMetadata.searchNeedle);
  await expect(
    page.getByRole("button", {
      name: new RegExp(`Performance message 9999.*${fixtureMetadata.searchNeedle}`),
    }),
  ).toBeVisible({ timeout: 30_000 });
  const searchLatencyMs = await elapsedSince(page, searchStartedAt);
  const heapAfterSearch = await readHeapUsage(page);
  const advancedSearchStartedAt = await page.evaluate(() => performance.now());
  await page.getByRole("searchbox").fill(fixtureMetadata.advancedSearchQuery);
  await expect(
    page.getByRole("button", {
      name: new RegExp(`Performance message 9999.*${fixtureMetadata.searchNeedle}`),
    }),
  ).toBeVisible({ timeout: 30_000 });
  await expect(page.getByText(`Subject: ${fixtureMetadata.searchNeedle}`)).toBeVisible();
  const advancedSearchLatencyMs = await elapsedSince(page, advancedSearchStartedAt);

  await page.getByRole("searchbox").fill("");
  await expect(list).toHaveAttribute(
    "data-message-count",
    String(PERFORMANCE_MESSAGE_COUNT),
    { timeout: 30_000 },
  );

  const scrollToLastRowMs = await scrollToLastMessageRow(page);
  await expect(
    page.getByRole("button", { name: /Performance message 10000\b/ }),
  ).toBeVisible({ timeout: 30_000 });
  const renderedRowCountAfterScroll = await page
    .locator("[data-message-index]")
    .count();

  const selectStartedAt = await page.evaluate(() => performance.now());
  await page.getByRole("button", { name: /Performance message 10000\b/ }).click();
  await expect(
    page
      .getByRole("region", { name: "Selected message" })
      .getByRole("heading", { name: "Performance message 10000" }),
  ).toBeVisible({ timeout: 30_000 });
  const selectLastRowMs = await elapsedSince(page, selectStartedAt);
  const heapAfterListInteraction = await readHeapUsage(page);

  const report: MailShellBenchmarkReport = {
    fixture: {
      attachmentCount: fixtureMetadata.attachmentCount,
      advancedSearchQuery: fixtureMetadata.advancedSearchQuery,
      inlineAttachmentCount: fixtureMetadata.inlineAttachmentCount,
      messageCount: fixtureMetadata.messageCount,
      searchNeedle: fixtureMetadata.searchNeedle,
      totalAttachmentBytes: fixtureMetadata.totalAttachmentBytes,
    },
    measurements: {
      heapAfterListInteraction,
      heapAfterLoad,
      heapAfterSearch,
      advancedSearchLatencyMs: roundMetric(advancedSearchLatencyMs),
      initialLoadMs: roundMetric(initialLoadMs),
      renderedRowCountAfterScroll,
      scrollToLastRowMs: roundMetric(scrollToLastRowMs),
      searchLatencyMs: roundMetric(searchLatencyMs),
      selectLastRowMs: roundMetric(selectLastRowMs),
    },
  };

  expect(report.fixture.messageCount).toBe(PERFORMANCE_MESSAGE_COUNT);
  expect(report.fixture.attachmentCount).toBeGreaterThan(0);
  expect(report.measurements.renderedRowCountAfterScroll).toBeGreaterThan(0);
  expect(report.measurements.renderedRowCountAfterScroll).toBeLessThan(80);
  expect(report.measurements.searchLatencyMs).toBeGreaterThanOrEqual(0);
  expect(report.measurements.advancedSearchLatencyMs).toBeGreaterThanOrEqual(0);
  expect(report.measurements.scrollToLastRowMs).toBeGreaterThanOrEqual(0);

  const reportJson = JSON.stringify(report, null, 2);
  console.log(`KOMOREBI_MAIL_SHELL_BENCHMARK ${reportJson}`);
  await testInfo.attach("mail-shell-performance-report", {
    body: reportJson,
    contentType: "application/json",
  });
});

async function readHeapUsage(page: Page): Promise<HeapUsage> {
  const session = await page.context().newCDPSession(page);
  try {
    const usage = await session.send("Runtime.getHeapUsage");
    return {
      totalBytes: Math.round(usage.totalSize),
      usedBytes: Math.round(usage.usedSize),
    };
  } finally {
    await session.detach();
  }
}

async function elapsedSince(page: Page, startedAt: number) {
  return (await page.evaluate(() => performance.now())) - startedAt;
}

async function scrollToLastMessageRow(page: Page) {
  return page.evaluate(async () => {
    const list = document.querySelector<HTMLElement>(
      "[data-testid='message-virtual-list']",
    );
    if (!list) {
      throw new Error("Message virtual list was not mounted.");
    }

    const startedAt = performance.now();
    list.scrollTop = list.scrollHeight - list.clientHeight;
    list.dispatchEvent(new Event("scroll", { bubbles: true }));
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
    );

    return performance.now() - startedAt;
  });
}

function roundMetric(value: number) {
  return Math.round(value * 100) / 100;
}
