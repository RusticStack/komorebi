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

  await page.getByRole("button", { name: /Personal.+person@example\.com/ }).click();
  await page.getByRole("button", { name: /Archive folder/ }).click();
  await expect(
    page.getByRole("button", { name: /Archived receipt/ }),
  ).toBeVisible();

  await page.getByRole("checkbox", { name: "Unified inbox" }).check();
  await expect(
    page.getByRole("button", { name: /Roadmap review/ }),
  ).toBeVisible();

  await page.getByRole("searchbox").fill("phoenix");
  await expect(
    page.getByRole("button", { name: /Phoenix report/ }),
  ).toBeVisible();
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

  const composer = page.getByRole("dialog", { name: "Composer" });
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

  const sentEvents = await page.evaluate(() => {
    const events =
      (
        window as Window & {
          __KOMOREBI_E2E_EVENTS__?: Array<{
            name: string;
            request?: { draftId?: string };
          }>;
        }
      ).__KOMOREBI_E2E_EVENTS__ ?? [];

    return events.filter((event) => event.name === "send-mail");
  });
  expect(sentEvents).toHaveLength(1);
  expect(sentEvents[0]?.request?.draftId).toBe("draft_e2e_saved");
});
