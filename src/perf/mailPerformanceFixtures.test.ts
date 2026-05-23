import { describe, expect, it } from "vitest";
import {
  createPerformanceMailFixture,
  normalizePerformanceMessageCount,
  performanceSearchNeedle,
  searchPerformanceMailFixture,
  PERFORMANCE_FIXTURE_MESSAGE_COUNT,
} from "./mailPerformanceFixtures";

describe("mail performance fixtures", () => {
  it("generates a deterministic 10,000 message mailbox with representative attachments", () => {
    const fixture = createPerformanceMailFixture();

    expect(fixture.metadata.messageCount).toBe(
      PERFORMANCE_FIXTURE_MESSAGE_COUNT,
    );
    expect(fixture.data.messages).toHaveLength(PERFORMANCE_FIXTURE_MESSAGE_COUNT);
    expect(fixture.data.accounts).toHaveLength(2);
    expect(fixture.data.folders).toHaveLength(2);
    expect(fixture.metadata.attachmentCount).toBeGreaterThanOrEqual(500);
    expect(fixture.metadata.inlineAttachmentCount).toBeGreaterThan(0);
    expect(fixture.metadata.totalAttachmentBytes).toBeGreaterThan(0);
    expect(
      fixture.data.messages.some((message) =>
        message.attachments.some(
          (attachment) => attachment.contentType === "application/pdf",
        ),
      ),
    ).toBe(true);
    expect(
      fixture.data.messages.some((message) =>
        message.attachments.some(
          (attachment) => attachment.disposition === "inline",
        ),
      ),
    ).toBe(true);
  });

  it("searches generated sender, subject, body, and attachment-heavy fixture terms", () => {
    const fixture = createPerformanceMailFixture();

    expect(
      searchPerformanceMailFixture(fixture.data, {
        accountId: null,
        folderId: null,
        query: performanceSearchNeedle(),
        limit: 10,
      }).results,
    ).toHaveLength(1);

    expect(
      searchPerformanceMailFixture(fixture.data, {
        accountId: null,
        folderId: null,
        query: "Perf Sender 04",
        limit: 5,
      }).results,
    ).toHaveLength(5);

    expect(
      searchPerformanceMailFixture(fixture.data, {
        accountId: null,
        folderId: null,
        query: "attachment-heavy-20",
        limit: 10,
      }).results[0]?.attachments,
    ).toHaveLength(1);
  });

  it("bounds requested message counts to production-sized local fixture ranges", () => {
    expect(normalizePerformanceMessageCount(10)).toBe(
      PERFORMANCE_FIXTURE_MESSAGE_COUNT,
    );
    expect(normalizePerformanceMessageCount(12_345.9)).toBe(12_345);
    expect(normalizePerformanceMessageCount(100_000)).toBe(50_000);
    expect(normalizePerformanceMessageCount(Number.NaN)).toBe(
      PERFORMANCE_FIXTURE_MESSAGE_COUNT,
    );
  });
});
