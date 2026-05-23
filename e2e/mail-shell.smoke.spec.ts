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

  await page.getByRole("button", { name: "Compose" }).click();
  const composer = page.getByRole("dialog", { name: "Composer" });
  await expect(composer).toBeVisible();
  await expect(
    composer.getByText("Drafts stay local to this device until sent or discarded."),
  ).toBeVisible();

  await composer.getByLabel("To").fill("qa@example.com");
  await composer.getByLabel("Subject").fill("E2E smoke send");
  await composer
    .getByRole("textbox", { name: "Message body" })
    .fill("Smoke body from Playwright.");
  await composer.getByRole("button", { name: "Send" }).click();

  await expect(composer.getByText("Message sent")).toBeVisible();
  await expect(composer.getByText("SMTP accepted")).toBeVisible();

  const sentEvents = await page.evaluate(() => {
    const events =
      (
        window as Window & {
          __KOMOREBI_E2E_EVENTS__?: Array<{ name: string }>;
        }
      ).__KOMOREBI_E2E_EVENTS__ ?? [];

    return events.filter((event) => event.name === "send-mail");
  });
  expect(sentEvents).toHaveLength(1);
});
