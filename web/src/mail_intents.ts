import type { Fields } from "./model";
import type { Change } from "./storage";
import {
  cacheStores,
  recordCacheChanges,
  type CacheMail,
} from "./cache_changes";
import type { MailAlias } from "./sent_cache";

export type IntentField = "folder" | "unread" | "starred";
export type IntentStatus = "pending" | "applied" | "failed";
export interface FieldIntent {
  revision: number;
  value: string | boolean;
  status: IntentStatus;
  origin?: number;
}
export interface MailIntent {
  id: string;
  account: string;
  fields: Partial<Record<IntentField, FieldIntent>>;
  // Last acknowledged action actually applied to this cache, atomically with
  // the mail row. Pending newer input does not erase the confirmed baseline.
  applied?: Partial<Record<IntentField, number>>;
}
export interface IntentLease {
  alias?: { id: string; lineage: string };
  id: string;
  account: string;
  revision: number;
  fields: Fields;
}
export interface IntentStore {
  reserve(): Promise<number>;
  register(id: string, fields: Fields): Promise<IntentLease>;
  claim(
    id: string,
    revision: number,
    fields: Fields,
    undoOf?: number,
  ): Promise<IntentLease>;
  effective(lease: IntentLease, id?: string): Promise<Fields>;
  uncached(lease: IntentLease): Promise<Fields>;
  finish(
    lease: IntentLease,
    status: Exclude<IntentStatus, "pending">,
  ): Promise<void>;
}
const names = [
  "mailIntents",
  "intentState",
  "mailAliases",
  "mailMetadata",
  "removedAccounts",
  ...cacheStores.filter((name) => name !== "mailMetadata"),
];
export const intentFields: IntentField[] = ["folder", "unread", "starred"];
const read = <T>(r: IDBRequest<T>) =>
  new Promise<T>((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
function counter(value: number) {
  if (!Number.isSafeInteger(value) || value < 1)
    throw Error("Invalid mail intent revision. Reopen the folder and retry.");
  return value;
}
export function intentValues(fields: Fields): Fields {
  const result: Fields = {};
  for (const key of intentFields) {
    const value = fields[key];
    if (value === undefined) continue;
    if (key === "folder") {
      if (typeof value !== "string" || !value || /[\x00-\x1f\x7f]/.test(value))
        throw Error("Choose a valid mail folder.");
      result.folder = value.toLowerCase() === "inbox" ? "INBOX" : value;
    } else {
      if (typeof value !== "boolean") throw Error("Choose a valid mail flag.");
      result[key] = value;
    }
  }
  return result;
}
/** Field ownership is independent of provider flags. Sync cannot erase newer
 * intent, and matching boolean values never justify an older group's Undo. */
export class BrowserIntents implements IntentStore {
  constructor(private db: IDBDatabase) {}
  private transaction<T>(
    mode: IDBTransactionMode,
    work: (tx: IDBTransaction) => Promise<T>,
  ): Promise<T> {
    return new Promise((resolve, reject) => {
      const tx = this.db.transaction(names, mode, { durability: "strict" });
      let result: T, cause: unknown;
      tx.oncomplete = () => resolve(result);
      tx.onabort = () =>
        reject(
          cause ??
            Error("Could not save mail intent. Keep Shep open and retry."),
        );
      // Await only IDB requests while this transaction owns its snapshot.
      void work(tx).then(
        (value) => {
          result = value;
        },
        (error) => {
          cause = error;
          try {
            tx.abort();
          } catch {
            reject(error);
          }
        },
      );
    });
  }
  private async next(tx: IDBTransaction) {
    const state = tx.objectStore("intentState");
    const previous = (await read<number | undefined>(state.get("clock"))) ?? 0;
    if (!Number.isSafeInteger(previous) || previous < 0)
      throw Error("The mail intent clock is invalid. Reopen Shep.");
    const revision = counter(previous + 1);
    state.put(revision, "clock");
    return revision;
  }
  private async record(tx: IDBTransaction, id: string): Promise<MailIntent> {
    const alias = await read<MailAlias | undefined>(
      tx.objectStore("mailAliases").get(id),
    );
    id = alias?.target ?? id;
    const mail = await read<CacheMail | undefined>(
      tx.objectStore("mailMetadata").get(id),
    );
    if (!mail)
      throw Error("This message is no longer cached. Refresh its folder.");
    const account = mail.core.account_id;
    if (await read(tx.objectStore("removedAccounts").get(account)))
      throw Error("This account was removed. Reopen Preferences.");
    const previous = await read<MailIntent | undefined>(
      tx.objectStore("mailIntents").get(id),
    );
    if (previous && previous.account !== account)
      throw Error("This message changed account. Refresh its review.");
    return previous ?? { id, account, fields: {} };
  }
  reserve() {
    return this.transaction("readwrite", (tx) => this.next(tx));
  }
  register(id: string, fields: Fields) {
    const values = intentValues(fields);
    return this.transaction("readwrite", async (tx) => {
      const record = await this.record(tx, id),
        revision = await this.next(tx);
      for (const key of intentFields)
        if (values[key] !== undefined)
          record.fields[key] = {
            revision,
            value: values[key]!,
            status: "pending",
          };
      tx.objectStore("mailIntents").put(record, record.id);
      recordCacheChanges(tx, [
        { store: "mailIntents", key: record.id, value: record },
      ]);
      return {
        id: record.id,
        account: record.account,
        revision,
        fields: values,
      };
    });
  }
  claim(id: string, revision: number, fields: Fields, undoOf?: number) {
    counter(revision);
    if (undoOf !== undefined) counter(undoOf);
    const values = intentValues(fields);
    return this.transaction("readwrite", async (tx) => {
      const latest =
        (await read<number | undefined>(
          tx.objectStore("intentState").get("clock"),
        )) ?? 0;
      if (revision > latest || (undoOf !== undefined && undoOf >= revision))
        throw Error("Reserve this group decision before changing messages.");
      const record = await this.record(tx, id),
        accepted: Fields = {};
      for (const key of intentFields) {
        const value = values[key];
        if (value === undefined) continue;
        const old = record.fields[key];
        if (old?.revision === revision && old.value !== value)
          throw Error(
            "This group action changed. Review it again before continuing.",
          );
        const retry =
          old?.revision === revision &&
          old.origin === undoOf &&
          old.status !== "applied";
        const owned =
          undoOf === undefined
            ? !old || old.revision < revision
            : old?.revision === undoOf;
        if (!retry && !owned) continue;
        record.fields[key] = {
          revision,
          value,
          status: "pending",
          ...(undoOf === undefined ? {} : { origin: undoOf }),
        };
        Object.assign(accepted, { [key]: value });
      }
      tx.objectStore("mailIntents").put(record, record.id);
      recordCacheChanges(tx, [
        { store: "mailIntents", key: record.id, value: record },
      ]);
      const alias =
        id !== record.id
          ? await read<MailAlias | undefined>(
              tx.objectStore("mailAliases").get(id),
            )
          : undefined;
      const metadata = alias
        ? await read<CacheMail | undefined>(
            tx.objectStore("mailMetadata").get(record.id),
          )
        : undefined;
      const proof =
        alias?.lineage &&
        alias.target === record.id &&
        alias.targetLineage === metadata?.lineage
          ? { id, lineage: alias.lineage }
          : undefined;
      return {
        ...(proof ? { alias: proof } : {}),
        id: record.id,
        account: record.account,
        revision,
        fields: accepted,
      };
    });
  }
  effective(lease: IntentLease, id = lease.id) {
    counter(lease.revision);
    return this.transaction("readonly", async (tx) => {
      const current = await this.record(tx, lease.id),
        fields: Fields = {};
      if (current.account !== lease.account)
        throw Error("This message changed account. Refresh its review.");
      if (id !== lease.id && (await this.record(tx, id)).id !== current.id)
        throw Error(
          "This action belongs to another message. Refresh its review.",
        );
      for (const key of intentFields) {
        const intent = current.fields[key];
        if (
          intent?.revision === lease.revision &&
          intent.status === "pending" &&
          intent.value === lease.fields[key]
        )
          Object.assign(fields, { [key]: intent.value });
      }
      return fields;
    });
  }
  finish(lease: IntentLease, status: Exclude<IntentStatus, "pending">) {
    counter(lease.revision);
    if (!["applied", "failed"].includes(status))
      return Promise.reject(Error("Invalid mail intent outcome."));
    return this.transaction("readwrite", async (tx) => {
      const current = await this.record(tx, lease.id);
      if (current.account !== lease.account)
        throw Error("This message changed account. Refresh its review.");
      for (const key of intentFields) {
        const intent = current.fields[key];
        if (
          intent?.revision === lease.revision &&
          intent.value === lease.fields[key]
        )
          if (intent.status !== "applied" || status === "applied")
            intent.status = status;
      }
      tx.objectStore("mailIntents").put(current, current.id);
      recordCacheChanges(tx, [
        { store: "mailIntents", key: current.id, value: current },
      ]);
    });
  }
  uncached(lease: IntentLease) {
    counter(lease.revision);
    return this.transaction("readonly", async (tx) => {
      const current = await this.record(tx, lease.id),
        fields: Fields = {};
      if (current.account !== lease.account)
        throw Error("This message changed account. Refresh its review.");
      for (const key of intentFields)
        if (
          lease.fields[key] !== undefined &&
          (current.applied?.[key] ?? 0) < lease.revision
        )
          Object.assign(fields, { [key]: lease.fields[key] });
      return fields;
    });
  }
}

/** Only the cache writer calls this, in the transaction that saved the actual
 * acknowledged mail fields. A later action cannot be hidden by a lost reply. */
export async function acknowledgeIntentCache(
  tx: IDBTransaction,
  lease: IntentLease,
  changes: Change[],
) {
  counter(lease.revision);
  const alias = await read<MailAlias | undefined>(
      tx.objectStore("mailAliases").get(lease.id),
    ),
    id = alias?.target ?? lease.id;
  const mail = await read<CacheMail | undefined>(
    tx.objectStore("mailMetadata").get(id),
  );
  if (
    !mail ||
    mail.core.account_id !== lease.account ||
    !changes.some((c) => c.store === "mail" && c.key === id && c.value)
  )
    throw Error(
      "The saved message does not match this action. Refresh its folder.",
    );
  const store = tx.objectStore("mailIntents"),
    current = await read<MailIntent | undefined>(store.get(id));
  if (!current || current.account !== lease.account)
    throw Error("The saved action changed account. Refresh its review.");
  const fields = intentValues(lease.fields);
  current.applied ??= {};
  for (const key of intentFields) {
    if (fields[key] === undefined) continue;
    if (mail.core[key] !== fields[key])
      throw Error("The cache did not save this action's acknowledged fields.");
    if ((current.applied[key] ?? 0) > lease.revision)
      throw Error(
        "A newer action already reached this cache. Refresh before recovering the older result.",
      );
    current.applied[key] = Math.max(current.applied[key] ?? 0, lease.revision);
  }
  store.put(current, id);
}

/** Merge alias ownership inside the cache transaction. An asynchronously read
 * copy must never overwrite a newer flag decision made by another tab. */
export async function adoptIntentAliases(
  tx: IDBTransaction,
  changes: Change[],
) {
  const intents = tx.objectStore("mailIntents");
  for (const change of changes) {
    if (change.store !== "mailAliases" || !change.value) continue;
    const alias = change.value as MailAlias;
    if (alias.alias === alias.target) continue;
    const previous = await read<MailIntent | undefined>(
      intents.get(alias.alias),
    );
    if (!previous) continue;
    const current = await read<MailIntent | undefined>(
      intents.get(alias.target),
    );
    if (current && current.account !== previous.account)
      throw Error("Cannot merge mail intents from different accounts.");
    const merged: MailIntent = current ?? {
      ...previous,
      id: alias.target,
      fields: {},
    };
    for (const key of intentFields) {
      const value = previous.fields[key];
      const target = merged.fields[key];
      if (
        value &&
        target?.revision === value.revision &&
        target.value !== value.value
      )
        throw Error(
          "These message copies have conflicting action records. Refresh their review.",
        );
      if (
        value &&
        (!target ||
          target.revision < value.revision ||
          (target.revision === value.revision && value.status === "applied"))
      )
        merged.fields[key] = value;
      if (previous.applied?.[key] !== undefined) {
        merged.applied ??= {};
        merged.applied[key] = Math.max(
          merged.applied[key] ?? 0,
          previous.applied[key]!,
        );
      }
    }
    intents.put(merged, alias.target);
    intents.delete(alias.alias);
  }
}
