import type { CoreMail, RecordMail } from "./provider";
import type { Change } from "./storage";

export const cacheStores = [
  "mailMetadata",
  "mailChanges",
  "cacheState",
] as const;
export interface CacheMail {
  id: string;
  core: CoreMail;
  moved: boolean;
  newest: [number, string];
  oldest: [number, string];
}
export interface CacheState {
  revision: number;
  floor: number;
}
export interface CacheChange {
  revision: number;
  id?: string;
  reset?: boolean;
}
export function mailMetadata(value: unknown): CacheMail | undefined {
  const mail = value as RecordMail | undefined;
  if (!mail?.core || !Number.isFinite(mail.core.timestamp)) return;
  const id = mail.localId ?? mail.core.id;
  if (typeof id !== "string") return;
  return {
    id,
    core: mail.core,
    moved: !!mail.moved,
    newest: [-mail.core.timestamp, id],
    oldest: [mail.core.timestamp, id],
  };
}
/** Keep a bounded replay window, not another mailbox. A lagging worker detects
 * the floor and rebuilds current metadata from a read-only cache snapshot. */
export function recordCacheChanges(
  tx: IDBTransaction,
  changes: Change[],
  removedAccount = false,
) {
  const affected = new Set<string>();
  const metadata = tx.objectStore("mailMetadata");
  for (const change of changes) {
    if (change.store === "mail") {
      const value = mailMetadata(change.value);
      if (value) metadata.put(value, change.key);
      else metadata.delete(change.key);
    }
    if (change.store === "mail" || change.store === "mailAliases")
      affected.add(change.key);
  }
  if (!affected.size && !removedAccount) return;
  const states = tx.objectStore("cacheState"),
    journal = tx.objectStore("mailChanges");
  const request = states.get("mail");
  request.onsuccess = () => {
    const state: CacheState = request.result ?? { revision: 0, floor: 0 };
    if (state.revision > Number.MAX_SAFE_INTEGER - affected.size - 1) {
      tx.abort();
      return;
    }
    if (removedAccount || affected.size > 1024) {
      state.floor = ++state.revision;
      journal.clear();
      journal.put({ revision: state.revision, reset: true }, state.revision);
    } else {
      for (const id of affected) {
        const revision = ++state.revision;
        journal.put({ revision, id }, revision);
      }
      state.floor = Math.max(state.floor, state.revision - 1024);
      journal.delete(IDBKeyRange.upperBound(state.floor));
    }
    states.put(state, "mail");
  };
}
