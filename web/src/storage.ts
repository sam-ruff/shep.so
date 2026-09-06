import {
  checkRemovedWrites,
  removalChanges,
  reviewStores,
  type RemovalReview,
} from "./account_removal";
// Mail and drafts stay on this browser. Passwords and OAuth grants never enter
// this database. A committed transaction is required before any SMTP request.
export const stores = [
  "accounts",
  "mail",
  "raw",
  "drafts",
  "outgoing",
  "draftFiles",
  "mailAliases",
  "mailRoles",
  "removedAccounts",
] as const;
export type StoreName = (typeof stores)[number];
export interface Change {
  store: StoreName;
  key: string;
  value?: unknown;
}
export interface LocalStore {
  removeAccount?(review: RemovalReview, discard: boolean): Promise<void>;
  all<T>(store: StoreName): Promise<T[]>;
  get<T>(store: StoreName, key: string): Promise<T | undefined>;
  commit(changes: Change[]): Promise<void>;
  snapshot(
    names: readonly StoreName[],
  ): Promise<Partial<Record<StoreName, unknown[]>>>;
  submissions<T>(id: string): Promise<T[]>;
}
export class BrowserStore implements LocalStore {
  private constructor(private db: IDBDatabase) {}
  static async open(user: string): Promise<BrowserStore> {
    if (!/^[A-Za-z0-9_-]{43}$/.test(user))
      throw new Error("Invalid browser profile identity.");
    return new Promise((resolve, reject) => {
      const request = indexedDB.open(`shep.mail.v1.${user}`, 4);
      request.onupgradeneeded = () => {
        for (const store of stores)
          if (!request.result.objectStoreNames.contains(store))
            request.result.createObjectStore(store);
        // Seed acknowledged folder roles from the earlier outgoing journal in
        // the same upgrade transaction; failure leaves version 2 intact.
        const tx = request.transaction!;
        const outgoing = tx.objectStore("outgoing");
        if (!outgoing.indexNames.contains("submission"))
          outgoing.createIndex("submission", "id");
        const roles = new Map<string, Set<string>>();
        const cursor = tx.objectStore("outgoing").openCursor();
        cursor.onsuccess = () => {
          const row = cursor.result;
          if (row) {
            const record = row.value;
            const account = record.account?.id ?? record.draft?.accountId;
            const folder = record.sent?.receipt?.folder;
            if (
              record.sent?.state === "saved" &&
              typeof account === "string" &&
              typeof folder === "string"
            ) {
              const known = roles.get(account) ?? new Set<string>();
              known.add(folder);
              roles.set(account, known);
            }
            row.continue();
          } else
            for (const [account, folders] of roles)
              tx.objectStore("mailRoles").put(
                { account, acknowledged: [...folders] },
                account,
              );
        };
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
  close() {
    this.db.close();
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
  snapshot(
    names: readonly StoreName[],
  ): Promise<Partial<Record<StoreName, unknown[]>>> {
    return new Promise((resolve, reject) => {
      const tx = this.db.transaction([...names], "readonly");
      const reads = names.map(
        (name) => [name, tx.objectStore(name).getAll()] as const,
      );
      tx.oncomplete = () =>
        resolve(
          Object.fromEntries(reads.map(([name, read]) => [name, read.result])),
        );
      tx.onabort = () =>
        reject(
          new Error(
            "Could not read the mailbox snapshot. Reopen Shep to retry.",
          ),
        );
    });
  }
  submissions<T>(id: string): Promise<T[]> {
    return new Promise((resolve, reject) => {
      const tx = this.db.transaction("outgoing", "readonly");
      const request = tx.objectStore("outgoing").index("submission").getAll(id);
      tx.oncomplete = () => resolve(request.result as T[]);
      tx.onabort = () =>
        reject(
          new Error("Could not check the saved Sent identity. Retry Refresh."),
        );
    });
  }
  removeAccount(review: RemovalReview, discard: boolean): Promise<void> {
    return new Promise((resolve, reject) => {
      const tx = this.db.transaction([...stores], "readwrite", {
        durability: "strict",
      });
      let error: unknown;
      tx.oncomplete = () => resolve();
      tx.onabort = () =>
        reject(
          error ??
            new Error(
              "Could not remove this account from browser storage. Local data is unchanged; retry.",
            ),
        );
      const snapshot: Partial<Record<StoreName, any[]>> = {};
      let remaining = reviewStores.length;
      for (const name of reviewStores) {
        const request = tx.objectStore(name).getAll();
        request.onsuccess = () => {
          snapshot[name] = request.result;
          if (--remaining) return;
          try {
            for (const c of removalChanges(snapshot, review, discard)) {
              if (c.value === undefined) tx.objectStore(c.store).delete(c.key);
              else tx.objectStore(c.store).put(c.value, c.key);
            }
          } catch (e) {
            error = e;
            tx.abort();
          }
        };
      }
    });
  }
  commit(changes: Change[]): Promise<void> {
    if (!changes.length) return Promise.resolve();
    return new Promise((resolve, reject) => {
      const tx = this.db.transaction(
        [
          ...new Set([
            ...changes.map((c) => c.store),
            "removedAccounts" as const,
          ]),
        ],
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
      const removed = tx.objectStore("removedAccounts").getAll();
      let cause: unknown;
      const originalAbort = tx.onabort;
      tx.onabort = (event) =>
        cause ? reject(cause) : originalAbort?.call(tx, event);
      removed.onsuccess = () => {
        try {
          checkRemovedWrites(changes, removed.result);
          for (const c of changes) {
            if (c.value === undefined) tx.objectStore(c.store).delete(c.key);
            else tx.objectStore(c.store).put(c.value, c.key);
          }
        } catch (error) {
          cause =
            error instanceof Error &&
            error.message.startsWith("This account was removed")
              ? error
              : undefined;
          // DataCloneError and invalid keys can throw synchronously after earlier
          // requests were queued. Abort them before rejecting the whole operation.
          tx.abort();
        }
      };
    });
  }
}
