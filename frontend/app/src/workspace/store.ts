import { applyDrag, applyResize } from "./grid";
import {
  layoutStorageKey,
  parseStoredLayout,
  persistableLayout,
  storedLayoutCorrupt,
} from "./persist";
import type { WorkspaceLayout } from "./types";

export class LayoutStore {
  private readonly storage: Storage;

  constructor(storage: Storage) {
    this.storage = storage;
  }

  load(workspaceId: string): WorkspaceLayout {
    const key = layoutStorageKey(workspaceId);
    return parseStoredLayout(workspaceId, this.storage.getItem(key));
  }

  corrupt(workspaceId: string): boolean {
    const key = layoutStorageKey(workspaceId);
    return storedLayoutCorrupt(this.storage.getItem(key));
  }

  save(workspaceId: string, layout: WorkspaceLayout): void {
    const key = layoutStorageKey(workspaceId);
    const persisted = persistableLayout({
      workspaceId,
      widgets: layout.widgets,
    });
    this.storage.setItem(key, JSON.stringify(persisted));
  }

  move(workspaceId: string, widgetId: string, col: number, row: number): WorkspaceLayout {
    const next = applyDrag(this.load(workspaceId), widgetId, col, row);
    this.save(workspaceId, next);
    return next;
  }

  resize(
    workspaceId: string,
    widgetId: string,
    colSpan: number,
    rowSpan: number,
  ): WorkspaceLayout {
    const next = applyResize(this.load(workspaceId), widgetId, colSpan, rowSpan);
    this.save(workspaceId, next);
    return next;
  }
}
