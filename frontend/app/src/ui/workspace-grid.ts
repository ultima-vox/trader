import {
  columnFromClientX,
  LayoutStore,
  persistableLayout,
  RESIZE_STEP_PX,
  type WidgetLayout,
  type WorkspaceLayout,
} from "../workspace";
import { append, el } from "./dom";

const COL_SPANS = [2, 3, 4, 5, 6, 7, 8, 12] as const;
export type GridColSpan = (typeof COL_SPANS)[number];

/** CSS Grid is 1-indexed; stored WidgetLayout.col/row are 0-based. */
export const GRID_COL_START = "--vox-grid-col-start";
export const GRID_ROW_START = "--vox-grid-row-start";
export const GRID_COL_SPAN = "--vox-grid-col-span";
export const GRID_ROW_SPAN = "--vox-grid-row-span";

export type WorkspaceGridItem = {
  id: string;
  colSpan: GridColSpan;
  element: HTMLElement;
  col?: number;
  row?: number;
  rowSpan?: number;
};

export type WorkspaceGridOptions = {
  workspaceId: string;
  layoutStore: LayoutStore;
  items: WorkspaceGridItem[];
};

export function createWorkspaceGrid(options: WorkspaceGridOptions): HTMLElement {
  const grid = el("div", "vox-workspace");
  const byId = new Map(options.items.map((item) => [item.id, item]));
  let layout = seedLayout(options);

  const paint = (): void => {
    const ordered = [...layout.widgets].sort((a, b) => a.row - b.row || a.col - b.col);
    for (const widget of ordered) {
      const item = byId.get(widget.id);
      if (item === undefined) continue;
      applyGridPlacement(item.element, widget);
      append(grid, item.element);
    }
  };

  paint();

  for (const item of options.items) {
    bindDrag(grid, item.element, (next) => {
      layout = next;
      paint();
    }, options);
    bindResize(grid, item.element, () => layout, (next) => {
      layout = next;
      paint();
    }, options);
  }
  return grid;
}

function seedLayout(options: WorkspaceGridOptions): WorkspaceLayout {
  const corrupt = options.layoutStore.corrupt(options.workspaceId);
  const loaded = options.layoutStore.load(options.workspaceId);
  const byId = new Map(loaded.widgets.map((widget) => [widget.id, widget]));
  let col = 0;
  let row = 0;
  const widgets: WidgetLayout[] = [];
  for (const item of options.items) {
    const existing = byId.get(item.id);
    if (existing !== undefined) {
      widgets.push(existing);
      continue;
    }
    const span = item.colSpan;
    if (col + span > 12) {
      col = 0;
      row += 1;
    }
    widgets.push({
      id: item.id,
      col: item.col ?? col,
      row: item.row ?? row,
      colSpan: span,
      rowSpan: item.rowSpan ?? 4,
    });
    col += span;
  }
  const layout = persistableLayout({ workspaceId: options.workspaceId, widgets });
  if (!corrupt) {
    options.layoutStore.save(options.workspaceId, layout);
  }
  return layout;
}

function nearestColSpan(value: number): GridColSpan {
  let best: GridColSpan = 2;
  for (const span of COL_SPANS) {
    if (Math.abs(span - value) < Math.abs(best - value)) best = span;
  }
  return best;
}

export function applyGridPlacement(element: HTMLElement, widget: WidgetLayout): void {
  const colStart = widget.col + 1;
  const rowStart = widget.row + 1;
  element.style.setProperty(GRID_COL_START, String(colStart));
  element.style.setProperty(GRID_ROW_START, String(rowStart));
  element.style.setProperty(GRID_COL_SPAN, String(widget.colSpan));
  element.style.setProperty(GRID_ROW_SPAN, String(widget.rowSpan));
  element.dataset.widgetId = widget.id;
  element.dataset.col = String(widget.col);
  element.dataset.row = String(widget.row);
  element.dataset.colSpan = String(widget.colSpan);
  element.dataset.rowSpan = String(widget.rowSpan);
  element.style.minHeight = `${Math.max(1, widget.rowSpan) * 48}px`;
  applyColSpan(element, nearestColSpan(widget.colSpan));
}

export function readGridPlacement(element: HTMLElement): {
  col: number;
  row: number;
  colSpan: number;
  rowSpan: number;
} {
  return {
    col: Number(element.style.getPropertyValue(GRID_COL_START)) - 1,
    row: Number(element.style.getPropertyValue(GRID_ROW_START)) - 1,
    colSpan: Number(element.style.getPropertyValue(GRID_COL_SPAN)),
    rowSpan: Number(element.style.getPropertyValue(GRID_ROW_SPAN)),
  };
}

