import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { LayoutStore } from "../workspace";
import { el } from "./dom";
import {
  createWorkspaceGrid,
  GRID_COL_SPAN,
  GRID_COL_START,
  GRID_ROW_SPAN,
  GRID_ROW_START,
  readGridPlacement,
} from "./workspace-grid";

function memoryStorage(): Storage {
  const data = new Map<string, string>();
  return {
    get length() {
      return data.size;
    },
    clear() {
      data.clear();
    },
    getItem(key: string) {
      return data.has(key) ? (data.get(key) as string) : null;
    },
    key(index: number) {
      return [...data.keys()][index] ?? null;
    },
    removeItem(key: string) {
      data.delete(key);
    },
    setItem(key: string, value: string) {
      data.set(key, String(value));
    },
  };
}

function widgetEl(): HTMLElement {
  const root = el("article", "vox-widget");
  root.append(el("div", "vox-widget__header"));
  root.append(el("span", "vox-widget__resize"));
  return root;
}

function stubRect(
  element: HTMLElement,
  box: { x: number; y: number; width: number; height: number },
): void {
  element.getBoundingClientRect = () =>
    ({
      x: box.x,
      y: box.y,
      left: box.x,
      top: box.y,
      right: box.x + box.width,
      bottom: box.y + box.height,
      width: box.width,
      height: box.height,
      toJSON() {
        return this;
      },
    }) as DOMRect;
}

describe("workspace grid placement", () => {
  it("paints persisted col/row/colSpan/rowSpan onto CSS custom properties", () => {
    const storage = memoryStorage();
    const store = new LayoutStore(storage);
    store.save("trade", {
      workspaceId: "trade",
      widgets: [{ id: "chart", col: 3, row: 1, colSpan: 4, rowSpan: 2 }],
    });
    const chart = widgetEl();
    createWorkspaceGrid({
      workspaceId: "trade",
      layoutStore: store,
      items: [{ id: "chart", colSpan: 6, element: chart }],
    });
    expect(readGridPlacement(chart)).toEqual({ col: 3, row: 1, colSpan: 4, rowSpan: 2 });
    expect(chart.style.getPropertyValue(GRID_COL_START)).toBe("4");
    expect(chart.style.getPropertyValue(GRID_ROW_START)).toBe("2");
    expect(chart.style.getPropertyValue(GRID_COL_SPAN)).toBe("4");
    expect(chart.style.getPropertyValue(GRID_ROW_SPAN)).toBe("2");
    expect(chart.dataset.col).toBe("3");
    expect(chart.dataset.row).toBe("1");
    expect(chart.classList.contains("vox-workspace__col-4")).toBe(true);
  });

  it("reload restores exact col,row,colSpan,rowSpan on the DOM", () => {
    const storage = memoryStorage();
    const first = new LayoutStore(storage);
    const chart = widgetEl();
    createWorkspaceGrid({
      workspaceId: "trade",
      layoutStore: first,
      items: [{ id: "chart", colSpan: 5, element: chart, col: 2, row: 3, rowSpan: 6 }],
    });
    expect(readGridPlacement(chart)).toEqual({ col: 2, row: 3, colSpan: 5, rowSpan: 6 });

    const reloaded = widgetEl();
    createWorkspaceGrid({
      workspaceId: "trade",
      layoutStore: new LayoutStore(storage),
      items: [{ id: "chart", colSpan: 12, element: reloaded, col: 0, row: 0, rowSpan: 1 }],
    });
    expect(readGridPlacement(reloaded)).toEqual({ col: 2, row: 3, colSpan: 5, rowSpan: 6 });
  });

  it("placement tokens stay exact at 1280 / 1440 / 1920 container widths", () => {
    const storage = memoryStorage();
    const chart = widgetEl();
    const grid = createWorkspaceGrid({
      workspaceId: "trade",
      layoutStore: new LayoutStore(storage),
      items: [{ id: "chart", colSpan: 7, element: chart, col: 1, row: 2, rowSpan: 3 }],
    });
    for (const width of [1280, 1440, 1920]) {
      grid.style.width = `${width}px`;
      expect(readGridPlacement(chart)).toEqual({ col: 1, row: 2, colSpan: 7, rowSpan: 3 });
    }
  });

  it("resize from span 4 to 6 updates the span property, persists, and reload restores 6", () => {
    const storage = memoryStorage();
    const store = new LayoutStore(storage);
    store.save("trade", {
      workspaceId: "trade",
      widgets: [{ id: "chart", col: 0, row: 0, colSpan: 4, rowSpan: 2 }],
    });
    const chart = widgetEl();
    const grid = createWorkspaceGrid({
      workspaceId: "trade",
      layoutStore: store,
      items: [{ id: "chart", colSpan: 4, element: chart }],
    });
    document.body.append(grid);
    stubRect(grid, { x: 0, y: 0, width: 1200, height: 400 });
    stubRect(chart, { x: 0, y: 0, width: 400, height: 96 });
    const handle = chart.querySelector(".vox-widget__resize");
    expect(handle).toBeInstanceOf(HTMLElement);
    if (!(handle instanceof HTMLElement)) return;
    handle.setPointerCapture = () => undefined;
    handle.releasePointerCapture = () => undefined;
    handle.dispatchEvent(
      new PointerEvent("pointerdown", { button: 0, clientX: 400, clientY: 96, bubbles: true }),
    );
    handle.dispatchEvent(
      new PointerEvent("pointermove", { button: 0, clientX: 600, clientY: 144, bubbles: true }),
    );
    expect(readGridPlacement(chart)).toEqual({ col: 0, row: 0, colSpan: 6, rowSpan: 3 });
    expect(chart.style.getPropertyValue(GRID_COL_SPAN)).toBe("6");
    expect(chart.style.getPropertyValue(GRID_ROW_SPAN)).toBe("3");
    handle.dispatchEvent(
      new PointerEvent("pointerup", { button: 0, clientX: 600, clientY: 144, bubbles: true }),
    );
    expect(store.load("trade").widgets[0]).toMatchObject({ colSpan: 6, rowSpan: 3 });

    const reloaded = widgetEl();
    createWorkspaceGrid({
      workspaceId: "trade",
      layoutStore: new LayoutStore(storage),
      items: [{ id: "chart", colSpan: 4, element: reloaded }],
    });
    expect(readGridPlacement(reloaded)).toEqual({ col: 0, row: 0, colSpan: 6, rowSpan: 3 });
    grid.remove();
  });

  it("design-system CSS binds start/span custom properties, not DOM order", () => {
    const cssPath = join(
      dirname(fileURLToPath(import.meta.url)),
      "../../../design-system/patterns/patterns.css",
    );
    const css = readFileSync(cssPath, "utf8");
    expect(css).toContain("grid-column: var(--vox-grid-col-start, auto) / span var(--vox-grid-col-span, 1);");
    expect(css).toContain("grid-row: var(--vox-grid-row-start, auto) / span var(--vox-grid-row-span, 1);");
    expect(css).not.toMatch(/\.vox-workspace__col-6\s*\{\s*grid-column:\s*span 6/);
  });
});
