import { describe, expect, it } from "vitest";
import { applyDrag, applyResize, columnFromClientX, snapPx } from "./grid";
import { persistableLayout } from "./persist";
import type { WorkspaceLayout } from "./types";

function layout(): WorkspaceLayout {
  return persistableLayout({
    workspaceId: "trade",
    widgets: [
      { id: "chart", col: 0, row: 0, colSpan: 7, rowSpan: 6 },
      { id: "ticket", col: 7, row: 0, colSpan: 3, rowSpan: 8 },
    ],
  });
}

describe("workspace grid math", () => {
  it("columnFromClientX maps 12 equal columns", () => {
    expect(columnFromClientX(0, 1200)).toBe(0);
    expect(columnFromClientX(99, 1200)).toBe(0);
    expect(columnFromClientX(100, 1200)).toBe(1);
    expect(columnFromClientX(1199, 1200)).toBe(11);
    expect(columnFromClientX(1200, 1200)).toBe(11);
    expect(columnFromClientX(-8, 1200)).toBe(0);
  });

  it("snapPx snaps to the 8px design-system step", () => {
    expect(snapPx(0)).toBe(0);
    expect(snapPx(3)).toBe(0);
    expect(snapPx(4)).toBe(8);
    expect(snapPx(16)).toBe(16);
    expect(snapPx(20)).toBe(24);
  });

  it("applyDrag returns a new layout and does not mutate input", () => {
    const input = layout();
    const frozenCol = input.widgets[0]?.col;
    const next = applyDrag(input, "chart", 2, 3);

    expect(next).not.toBe(input);
    expect(input.widgets[0]?.col).toBe(frozenCol);
    expect(input.widgets[0]?.row).toBe(0);
    expect(next.widgets[0]).toEqual({
      id: "chart",
      col: 2,
      row: 3,
      colSpan: 7,
      rowSpan: 6,
    });
    expect(next.widgets[1]).toEqual(input.widgets[1]);
  });

  it("applyResize returns a new layout, clamps colSpan 1..12, min 2", () => {
    const input = layout();
    const next = applyResize(input, "ticket", 1, 0);

    expect(next).not.toBe(input);
    expect(input.widgets[1]?.colSpan).toBe(3);
    expect(input.widgets[1]?.rowSpan).toBe(8);
    expect(next.widgets[1]).toEqual({
      id: "ticket",
      col: 7,
      row: 0,
      colSpan: 2,
      rowSpan: 1,
    });

    const tooWide = applyResize(input, "chart", 20, 4);
    expect(tooWide.widgets[0]?.colSpan).toBe(12);
    expect(tooWide.widgets[0]?.rowSpan).toBe(4);
  });

  it("applyDrag keeps the widget inside the 12-column grid", () => {
    const input = layout();
    const next = applyDrag(input, "chart", 11, -4);
    expect(next.widgets[0]?.col).toBe(5);
    expect(next.widgets[0]?.row).toBe(0);
    expect((next.widgets[0]?.col ?? 0) + (next.widgets[0]?.colSpan ?? 0)).toBe(12);
  });
});
