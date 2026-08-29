export type PersistStorage = Storage | {
  getItem: (key: string) => string | null;
  setItem: (key: string, value: string) => void;
  removeItem: (key: string) => void;
  key: (index: number) => string | null;
  readonly length: number;
};

const CREDENTIAL_SHAPE = /token|secret|password|authorization|bearer|api[-_]?key/i;

export function forbiddenStorageKey(key: string): boolean {
  return CREDENTIAL_SHAPE.test(key);
}

export function assertNoSecretPersistence(storage: PersistStorage): void {
  for (let index = 0; index < storage.length; index += 1) {
    const key = storage.key(index);
    if (key === null) continue;
    if (forbiddenStorageKey(key)) {
      throw new Error(`credential-shaped storage key forbidden: ${key}`);
    }
    const value = storage.getItem(key);
    if (value !== null && CREDENTIAL_SHAPE.test(value)) {
      throw new Error(`credential-shaped storage value forbidden at key: ${key}`);
    }
  }
}

/** Foundation production source must not open IndexedDB, including secret-shaped stores. */
export function assertNoIndexedDbSecrets(source: string): void {
  const indexedDb = "indexed" + "DB";
  if (source.includes(indexedDb)) {
    throw new Error(`browser source must not use ${indexedDb}`);
  }
  const stores = source.matchAll(/createObjectStore\s*\(\s*(['"`])(.*?)\1/g);
  for (const match of stores) {
    const name = match[2];
    if (name !== undefined && forbiddenStorageKey(name)) {
      throw new Error(`createObjectStore secret-shaped name forbidden: ${name}`);
    }
  }
}
