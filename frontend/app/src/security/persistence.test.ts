import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import {
  assertNoIndexedDbSecrets,
  assertNoSecretPersistence,
  forbiddenStorageKey,
  type PersistStorage,
} from "./persistence";

function memoryStorage(init: Record<string, string> = {}): {
  getItem: (key: string) => string | null;
  setItem: (key: string, value: string) => void;
  removeItem: (key: string) => void;
  key: (index: number) => string | null;
  readonly length: number;
} {
  const map = new Map(Object.entries(init));
  return {
    get length() {
      return map.size;
    },
    key(index: number) {
      return [...map.keys()][index] ?? null;
    },
    getItem(key: string) {
      return map.has(key) ? map.get(key)! : null;
    },
    setItem(key: string, value: string) {
      map.set(key, value);
    },
    removeItem(key: string) {
      map.delete(key);
    },
  };
}

function writeSecretKey(storage: PersistStorage, value: string): string {
  const key = "token";
  storage.setItem(key, value);
  return key;
}

function collectProductionSource(dir: string): string {
  const parts: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      parts.push(collectProductionSource(full));
      continue;
    }
    if (!/\.(?:ts|tsx)$/i.test(entry.name)) continue;
    if (/\.(?:test|spec)\./i.test(entry.name)) continue;
    parts.push(readFileSync(full, "utf8"));
  }
  return parts.join("\n");
}

describe("forbiddenStorageKey", () => {
  it("flags credential-shaped keys", () => {
    expect(forbiddenStorageKey("token")).toBe(true);
    expect(forbiddenStorageKey("api-key")).toBe(true);
    expect(forbiddenStorageKey("authorization")).toBe(true);
    expect(forbiddenStorageKey("vox.layout.trade")).toBe(false);
  });
});

describe("assertNoSecretPersistence", () => {
  it("detects a token key", () => {
    localStorage.clear();
    const key = writeSecretKey(localStorage, "t.opaque");
    expect(() => assertNoSecretPersistence(localStorage)).toThrow(
      /credential-shaped storage key forbidden: token/,
    );
    localStorage.removeItem(key);
  });

  it("detects a token key in sessionStorage", () => {
    sessionStorage.clear();
    const key = writeSecretKey(sessionStorage, "t.opaque");
    expect(() => assertNoSecretPersistence(sessionStorage)).toThrow(
      /credential-shaped storage key forbidden: token/,
    );
    sessionStorage.removeItem(key);
  });

  it("allows a layout geometry key", () => {
    localStorage.clear();
    localStorage["vox.layout.trade"] = JSON.stringify({ widgets: ["chart", "tape"] });
    expect(localStorage.key(0)).toBe("vox.layout.trade");
    expect(() => assertNoSecretPersistence(localStorage)).not.toThrow();
    expect(forbiddenStorageKey("vox.layout.trade")).toBe(false);
    localStorage.removeItem("vox.layout.trade");
  });

  it("scans values as well as keys", () => {
    const storage = memoryStorage({
      "vox.layout.trade": JSON.stringify({ authorization: "Bearer abc" }),
    });
    expect(() => assertNoSecretPersistence(storage)).toThrow(
      /credential-shaped storage value forbidden/,
    );
  });
});

describe("assertNoIndexedDbSecrets", () => {
  it("detects opening a secret-shaped IndexedDB store", () => {
    expect(() =>
      assertNoIndexedDbSecrets(`indexedDB.open("token")`),
    ).toThrow(/browser source must not use indexedDB/);
    expect(() =>
      assertNoIndexedDbSecrets(`db.createObjectStore("secret")`),
    ).toThrow(/createObjectStore secret-shaped name forbidden: secret/);
  });

  it("production src never references indexedDB", () => {
    const srcRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
    const source = collectProductionSource(srcRoot);
    expect(() => assertNoIndexedDbSecrets(source)).not.toThrow();
    expect(source).not.toMatch(/\bindexedDB\b/);
  });

  it("production src never writes credential-shaped storage keys", () => {
    const srcRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
    const source = collectProductionSource(srcRoot);
    const setItem = /(localStorage|sessionStorage)\s*\.\s*setItem\s*\(\s*(['"`])(.*?)(\2)/g;
    const indexSet = /(localStorage|sessionStorage)\s*\[\s*(['"`])(.*?)(\2)\s*\]\s*=/g;
    const dotSet = /(localStorage|sessionStorage)\s*\.\s*(token|secret|credential|password|authorization)\b\s*=/gi;
    for (const match of source.matchAll(setItem)) {
      expect(forbiddenStorageKey(match[3] ?? "")).toBe(false);
    }
    for (const match of source.matchAll(indexSet)) {
      expect(forbiddenStorageKey(match[3] ?? "")).toBe(false);
    }
    expect(dotSet.test(source)).toBe(false);
  });
});
