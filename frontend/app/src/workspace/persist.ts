import { forbiddenStorageKey } from "../security";
import {
  GRID_COLUMNS,
  MIN_COL_SPAN,
  MIN_ROW_SPAN,
  type WidgetLayout,
  type WorkspaceLayout,
} from "./types";

const LAYOUT_KEY_PREFIX = "vox.layout.";
const WORKSPACE_ID_PATTERN = /^[A-Za-z0-9._-]+$/;

export function layoutStorageKey(workspaceId: string): string {
  const id = assertWorkspaceId(workspaceId);
  const key = `${LAYOUT_KEY_PREFIX}${id}`;
  if (forbiddenStorageKey(key)) {
    throw new Error("refusing illegal storage key");
  }
  return key;
}

export function persistableLayout(layout: WorkspaceLayout): WorkspaceLayout {
  const workspaceId = assertWorkspaceId(layout.workspaceId);
  const widgets = layout.widgets.map(persistableWidget);
  return Object.freeze({
    workspaceId,
    widgets: Object.freeze(widgets),
  });
}

export function parseStoredLayout(workspaceId: string, raw: string | null): WorkspaceLayout {
  const id = assertWorkspaceId(workspaceId);
  if (raw === null || raw === "") {
    return emptyLayout(id);
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw) as unknown;
  } catch {
    return emptyLayout(id);
  }
  if (!isRecord(parsed)) {
    return emptyLayout(id);
  }
  const widgetsRaw = parsed["widgets"];
  const widgets = Array.isArray(widgetsRaw)
    ? widgetsRaw.flatMap((entry) => {
        const widget = readWidget(entry);
        return widget ? [widget] : [];
      })
    : [];
  return persistableLayout({ workspaceId: id, widgets });
}

function persistableWidget(widget: WidgetLayout): WidgetLayout {
  const read = readWidget(widget);
  if (!read) {
    throw new Error("widget layout is not persistable geometry");
  }
  return read;
}

function readWidget(entry: unknown): WidgetLayout | null {
  if (!isRecord(entry)) {
    return null;
  }
  const id = entry["id"];
  const col = asInt(entry["col"]);
  const row = asInt(entry["row"]);
  const colSpan = asInt(entry["colSpan"]);
  const rowSpan = asInt(entry["rowSpan"]);
  if (typeof id !== "string" || id.length === 0) {
    return null;
  }
  if (forbiddenStorageKey(id)) {
    return null;
  }
  if (col === null || row === null || colSpan === null || rowSpan === null) {
    return null;
  }
  if (col < 0 || col > GRID_COLUMNS - 1) {
    return null;
  }
  if (row < 0 || rowSpan < MIN_ROW_SPAN) {
    return null;
  }
  if (colSpan < MIN_COL_SPAN || colSpan > GRID_COLUMNS || col + colSpan > GRID_COLUMNS) {
    return null;
  }
  return Object.freeze({ id, col, row, colSpan, rowSpan });
}

function emptyLayout(workspaceId: string): WorkspaceLayout {
  return Object.freeze({
    workspaceId,
    widgets: Object.freeze([] as WidgetLayout[]),
  });
}

function assertWorkspaceId(workspaceId: string): string {
  if (typeof workspaceId !== "string" || !WORKSPACE_ID_PATTERN.test(workspaceId)) {
    throw new Error("workspaceId must be a layout identifier");
  }
  if (forbiddenStorageKey(workspaceId) || forbiddenStorageKey(`${LAYOUT_KEY_PREFIX}${workspaceId}`)) {
    throw new Error("refusing illegal storage key");
  }
  return workspaceId;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function storedLayoutCorrupt(raw: string | null): boolean {
  if (raw === null || raw === "") return false;
  try {
    return !isRecord(JSON.parse(raw) as unknown);
  } catch {
    return true;
  }
}

function asInt(value: unknown): number | null {
  if (typeof value !== "number" || !Number.isInteger(value)) {
    return null;
  }
  return value;
}
