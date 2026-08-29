import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";

const WIDTHS = [1280, 1440, 1920] as const;
const ARTIFACTS = join(dirname(fileURLToPath(import.meta.url)), "artifacts");

const SEEDED_LAYOUT = JSON.stringify({
  workspaceId: "foundation",
  widgets: [
    { id: "linked", col: 1, row: 0, colSpan: 4, rowSpan: 3 },
    { id: "pinned", col: 6, row: 0, colSpan: 6, rowSpan: 4 },
  ],
});

async function openFoundation(page: Page): Promise<void> {
  await page.addInitScript((layout) => {
    sessionStorage.setItem("vox.layout.foundation", layout);
  }, SEEDED_LAYOUT);
  await page.goto("/");
  await page.waitForSelector(".vox-shell");
  await page.waitForSelector(".vox-workspace [data-widget-id='linked']");
}

test.describe("executable foundation shell", () => {
  test.beforeAll(() => {
    mkdirSync(ARTIFACTS, { recursive: true });
  });

  for (const width of WIDTHS) {
    test(`AppShell + grid at ${width}px: no overflow, computed placement matches seed`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await openFoundation(page);

      await expect(page.locator(".vox-shell")).toBeVisible();
      await expect(page.locator(".vox-workspace")).toBeVisible();
      await expect
        .poll(async () =>
          page.locator(".vox-workspace").evaluate((node) => getComputedStyle(node).display),
        )
        .toBe("grid");

      const overflow = await page.evaluate(() => {
        const root = document.documentElement;
        return root.scrollWidth - root.clientWidth;
      });
      expect(overflow).toBeLessThanOrEqual(1);

      const linked = page.locator("[data-widget-id='linked']");
      const placement = await linked.evaluate((node) => {
        const cs = getComputedStyle(node);
        return {
          colStart: cs.gridColumnStart,
          colEnd: cs.gridColumnEnd,
          rowStart: cs.gridRowStart,
          rowEnd: cs.gridRowEnd,
          tokenCol: node.style.getPropertyValue("--vox-grid-col-start"),
          tokenSpan: node.style.getPropertyValue("--vox-grid-col-span"),
          tokenRow: node.style.getPropertyValue("--vox-grid-row-start"),
          tokenRowSpan: node.style.getPropertyValue("--vox-grid-row-span"),
        };
      });
      expect(placement.tokenCol).toBe("2");
      expect(placement.tokenSpan).toBe("4");
      expect(placement.tokenRow).toBe("1");
      expect(placement.tokenRowSpan).toBe("3");
      expect(placement.colStart).toBe("2");
      expect(placement.rowStart).toBe("1");
      expect(placement.colEnd === "span 4" || placement.colEnd === "6").toBe(true);
      expect(placement.rowEnd === "span 3" || placement.rowEnd === "4").toBe(true);

      await page.screenshot({
        path: join(ARTIFACTS, `foundation-${width}.png`),
        fullPage: true,
      });
    });
  }
});
