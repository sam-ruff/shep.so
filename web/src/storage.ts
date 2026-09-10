import { prepareLineage, type IdentityTransition } from "./mail_lineage";
import { cacheStores, mailMetadata, recordCacheChanges } from "./cache_changes";
import {
  BrowserIntents,
  adoptIntentAliases,
  acknowledgeIntentCache,
  type IntentStore,
  type IntentLease,
} from "./mail_intents";
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
  "mailIntents",
  "intentState",
  ...cacheStores,
] as const;
export type StoreName = (typeof stores)[number];
export interface Change {
  store: StoreName;
  key: string;
  value?: unknown;
  identity?: IdentityTransition;
}
export interface LocalStore {
  readonly profileId?: string;
  intents?: IntentStore;
  removeAccount?(review: RemovalReview, discard: boolean): Promise<void>;
  all<T>(store: StoreName): Promise<T[]>;
  get<T>(store: StoreName, key: string): Promise<T | undefined>;
  commit(changes: Change[], intent?: IntentLease): Promise<void>;
  snapshot(
    names: readonly StoreName[],
  ): Promise<Partial<Record<StoreName, unknown[]>>>;
  submissions<T>(id: string): Promise<T[]>;
}
export async function openMailDatabase(user: string): Promise<IDBDatabase> {
  if (!/^[A-Za-z0-9_-]{43}$/.test(user))
    throw new Error("Invalid browser profile identity.");
  return new Promise((resolve, reject) => {
    let abandoned = false;
    // Version 12 tracks acknowledged physical identity continuity in metadata.
    // Version 11 fences writers that omit intent-only query invalidation.
    // Version 10 indexes account/physical identities without a JS body migration.
    // Version 9 binds the derived persistent index to this source incarnation.
    // Version 8 fences older tabs that remove accounts without group ownership.
    // Version 7 fenced writes lacking atomic cache-applied intent revisions.
    const request = indexedDB.open(`shep.mail.v1.${user}`, 12);
    request.onupgradeneeded = (event) => {
      for (const store of stores)
        if (!request.result.objectStoreNames.contains(store))
          request.result.createObjectStore(store);
      // Seed acknowledged folder roles from the earlier outgoing journal in
      // the same upgrade transaction; failure leaves version 2 intact.
      const tx = request.transaction!;
      if (event.oldVersion < 10) {
        const mail = tx.objectStore("mail");
        mail.createIndex("account", "core.account_id");
        mail.createIndex("accountFolder", ["core.account_id", "core.folder"]);
        mail.createIndex("serverIdentity", ["core.account_id", "core.id"]);
      }
      if (event.oldVersion < 5) {
        const metadata = tx.objectStore("mailMetadata");
        metadata.createIndex("newest", "newest");
        metadata.createIndex("oldest", "oldest");
        const cursor = tx.objectStore("mail").openCursor();
        cursor.onsuccess = () => {
          const row = cursor.result;
          if (!row) return;
          const value = mailMetadata(row.value);
          if (value) metadata.put(value, row.primaryKey);
          row.continue();
        };
        tx.objectStore("cacheState").put({ revision: 0, floor: 0 }, "mail");
      }
      if (event.oldVersion < 9) {
        const state = tx.objectStore("cacheState").get("mail");
        state.onsuccess = () =>
          tx.objectStore("cacheState").put(
            {
              ...(state.result ?? { revision: 0, floor: 0 }),
              epoch: crypto.randomUUID(),
            },
            "mail",
          );
      }
      if (event.oldVersion >= 5 && event.oldVersion < 12) {
        const cursor = tx.objectStore("mailMetadata").openCursor();
        cursor.onsuccess = () => {
          const row = cursor.result;
          if (!row) return;
          row.update({ ...row.value, lineage: crypto.randomUUID() });
          row.continue();
        };
      }
      const outgoing = tx.objectStore("outgoing");
      if (!outgoing.indexNames.contains("submission"))
        outgoing.createIndex("submission", "id");
      // Never replace roles acknowledged after the original v3 migration.
      if (event.oldVersion >= 3) return;
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
    request.onblocked = () => {
      abandoned = true;
      reject(new Error("Close other Shep tabs to update browser storage."));
    };
    request.onsuccess = () => {
      if (abandoned) {
        request.result.close();
        return;
      }
      request.result.onversionchange = () => request.result.close();
      resolve(request.result);
    };
  });
}

export class BrowserStore implements LocalStore {
  readonly intents: IntentStore;
  private constructor(
    private db: IDBDatabase,
    readonly profileId: string,
  ) {
    this.intents = new BrowserIntents(db);
  }
  static async open(user: string): Promise<BrowserStore> {
    return new BrowserStore(await openMailDatabase(user), user);
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
            const changes = removalChanges(snapshot, review, discard);
            for (const c of changes) {
              if (c.value === undefined) tx.objectStore(c.store).delete(c.key);
              else tx.objectStore(c.store).put(c.value, c.key);
            }
            if (changes.length) recordCacheChanges(tx, changes, true);
          } catch (e) {
            error = e;
            tx.abort();
          }
        };
      }
    });
  }
  commit(changes: Change[], intent?: IntentLease): Promise<void> {
    if (!changes.length) return Promise.resolve();
    changes = changes.map((change) => ({ ...change }));
    return new Promise((resolve, reject) => {
      const tx = this.db.transaction(
        [
          ...new Set([
            ...changes.map((c) => c.store),
            ...(changes.some(
              (c) =>
                c.store === "mail" ||
                c.store === "mailAliases" ||
                c.store === "mailIntents",
            )
              ? cacheStores
              : []),
            ...(changes.some((c) => c.store === "mailAliases")
              ? ["mailIntents"]
              : []),
            ...(changes.some((c) => c.store === "mail") ? ["mailAliases"] : []),
            ...(intent ? ["mailIntents", "mailAliases", "mailMetadata"] : []),
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
      removed.onsuccess = async () => {
        try {
          checkRemovedWrites(changes, removed.result);
          // Keep derived origin metadata atomic with the raw cache and aliases.
          const prepared = changes.some(
            (c) => c.store === "mail" || c.store === "mailAliases",
          )
            ? await prepareLineage(tx, changes)
            : undefined;
          if (changes.some((c) => c.store === "mailAliases"))
            await adoptIntentAliases(tx, changes);
          for (const c of changes) {
            if (c.value === undefined) tx.objectStore(c.store).delete(c.key);
            else tx.objectStore(c.store).put(c.value, c.key);
          }
          if (
            changes.some(
              (c) =>
                c.store === "mail" ||
                c.store === "mailAliases" ||
                c.store === "mailIntents",
            )
          )
            recordCacheChanges(tx, changes, false, prepared);
          if (intent) await acknowledgeIntentCache(tx, intent, changes);
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