function applyColSpan(element: HTMLElement, span: GridColSpan): void {
  Array.from(element.classList).forEach((token) => {
    if (token.startsWith("vox-workspace__col-")) element.classList.remove(token);
  });
  element.classList.add(`vox-workspace__col-${span}`);
}

function bindDrag(
  grid: HTMLElement,
  widget: HTMLElement,
  commit: (layout: WorkspaceLayout) => void,
  options: WorkspaceGridOptions,
): void {
  const header = widget.querySelector(".vox-widget__header");
  if (!(header instanceof HTMLElement)) return;
  if (widget.classList.contains("is-pinned")) return;

  header.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    if ((event.target as HTMLElement | null)?.closest(".vox-widget__resize")) return;
    event.preventDefault();
    const preview = el("div", "vox-drop-target");
    preview.style.minHeight = `${widget.getBoundingClientRect().height}px`;
    grid.classList.add("is-dragging");
    header.setPointerCapture(event.pointerId);

    const onMove = (move: PointerEvent): void => {
      const over = widgetAt(grid, move.clientX, move.clientY, widget);
      if (over === null) return;
      const rect = over.getBoundingClientRect();
      const before = move.clientX < rect.left + rect.width / 2;
      if (before) grid.insertBefore(preview, over);
      else grid.insertBefore(preview, over.nextSibling);
    };

    const onUp = (up: PointerEvent): void => {
      header.removeEventListener("pointermove", onMove);
      header.removeEventListener("pointerup", onUp);
      if (preview.parentElement === grid) grid.insertBefore(widget, preview);
      preview.remove();
      grid.classList.remove("is-dragging");
      const id = widget.dataset.widgetId;
      if (id === undefined) return;
      const rect = grid.getBoundingClientRect();
      const col = columnFromClientX(up.clientX - rect.left, rect.width);
      const row = Math.max(0, Math.floor((up.clientY - rect.top) / 48));
      commit(options.layoutStore.move(options.workspaceId, id, col, row));
    };

    header.addEventListener("pointermove", onMove);
    header.addEventListener("pointerup", onUp);
  });
}

function bindResize(
  grid: HTMLElement,
  widget: HTMLElement,
  current: () => WorkspaceLayout,
  commit: (layout: WorkspaceLayout) => void,
  options: WorkspaceGridOptions,
): void {
  const handle = widget.querySelector(".vox-widget__resize");
  if (!(handle instanceof HTMLElement)) return;

  handle.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    const startX = event.clientX;
    const startY = event.clientY;
    const startWidth = widget.getBoundingClientRect().width;
    const startHeight = widget.getBoundingClientRect().height;
    const colWidth = grid.getBoundingClientRect().width / 12;
    const layout = current();
    const id = widget.dataset.widgetId;
    const existing = layout.widgets.find((row) => row.id === id);
    handle.setPointerCapture(event.pointerId);

    const onMove = (move: PointerEvent): void => {
      if (id === undefined || existing === undefined) return;
      const dx = move.clientX - startX;
      const dy = move.clientY - startY;
      const snappedWidth = Math.round((startWidth + dx) / RESIZE_STEP_PX) * RESIZE_STEP_PX;
      const snappedHeight = Math.max(48, Math.round((startHeight + dy) / RESIZE_STEP_PX) * RESIZE_STEP_PX);
      const span = nearestColSpan(Math.max(2, snappedWidth / Math.max(colWidth, 1)));
      const rowSpan = Math.max(1, Math.round(snappedHeight / 48));
      applyGridPlacement(widget, {
        id,
        col: existing.col,
        row: existing.row,
        colSpan: span,
        rowSpan,
      });
    };

    const onUp = (): void => {
      handle.removeEventListener("pointermove", onMove);
      handle.removeEventListener("pointerup", onUp);
      if (id === undefined || existing === undefined) return;
      const previewed = readGridPlacement(widget);
      commit(
        options.layoutStore.resize(
          options.workspaceId,
          id,
          previewed.colSpan,
          previewed.rowSpan,
        ),
      );
    };

    handle.addEventListener("pointermove", onMove);
    handle.addEventListener("pointerup", onUp);
  });
}

function widgetAt(
  grid: HTMLElement,
  x: number,
  y: number,
  skip: HTMLElement,
): HTMLElement | null {
  for (const node of Array.from(grid.children)) {
    if (!(node instanceof HTMLElement) || node === skip) continue;
    if (node.classList.contains("vox-drop-target")) continue;
    const rect = node.getBoundingClientRect();
    if (x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom) return node;
  }
  return null;
}
