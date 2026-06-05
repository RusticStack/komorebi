import { mkdir } from "node:fs/promises";
import { expect, test } from "@playwright/test";

const screenshotDir = "test-results/screenshots";

async function expectNoHorizontalOverflow(page: import("@playwright/test").Page) {
  const overflow = await page.evaluate(() => {
    const documentWidth = document.documentElement.scrollWidth;
    const viewportWidth = document.documentElement.clientWidth;

    return {
      documentWidth,
      viewportWidth,
      overflowing: documentWidth > viewportWidth + 1,
    };
  });

  expect(overflow, JSON.stringify(overflow)).toEqual(
    expect.objectContaining({ overflowing: false }),
  );
}

test.describe("Windows 11 shell refresh", () => {
  test("renders custom chrome and account setup wizard on desktop", async ({
    page,
  }) => {
    await mkdir(screenshotDir, { recursive: true });
    await page.setViewportSize({ width: 1440, height: 920 });
    await page.goto("/e2e/");

    await expect(page.getByText("Sunlit mail")).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Minimize window" }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Maximize window" }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Close window" }),
    ).toBeVisible();

    const addAccount = page.getByRole("button", { name: "Add account" });
    await expect(addAccount).toBeEnabled();
    await addAccount.click();

    const settings = page.getByRole("region", { name: "Settings" });
    const accountsRegion = settings.getByRole("region", { name: "Accounts" });
    await accountsRegion.scrollIntoViewIfNeeded();
    await expect(
      accountsRegion.getByRole("heading", { name: "Add mail account" }),
    ).toBeVisible();
    await expect(
      accountsRegion.getByRole("button", { name: "Continue" }),
    ).toBeVisible();

    await accountsRegion.getByLabel("Email address").fill("person@fastmail.com");
    await accountsRegion.getByRole("button", { name: "Continue" }).click();

    await expect(
      accountsRegion.getByText("Generic IMAP/SMTP", { exact: true }),
    ).toBeVisible();
    await expect(accountsRegion.getByLabel("Username")).toHaveValue(
      "person@fastmail.com",
    );
    await accountsRegion.getByText("Server settings").click();
    await expect(accountsRegion.getByLabel("IMAP host")).toHaveValue(
      "imap.fastmail.com",
    );
    await expect(accountsRegion.getByLabel("SMTP host")).toHaveValue(
      "smtp.fastmail.com",
    );
    await expect(
      accountsRegion.getByText("Generic IMAP/SMTP is ready. Settings found."),
    ).toBeVisible();

    await expectNoHorizontalOverflow(page);
    await page.screenshot({
      fullPage: true,
      path: `${screenshotDir}/win11-shell-desktop.png`,
    });
  });

  test("keeps the refreshed shell usable on a narrow viewport", async ({
    page,
  }) => {
    await mkdir(screenshotDir, { recursive: true });
    await page.setViewportSize({ width: 390, height: 860 });
    await page.goto("/e2e/");

    await expect(page.getByText("Sunlit mail")).toBeVisible();
    await page.getByRole("button", { name: "Add account" }).click();

    const settings = page.getByRole("region", { name: "Settings" });
    const accountsRegion = settings.getByRole("region", { name: "Accounts" });
    await accountsRegion.scrollIntoViewIfNeeded();
    await expect(
      accountsRegion.getByRole("heading", { name: "Add mail account" }),
    ).toBeVisible();
    await accountsRegion.getByLabel("Email address").fill("ada@gmail.com");
    await accountsRegion.getByRole("button", { name: "Continue" }).click();
    await expect(accountsRegion.getByText("Gmail", { exact: true })).toBeVisible();
    await expect(accountsRegion.getByLabel("OAuth client ID")).toBeVisible();
    await expect(
      accountsRegion.getByText("Gmail is ready. Ready to sign in."),
    ).toBeVisible();

    await expectNoHorizontalOverflow(page);
    await page.screenshot({
      fullPage: true,
      path: `${screenshotDir}/win11-shell-narrow.png`,
    });
  });
});
