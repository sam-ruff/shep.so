import type { IntentLease } from "./mail_intents";
import type { BulkIdentity, BulkReceipt } from "./bulk_journal";
import type { Fields } from "./model";
import { cacheStores, recordCacheChanges } from "./cache_changes";
import type { MailIntent } from "./mail_intents";
import type { CacheMail } from "./cache_changes";
import type { MailAlias } from "./sent_cache";
import { samePhysical, metadataIdentity } from "./mail_lineage";

export type ActionStatus = "Queued" | "Waiting" | "Running" | "Rejected" | "Uncertain" | "Repair" | "Succeeded";
export interface MailAction {
  id: string;
  account: string;
  lease: IntentLease;
  source: BulkIdentity;
  status: ActionStatus;
  receipt?: BulkReceipt;
  applied?: Fields;
  error?: string;
  owner?: string;
  connection?: string;
  undoneBy?: string;
}
export interface ActionPage {
  rows: MailAction[];
  next?: string;
}
export interface ActionActivity {
  page(after?: string, completed?: boolean): Promise<ActionPage>;
  update(lease: IntentLease, outcome: Partial<Pick<MailAction, "status" | "receipt" | "applied" | "error">>): Promise<void>;
  complete(lease: IntentLease, retain?: boolean): Promise<void>;
  get(id: string): Promise<MailAction | undefined>;
  dismissRejected(id: string): Promise<void>;
  adopt(id: string, expectedOwner: string, owner: string): Promise<MailAction | undefined>;
  acceptReviewed(expected: MailAction): Promise<void>;
  cancelQueued(expected: MailAction): Promise<void>;
}
export const actionId = (revision: number) => String(revision).padStart(16, "0");
const read = <T>(request: IDBRequest<T>) => new Promise<T>((resolve, reject) => {
  request.onsuccess = () => resolve(request.result);
  request.onerror = () => reject(request.error);
});

export async function queuedActionFields(tx: IDBTransaction): Promise<Map<string, Fields>> {
  const result = new Map<string, Fields>();
  const actions = await read<MailAction[]>(tx.objectStore("mailActions").getAll(undefined, 148));
  for (const action of actions) {
    if (!["Queued", "Waiting", "Running", "Uncertain"].includes(action.status)) continue;
    const alias = await read<MailAlias | undefined>(tx.objectStore("mailAliases").get(action.lease.id));
    const id = alias?.target ?? action.lease.id;
    const mail = await read<CacheMail | undefined>(tx.objectStore("mailMetadata").get(id));
    if (!mail || mail.core.account_id !== action.account) continue;
    const origin = action.source;
    const matches = origin.lineage
      ? mail.lineage === origin.lineage || (alias?.lineage === origin.lineage && alias.targetLineage === mail.lineage)
      : samePhysical(origin, metadataIdentity(mail));
    if (!matches) continue;
    const intent = await read<MailIntent | undefined>(tx.objectStore("mailIntents").get(id));
    if (intent?.account !== action.account) continue;
    const fields = result.get(id) ?? {};
    for (const key of ["folder", "unread", "starred", "accountId"] as const) {
      const field = intent.fields[key];
      if (field?.revision !== action.lease.revision || field.status !== "pending" || (intent.applied?.[key] ?? 0) >= field.revision) continue;
      Object.assign(fields, { [key]: field.value });
    }
    result.set(id, fields);
  }
  return result;
}

