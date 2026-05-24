import { expect, test } from "@playwright/test";

test("loads fixture mail and navigates search, detail, and composer workflows", async ({
  page,
}) => {
  await page.goto("/e2e/");

  await expect(page.getByText("Loading accounts")).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Roadmap review/ }),
  ).toBeVisible();

  await page.getByRole("button", { name: /Roadmap review/ }).click();
  await expect(
    page
      .getByRole("region", { name: "Selected message" })
      .getByRole("heading", { name: "Roadmap review" }),
  ).toBeVisible();

  await page.getByRole("button", { name: /Invoice question/ }).click();
  const invoiceReader = page.getByRole("region", { name: "Selected message" });
  await expect(
    invoiceReader.getByRole("heading", { name: "Invoice question" }),
  ).toBeVisible();
  await expect(
    invoiceReader.getByText("Remote content blocked", { exact: true }),
  ).toBeVisible();
  await expect(
    invoiceReader.locator('img[alt="tracking pixel"][src]'),
  ).toHaveCount(0);
  await invoiceReader.getByRole("link", { name: "invoice portal" }).click();
  await expect(invoiceReader.getByText("External link opened.")).toBeVisible();
  await invoiceReader.getByRole("button", { name: "Mark read" }).click();
  await expect(invoiceReader.getByText("Message marked read.")).toBeVisible();
  await invoiceReader.getByRole("button", { name: "Undo" }).click();
  await expect(invoiceReader.getByText("Message marked unread.")).toBeVisible();
  await invoiceReader.getByRole("button", { name: "Load remote content" }).click();
  await invoiceReader
    .getByRole("button", { name: "Allow this message" })
    .click();
  await expect(
    invoiceReader.getByText("Remote content allowed for this message").first(),
  ).toBeVisible();
  await expect(
    invoiceReader.locator(
      'img[alt="tracking pixel"][src="https://tracker.example/pixel.png"]',
    ),
  ).toHaveCount(1);
  await expect(invoiceReader.getByText("notes.txt")).toBeVisible();
  await invoiceReader.getByRole("button", { name: "Preview notes.txt" }).click();
  await expect(
    invoiceReader
      .getByRole("region", { name: "Attachment preview" })
      .getByText("E2E attachment notes"),
  ).toBeVisible();
  await invoiceReader.getByRole("button", { name: "Save notes.txt" }).click();
  await expect(
    invoiceReader.getByText("Saved notes.txt to notes.txt."),
  ).toBeVisible();

  await invoiceReader.getByRole("button", { name: "Reply all" }).click();
  let composer = page.getByRole("dialog", { name: "Composer" });
  await expect(composer).toBeVisible();
  await expect(composer.getByLabel("To")).toHaveValue(
    "Finance <finance@example.com>",
  );
  await expect(
    composer.getByRole("combobox", { exact: true, name: "CC" }),
  ).toHaveValue("Ada <ada@example.com>");
  await expect(composer.getByLabel("Subject")).toHaveValue(
    "Re: Invoice question",
  );
  await expect(composer.locator(".composer-editor")).toContainText(
    "Can you confirm the invoice number?",
  );
  await composer.getByRole("button", { name: "Send" }).click();
  await expect(composer.getByText("Message sent")).toBeVisible();
  await composer.getByRole("button", { name: "Close" }).click();
  await expect(composer).toBeHidden();

  await invoiceReader
    .getByRole("button", { name: "Forward attachments" })
    .click();
  composer = page.getByRole("dialog", { name: "Composer" });
  await expect(composer).toBeVisible();
  await expect(composer.getByLabel("To")).toHaveValue("");
  await expect(composer.getByLabel("Subject")).toHaveValue(
    "Fwd: Invoice question",
  );
  await expect(composer.locator(".composer-editor")).toContainText(
    "Forwarded message",
  );
  await expect(composer.getByText(/invoice\.pdf/)).toBeVisible();
  await expect(composer.getByText(/notes\.txt/)).toBeVisible();
  await composer.getByRole("button", { name: "Save draft" }).click();
  await expect(composer.getByText("Draft saved")).toBeVisible();
  await composer.getByRole("button", { name: "Close" }).click();
  await expect(composer).toBeHidden();

  await page.getByRole("button", { name: /Personal.+person@example\.com/ }).click();
  await page.getByRole("button", { name: /Archive folder/ }).click();
  await expect(
    page.getByRole("button", { name: /Archived receipt/ }),
  ).toBeVisible();

  await page.getByRole("checkbox", { name: "Unified inbox" }).check();
  await expect(
    page.getByRole("button", { name: /Roadmap review/ }),
  ).toBeVisible();

  await page.getByRole("searchbox").fill("from:Research phoenix");
  await expect(
    page.getByRole("button", { name: /Phoenix report/ }),
  ).toBeVisible();
  await expect(page.getByText("From: Research")).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Phoenix report/ }).locator("mark").first(),
  ).toHaveText("Research");
  await page.getByRole("button", { name: "Save search" }).click();
  await page.getByRole("searchbox").fill("");
  const savedAdvancedSearch = page.getByRole("button", {
    exact: true,
    name: "from:Research phoenix",
  });
  await expect(
    savedAdvancedSearch,
  ).toBeVisible();
  await savedAdvancedSearch.click();
  await expect(page.getByText("Personal / Archive")).toBeVisible();

  await page.getByRole("button", { name: /Phoenix report/ }).click();
  await expect(
    page
      .getByRole("region", { name: "Selected message" })
      .getByRole("heading", { name: "Phoenix report" }),
  ).toBeVisible();

  await page.getByRole("button", { name: "Drafts, 1 saved locally" }).click();
  const drafts = page.getByRole("dialog", { name: "Local drafts" });
  await expect(drafts.getByText("Saved local draft")).toBeVisible();
  await expect(drafts.getByText("1 attachment")).toBeVisible();
  await drafts.getByRole("button", { name: "Open draft: Saved local draft" }).click();

  composer = page.getByRole("dialog", { name: "Composer" });
  await expect(composer).toBeVisible();
  await expect(
    composer.getByText("Drafts stay local to this device until sent or discarded."),
  ).toBeVisible();
  await expect(composer.getByLabel("To")).toHaveValue("qa@example.com");
  await expect(composer.getByLabel("Subject")).toHaveValue("Saved local draft");
  await expect(composer.getByText("briefing.txt - text/plain - 5 B")).toBeVisible();
  await composer.getByRole("button", { name: "Send" }).click();

  await expect(composer.getByText("Message sent")).toBeVisible();
  await expect(composer.getByText("SMTP accepted")).toBeVisible();

  const events = await page.evaluate(() => {
    const events =
      (
        window as Window & {
          __KOMOREBI_E2E_EVENTS__?: Array<{
            name: string;
            request?: {
              accountId?: string;
              cursor?: { messageId?: string; sortKey?: string } | null;
              draftId?: string;
              folderId?: string;
              limit?: number;
              messageId?: string;
              action?: { kind?: string; read?: boolean };
              scope?: string;
              url?: string;
              userRequested?: boolean;
              inReplyTo?: string | null;
              references?: string[];
              attachmentIds?: string[];
              sourceAttachmentIds?: string[];
              to?: Array<{ address?: string; name?: string | null }>;
              cc?: Array<{ address?: string; name?: string | null }>;
              subject?: string;
              unifiedInbox?: boolean;
            };
          }>;
        }
      ).__KOMOREBI_E2E_EVENTS__ ?? [];

    return {
      remoteContent: events.filter(
        (event) => event.name === "allow-remote-content",
      ),
      externalLinks: events.filter(
        (event) => event.name === "open-external-link",
      ),
      mailActions: events.filter(
        (event) => event.name === "perform-mail-action",
      ),
      messagePages: events.filter(
        (event) => event.name === "load-message-page",
      ),
      savedDrafts: events.filter((event) => event.name === "save-draft"),
      sent: events.filter((event) => event.name === "send-mail"),
      stagedForwardedAttachments: events.filter(
        (event) => event.name === "stage-forwarded-attachments",
      ),
    };
  });
  expect(events.externalLinks).toHaveLength(1);
  expect(events.messagePages).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        request: expect.objectContaining({
          accountId: null,
          cursor: null,
          folderId: null,
          limit: 100,
          unifiedInbox: true,
        }),
      }),
    ]),
  );
  expect(events.externalLinks[0]?.request).toMatchObject({
    url: "https://example.com/invoice",
    userRequested: true,
  });
  expect(events.remoteContent).toHaveLength(1);
  expect(events.remoteContent[0]?.request).toMatchObject({
    accountId: "acct_work",
    folderId: "folder_work_inbox",
    messageId: "msg_invoice",
    scope: "message",
    userRequested: true,
  });
  expect(events.mailActions).toHaveLength(2);
  expect(events.mailActions[0]?.request).toMatchObject({
    accountId: "acct_work",
    action: { kind: "mark-read", read: true },
    folderId: "folder_work_inbox",
    messageId: "msg_invoice",
    userRequested: true,
  });
  expect(events.mailActions[1]?.request).toMatchObject({
    action: { kind: "mark-read", read: false },
    userRequested: true,
  });
  expect(events.stagedForwardedAttachments).toHaveLength(1);
  const forwardedDraftId =
    events.stagedForwardedAttachments[0]?.request?.draftId ?? "";
  expect(forwardedDraftId).toMatch(/^draft_forward_msg_invoice_/);
  expect(events.stagedForwardedAttachments[0]?.request).toMatchObject({
    accountId: "acct_work",
    draftId: forwardedDraftId,
    sourceAttachmentIds: ["att_invoice", "att_invoice_notes"],
    sourceFolderId: "folder_work_inbox",
    sourceMessageId: "msg_invoice",
    userRequested: true,
  });
  expect(events.savedDrafts).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        request: expect.objectContaining({
          attachmentIds: [
            "catt_forwarded_att_invoice",
            "catt_forwarded_att_invoice_notes",
          ],
          draftId: forwardedDraftId,
          inReplyTo: null,
          references: [],
          subject: "Fwd: Invoice question",
        }),
      }),
    ]),
  );
  expect(events.sent).toHaveLength(2);
  expect(events.sent[0]?.request).toMatchObject({
    cc: [{ address: "ada@example.com", name: "Ada" }],
    draftId: expect.stringMatching(/^draft_reply_all_msg_invoice_/),
    inReplyTo: "<invoice-question@example.com>",
    references: ["<thread-root@example.com>", "<invoice-question@example.com>"],
    subject: "Re: Invoice question",
    to: [{ address: "finance@example.com", name: "Finance" }],
  });
  expect(events.sent[1]?.request?.draftId).toBe("draft_e2e_saved");
});
