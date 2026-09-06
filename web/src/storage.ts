// Mail and drafts stay on this browser. Passwords and OAuth grants never enter
// this database. A committed transaction is required before any SMTP request.
export const stores = [
  "accounts",
  "mail",
  "raw",
  "drafts",
  "outgoing",
  "draftFiles",
] as const;
export type StoreName = (typeof stores)[number];
export interface Change {
  store: StoreName;
  key: string;
  value?: unknown;
}
export interface LocalStore {
  all<T>(store: StoreName): Promise<T[]>;
  get<T>(store: StoreName, key: string): Promise<T | undefined>;
  commit(changes: Change[]): Promise<void>;
}
export class BrowserStore implements LocalStore {
  private constructor(private db: IDBDatabase) {}
  static async open(user: string): Promise<BrowserStore> {
    if (!/^[A-Za-z0-9_-]{43}$/.test(user))
      throw new Error("Invalid browser profile identity.");
    return new Promise((resolve, reject) => {
      const request = indexedDB.open(`shep.mail.v1.${user}`, 2);
      request.onupgradeneeded = () => {
        for (const store of stores)
          if (!request.result.objectStoreNames.contains(store))
            request.result.createObjectStore(store);
      };
      request.onerror = () =>
        reject(
          new Error(
            "Browser storage is unavailable. Allow site storage and reopen Shep.",
          ),
        );
      request.onblocked = () =>
        reject(new Error("Close other Shep tabs to update browser storage."));
      request.onsuccess = () => {
        request.result.onversionchange = () => request.result.close();
        resolve(new BrowserStore(request.result));
      };
    });
  }
  private read<T>(store: StoreName, key?: string): Promise<T> {
    return new Promise((resolve, reject) => {
      const tx = this.db.transaction(store, "readonly");
      const request =
        key === undefined
          ? tx.objectStore(store).getAll()
          : tx.objectStore(store).get(key);
      tx.oncomplete = () => resolve(request.result as T);
      tx.onabort = () =>
        reject(
          new Error("Could not read browser storage. Reopen Shep to retry."),
        );
    });
  }
  all<T>(store: StoreName) {
    return this.read<T[]>(store);
  }
  get<T>(store: StoreName, key: string) {
    return this.read<T | undefined>(store, key);
  }
  commit(changes: Change[]): Promise<void> {
    if (!changes.length) return Promise.resolve();
    return new Promise((resolve, reject) => {
      const tx = this.db.transaction(
        [...new Set(changes.map((c) => c.store))],
        "readwrite",
        { durability: "strict" },
      );
      tx.oncomplete = () => resolve();
      tx.onabort = () =>
        reject(
          new Error(
            "Could not save on this browser. Free storage space and retry; keep the draft open.",
          ),
        );
      for (const c of changes) {
        if (c.value === undefined) tx.objectStore(c.store).delete(c.key);
        else tx.objectStore(c.store).put(c.value, c.key);
      }
    });
  }
}