/** This is the individual intent owner's receipt ledger, never another dispatcher. */
export class BrowserActivity implements ActionActivity {
  constructor(private db: IDBDatabase) {}
  private transaction<T>(mode: IDBTransactionMode, work: (tx: IDBTransaction) => Promise<T>): Promise<T> {
    return new Promise((resolve, reject) => {
      const tx = this.db.transaction(["mailActions", "removedAccounts", "mailIntents", "mailAliases", ...cacheStores], mode, { durability: "strict" });
      let result: T, error: unknown;
      tx.oncomplete = () => resolve(result);
      tx.onabort = () => reject(error ?? Error("Could not save action progress. Keep Shep open and retry."));
      void work(tx).then(value => { result = value; }, cause => {
        error = cause;
        try { tx.abort(); } catch { reject(cause); }
      });
    });
  }
  page(after?: string, completed = false): Promise<ActionPage> {
    return this.transaction("readonly", async tx => {
      const records = await read<MailAction[]>(tx.objectStore("mailActions").getAll(after ? IDBKeyRange.lowerBound(after, true) : undefined, 148));
      const rows = records.filter(row => (row.status === "Succeeded") === completed).slice(0, 21);
      const more = rows.length > 20;
      rows.length = Math.min(rows.length, 20);
      return { rows, next: more ? rows.at(-1)!.id : undefined };
    });
  }
  get(id: string) {
    return this.transaction("readonly", tx => read<MailAction | undefined>(tx.objectStore("mailActions").get(id)));
  }
  update(lease: IntentLease, outcome: Partial<Pick<MailAction, "status" | "receipt" | "applied" | "error">>) {
    return this.transaction("readwrite", async tx => {
      const store = tx.objectStore("mailActions"), id = actionId(lease.revision);
      const row = await read<MailAction | undefined>(store.get(id));
      if (!row) {
        if (lease.action && outcome.status === "Running") throw Error("This action has already finished. Review its current message.");
        return;
      }
      if (row.account !== lease.account || row.lease.id !== lease.id || await read(tx.objectStore("removedAccounts").get(lease.account)))
        throw Error("This action belongs to a removed or changed account.");
      if (outcome.status === "Running" && row.status !== "Queued" && row.status !== "Waiting")
        throw Error("This action has already started. Check its saved result before repeating it.");
      // An acknowledgment cannot become a rejection after a later cache failure.
      if (row.receipt && outcome.status !== "Repair") return;
      store.put({ ...row, ...outcome }, id);
    });
  }
  complete(lease: IntentLease, retain = false) {
    return this.transaction("readwrite", async tx => {
      const store = tx.objectStore("mailActions"), id = actionId(lease.revision);
      const row = await read<MailAction | undefined>(store.get(id));
      if (!row || row.account !== lease.account || row.lease.id !== lease.id) return;
      if (!retain || !row.receipt) { store.delete(id); return; }
      store.put({ ...row, status: "Succeeded", error: undefined }, id);
      const completed = (await read<MailAction[]>(store.getAll(undefined, 148))).filter(action => action.status === "Succeeded");
      for (const old of completed.slice(0, Math.max(0, completed.length - 20))) store.delete(old.id);
    });
  }
  dismissRejected(id: string) {
    return this.get(id).then(row => {
      if (!row) return;
      if (row.status !== "Rejected") throw Error("This action needs its original recovery. Refresh Activity.");
      return this.acceptReviewed(row);
    });
  }
  adopt(id: string, expectedOwner: string, owner: string) {
    return this.transaction("readwrite", async tx => {
      const store = tx.objectStore("mailActions"), row = await read<MailAction | undefined>(store.get(id));
      if (!row || row.owner !== expectedOwner || !["Queued", "Waiting"].includes(row.status)) return;
      if (await read(tx.objectStore("removedAccounts").get(row.account))) return;
      const earlier = await read<MailAction[]>(store.getAll(IDBKeyRange.upperBound(id, true), 148));
      if (earlier.some(action => action.account === row.account && !["Rejected", "Succeeded"].includes(action.status))) return;
      const adopted = { ...row, owner };
      store.put(adopted, id);
      return adopted;
    });
  }
  acceptReviewed(expected: MailAction) {
    return this.retire(expected, ["Rejected", "Uncertain", "Running"]);
  }
  cancelQueued(expected: MailAction) {
    return this.retire(expected, ["Queued", "Waiting"]);
  }
  private retire(expected: MailAction, allowed: ActionStatus[]) {
    return this.transaction("readwrite", async tx => {
      const store = tx.objectStore("mailActions"), row = await read<MailAction | undefined>(store.get(expected.id));
      if (!row || JSON.stringify(row) !== JSON.stringify(expected) || row.receipt || !allowed.includes(row.status))
        throw Error("This saved action changed. Refresh its review.");
      if (allowed.includes("Queued")) {
        const earlier = await read<MailAction[]>(store.getAll(IDBKeyRange.upperBound(row.id, true), 148));
        if (earlier.some(action => action.lease.id === row.lease.id && !["Rejected", "Succeeded"].includes(action.status) && Object.keys(row.lease.fields).some(field => field in action.lease.fields)))
          throw Error("An earlier saved change affects the same field. Finish or review that change in Activity before cancelling this one.");
      }
      const alias = await read<{target: string} | undefined>(tx.objectStore("mailAliases").get(row.lease.id));
      const id = alias?.target ?? row.lease.id;
      const intent = await read<MailIntent | undefined>(tx.objectStore("mailIntents").get(id));
      if (intent && intent.account === row.account) {
        for (const field of Object.values(intent.fields))
          if (field?.revision === row.lease.revision && field.status === "pending") field.status = "failed";
        tx.objectStore("mailIntents").put(intent, id);
        recordCacheChanges(tx, [{ store: "mailIntents", key: id, value: intent }]);
      }
      store.delete(row.id);
    });
  }
}
