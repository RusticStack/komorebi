import { expect, type Page, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";

const fixedNowScript = `
(() => {
  const fixedNow = new Date("2026-05-25T12:00:00").getTime();
  const RealDate = Date;
  class FixedDate extends RealDate {
    constructor(...args) {
      super(...(args.length > 0 ? args : [fixedNow]));
    }
    static now() {
      return fixedNow;
    }
    static parse(value) {
      return RealDate.parse(value);
    }
    static UTC(...args) {
      return RealDate.UTC(...args);
    }
  }
  globalThis.Date = FixedDate;
})();
`;

const languageCases = [
  {
    code: "en",
    lastSyncLabel: "Last sync: Today, 8:15 AM",
    settingsName: "Settings",
    yesterdayLabel: "Yesterday, 9:30 AM",
  },
  {
    code: "pt-PT",
    lastSyncLabel: "Ultima sincronizacao: Hoje, 08:15",
    settingsName: "Definicoes",
    yesterdayLabel: "Ontem, 09:30",
  },
] as const;

const viewports = [
  { height: 960, name: "desktop", width: 1440 },
  { height: 860, name: "narrow", width: 390 },
] as const;

for (const language of languageCases) {
  for (const viewport of viewports) {
    test(`long localized strings fit without horizontal overflow in ${language.code} ${viewport.name}`, async ({
      page,
    }, testInfo) => {
      await page.setViewportSize(viewport);
      await page.addInitScript(fixedNowScript);
      await page.goto(`/e2e/?fixture=long-strings&language=${language.code}`);

      const message = page.getByRole("button", {
        name: /Regionalization layout verification/,
      });
      await expect(message).toBeVisible();
      await expect(page.getByText(language.yesterdayLabel)).toBeVisible();
      await expect(page.getByText(language.lastSyncLabel)).toBeVisible();
      await message.click();
      await expect(
        page
          .getByRole("region", { name: /Selected message|Mensagem selecionada/ })
          .getByRole("heading", { name: /Regionalization layout verification/ }),
      ).toBeVisible();
      await expectNoHorizontalOverflow(page);
      const readerScreenshot = `test-results/screenshots/locale-${language.code}-${viewport.name}-reader.png`;
      await mkdir("test-results/screenshots", { recursive: true });
      await page.screenshot({ fullPage: true, path: readerScreenshot });
      await testInfo.attach(`locale-${language.code}-${viewport.name}-reader`, {
        path: readerScreenshot,
        contentType: "image/png",
      });

      await page.getByRole("button", { name: language.settingsName }).click();
      await expectNoHorizontalOverflow(page);
      const settingsScreenshot = `test-results/screenshots/locale-${language.code}-${viewport.name}-settings.png`;
      await page.screenshot({ fullPage: true, path: settingsScreenshot });
      await testInfo.attach(`locale-${language.code}-${viewport.name}-settings`, {
        path: settingsScreenshot,
        contentType: "image/png",
      });
    });
  }
}

async function expectNoHorizontalOverflow(page: Page) {
  const result = await page.evaluate(() => {
    const viewportWidth = document.documentElement.clientWidth;
    const rootOverflow = Math.max(
      document.documentElement.scrollWidth - viewportWidth,
      document.body.scrollWidth - viewportWidth,
    );
    const offenders: string[] = [];
    const selector = [
      "aside",
      "section",
      "header",
      "footer",
      "button",
      "input",
      "select",
      "textarea",
      "h1",
      "h2",
      "h3",
      "p",
      "span",
      "small",
      "time",
      "label",
      "dd",
      "dt",
    ].join(",");

    for (const element of Array.from(document.querySelectorAll(selector))) {
      const rect = element.getBoundingClientRect();
      const style = getComputedStyle(element);
      if (
        style.display === "none" ||
        style.visibility === "hidden" ||
        rect.width === 0 ||
        rect.height === 0 ||
        rect.bottom < 0 ||
        rect.top > window.innerHeight
      ) {
        continue;
      }

      if (rect.left < -1 || rect.right > viewportWidth + 1) {
        offenders.push(
          `${element.tagName.toLowerCase()}.${Array.from(element.classList).join(
            ".",
          )} ${Math.round(rect.left)}-${Math.round(rect.right)}`,
        );
      }
    }

    return {
      offenders: offenders.slice(0, 10),
      rootOverflow,
    };
  });

  expect(result.rootOverflow).toBeLessThanOrEqual(1);
  expect(result.offenders).toEqual([]);
}
