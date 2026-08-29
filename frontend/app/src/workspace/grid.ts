import {
  GRID_COLUMNS,
  MIN_COL_SPAN,
  MIN_ROW_SPAN,
  RESIZE_STEP_PX,
  type WidgetLayout,
  type WorkspaceLayout,
} from "./types";
import { persistableLayout } from "./persist";

export { GRID_COLUMNS, MIN_COL_SPAN, MIN_ROW_SPAN, RESIZE_STEP_PX };

export function columnFromClientX(clientX: number, gridRectWidth: number): number {
  if (!(gridRectWidth > 0) || !Number.isFinite(clientX) || !Number.isFinite(gridRectWidth)) {
    return 0;
  }
  const col = Math.floor((clientX / gridRectWidth) * GRID_COLUMNS);
  return clampInt(col, 0, GRID_COLUMNS - 1);
}

export function snapPx(px: number): number {
  if (!Number.isFinite(px)) {
    return 0;
  }
  return Math.round(px / RESIZE_STEP_PX) * RESIZE_STEP_PX;
}

export function applyDrag(
  layout: WorkspaceLayout,
  widgetId: string,
  col: number,
  row: number,
): WorkspaceLayout {
  return mapWidget(layout, widgetId, (widget) => {
    const nextCol = clampCol(col, widget.colSpan);
    const nextRow = clampRow(row);
    return freezeWidget({ ...widget, col: nextCol, row: nextRow });
  });
}

export function applyResize(
  layout: WorkspaceLayout,
  widgetId: string,
  colSpan: number,
  rowSpan: number,
): WorkspaceLayout {
  return mapWidget(layout, widgetId, (widget) => {
    const nextColSpan = clampColSpan(colSpan, widget.col);
    const nextRowSpan = clampRowSpan(rowSpan);
    return freezeWidget({ ...widget, colSpan: nextColSpan, rowSpan: nextRowSpan });
  });
}

function mapWidget(
  layout: WorkspaceLayout,
  widgetId: string,
  update: (widget: WidgetLayout) => WidgetLayout,
): WorkspaceLayout {
  const index = layout.widgets.findIndex((widget) => widget.id === widgetId);
  if (index < 0) {
    throw new Error(`unknown widget: ${widgetId}`);
  }
  const widgets = layout.widgets.map((widget, i) => (i === index ? update(widget) : freezeWidget(widget)));
  return persistableLayout({ workspaceId: layout.workspaceId, widgets });
}

function freezeWidget(widget: WidgetLayout): WidgetLayout {
  return Object.freeze({
    id: widget.id,
    col: widget.col,
    row: widget.row,
    colSpan: widget.colSpan,
    rowSpan: widget.rowSpan,
  });
}

function clampCol(col: number, colSpan: number): number {
  const span = clampColSpan(colSpan, 0);
  return clampInt(Math.round(col), 0, GRID_COLUMNS - span);
}

function clampColSpan(span: number, col: number): number {
  const origin = clampInt(Math.round(col), 0, GRID_COLUMNS - 1);
  const remaining = GRID_COLUMNS - origin;
  const floor = remaining >= MIN_COL_SPAN ? MIN_COL_SPAN : 1;
  return clampInt(Math.round(span), floor, remaining);
}

function clampRow(row: number): number {
  return Math.max(0, Math.round(row));
}

function clampRowSpan(span: number): number {
  return Math.max(MIN_ROW_SPAN, Math.round(span));
}

function clampInt(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) {
    return min;
  }
  return Math.min(max, Math.max(min, Math.trunc(value)));
}
