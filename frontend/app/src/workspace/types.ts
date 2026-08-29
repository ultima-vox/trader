export type WidgetLayout = Readonly<{
  id: string;
  col: number;
  row: number;
  colSpan: number;
  rowSpan: number;
}>;

export type WorkspaceLayout = Readonly<{
  workspaceId: string;
  widgets: readonly WidgetLayout[];
}>;

export const GRID_COLUMNS = 12;
export const RESIZE_STEP_PX = 8;
export const MIN_COL_SPAN = 2;
export const MIN_ROW_SPAN = 1;
