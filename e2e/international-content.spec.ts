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

const viewports = [
  { height: 960, name: "desktop", width: 1440 },
  { height: 860, name: "narrow", width: 390 },
] as const;

for (const viewport of viewports) {
  test(`mixed-script mailbox content fits in ${viewport.name}`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize(viewport);
    await page.addInitScript(fixedNowScript);
    await page.goto("/e2e/?fixture=international&language=pt-PT");

    const message = page.getByRole("button", {
      name: /Relatorio acao|Relatório ação/,
    });
    await expect(message).toBeVisible();
    await expect(page.getByText("João da Silva / 山田 太郎")).toBeVisible();
    await expect(page.locator("time", { hasText: "Hoje, 07:05" })).toBeVisible();
    await message.click();

    await expect(
      page
        .getByRole("region", { name: /Mensagem selecionada/ })
        .getByRole("heading", { name: /Relatório ação 日本語 العربية/ }),
    ).toBeVisible();
    await expect(page.getByText("relatorio-acao-日本語.txt")).toBeVisible();
    await expect(
      page.getByText(/Olá Marta.*日本語.*العربية.*עברית/s),
    ).toBeVisible();
    await expectNoHorizontalOverflow(page);

    const screenshot = `test-results/screenshots/international-${viewport.name}.png`;
    await mkdir("test-results/screenshots", { recursive: true });
    await page.screenshot({ fullPage: true, path: screenshot });
    await testInfo.attach(`international-${viewport.name}`, {
      path: screenshot,
      contentType: "image/png",
    });
  });
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
      "li",
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
