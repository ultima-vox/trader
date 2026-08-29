import { describe, expect, it } from "vitest";
import { forbiddenStorageKey } from "../security";
import { createWorkspaceGrid } from "../ui/workspace-grid";
import { persistableLayout, layoutStorageKey, parseStoredLayout, storedLayoutCorrupt } from "./persist";
import { LayoutStore } from "./store";
import type { WidgetLayout, WorkspaceLayout } from "./types";

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

function widget(overrides: Partial<WidgetLayout> & Pick<WidgetLayout, "id">): WidgetLayout {
  return {
    col: 0,
    row: 0,
    colSpan: 7,
    rowSpan: 6,
    ...overrides,
  };
}

describe("LayoutStore persistence", () => {
  it("save then load from a new LayoutStore on the same Storage is equal", () => {
    const storage = memoryStorage();
    const layout: WorkspaceLayout = persistableLayout({
      workspaceId: "trade",
      widgets: [
        widget({ id: "chart", col: 0, row: 0, colSpan: 7, rowSpan: 6 }),
        widget({ id: "ticket", col: 7, row: 0, colSpan: 3, rowSpan: 8 }),
        widget({ id: "book", col: 10, row: 0, colSpan: 2, rowSpan: 4 }),
      ],
    });

    const writer = new LayoutStore(storage);
    writer.save("trade", layout);

    expect(storage.key(0)).toBe("vox.layout.trade");
    expect(storage.length).toBe(1);
    expect(forbiddenStorageKey("vox.layout.trade")).toBe(false);

    const reader = new LayoutStore(storage);
    expect(reader.load("trade")).toEqual(persistableLayout(layout));
    expect(reader.load("trade")).toEqual(layout);
  });

  it("persistableLayout keeps geometry only and drops extra fields", () => {
    const dirty = {
      workspaceId: "scalp",
      token: "must-not-persist",
      widgets: [
        {
          id: "tape",
          col: 0,
          row: 1,
          colSpan: 4,
          rowSpan: 3,
          credential: "t.secret",
        },
      ],
    };

    const clean = persistableLayout(dirty as unknown as WorkspaceLayout);
    expect(clean).toEqual({
      workspaceId: "scalp",
      widgets: [{ id: "tape", col: 0, row: 1, colSpan: 4, rowSpan: 3 }],
    });
    expect("token" in clean).toBe(false);
    expect("credential" in (clean.widgets[0] as object)).toBe(false);

    const storage = memoryStorage();
    new LayoutStore(storage).save("scalp", dirty as unknown as WorkspaceLayout);
    expect(JSON.parse(storage.getItem("vox.layout.scalp") as string).token).toBeUndefined();
    expect(JSON.parse(storage.getItem("vox.layout.scalp") as string)).toEqual(
      persistableLayout(clean),
    );
  });

  it("move and resize persist through a fresh store", () => {
    const storage = memoryStorage();
    const store = new LayoutStore(storage);
    store.save("trade", {
      workspaceId: "trade",
      widgets: [widget({ id: "chart", col: 0, row: 0, colSpan: 7, rowSpan: 6 })],
    });

    store.move("trade", "chart", 2, 1);
    store.resize("trade", "chart", 4, 5);

    const reloaded = new LayoutStore(storage).load("trade");
    expect(reloaded).toEqual(
      persistableLayout({
        workspaceId: "trade",
        widgets: [widget({ id: "chart", col: 2, row: 1, colSpan: 4, rowSpan: 5 })],
      }),
    );
  });

  it("unknown workspace loads empty geometry under that id", () => {
    const store = new LayoutStore(memoryStorage());
    expect(store.load("research")).toEqual(
      persistableLayout({
        workspaceId: "research",
        widgets: [],
      }),
    );
  });

  it("layoutStorageKey is vox.layout.${workspaceId} only and refuses illegal keys", () => {
    expect(layoutStorageKey("trade")).toBe("vox.layout.trade");
    expect(forbiddenStorageKey(layoutStorageKey("trade"))).toBe(false);
    expect(() => layoutStorageKey("token")).toThrow(/illegal storage key/);
    expect(() => layoutStorageKey("api-key")).toThrow(/illegal storage key/);
    expect(() => layoutStorageKey("")).toThrow();
    expect(() => layoutStorageKey("trade/secret")).toThrow();
  });

  it("parseStoredLayout drops corrupt JSON and overflow geometry", () => {
    expect(parseStoredLayout("trade", "{not json")).toEqual({
      workspaceId: "trade",
      widgets: [],
    });
    expect(storedLayoutCorrupt("{not json")).toBe(true);
    expect(parseStoredLayout("trade", "[]")).toEqual({
      workspaceId: "trade",
      widgets: [],
    });
    expect(storedLayoutCorrupt("[]")).toBe(true);
    const overflow = JSON.stringify({
      widgets: [
        { id: "chart", col: 11, row: 0, colSpan: 4, rowSpan: 4 },
        { id: "tape", col: 0, row: 0, colSpan: 2, rowSpan: 3 },
      ],
    });
    expect(parseStoredLayout("trade", overflow)).toEqual({
      workspaceId: "trade",
      widgets: [{ id: "tape", col: 0, row: 0, colSpan: 2, rowSpan: 3 }],
    });
    expect(JSON.stringify(parseStoredLayout("trade", overflow))).not.toMatch(/token|secret|password/i);
  });

  it("refuses a credential-shaped widget id", () => {
    expect(() =>
      persistableLayout({
        workspaceId: "trade",
        widgets: [widget({ id: "token", col: 0, row: 0, colSpan: 2, rowSpan: 2 })],
      }),
    ).toThrow(/not persistable geometry/);
    expect(
      parseStoredLayout(
        "trade",
        JSON.stringify({ widgets: [{ id: "token", col: 0, row: 0, colSpan: 2, rowSpan: 2 }] }),
      ).widgets,
    ).toEqual([]);
  });

  it("does not treat missing storage as corrupt", () => {
    const storage = memoryStorage();
    const store = new LayoutStore(storage);
    expect(store.corrupt("trade")).toBe(false);
    storage.setItem("vox.layout.trade", "{broken");
    expect(store.corrupt("trade")).toBe(true);
    expect(store.load("trade").widgets).toEqual([]);
  });

  it("drops colSpan below MIN_COL_SPAN", () => {
    expect(
      parseStoredLayout(
        "trade",
        JSON.stringify({ widgets: [{ id: "chart", col: 0, row: 0, colSpan: 1, rowSpan: 2 }] }),
      ).widgets,
    ).toEqual([]);
  });

  it("grid seed does not overwrite corrupt storage", () => {
    const storage = memoryStorage();
    storage.setItem("vox.layout.trade", "{broken");
    const store = new LayoutStore(storage);
    expect(store.corrupt("trade")).toBe(true);
    const raw = storage.getItem("vox.layout.trade");
    createWorkspaceGrid({
      workspaceId: "trade",
      layoutStore: store,
      items: [{ id: "chart", colSpan: 6, element: document.createElement("div") }],
    });
    expect(storage.getItem("vox.layout.trade")).toBe(raw);
    expect(store.corrupt("trade")).toBe(true);
  });
});
