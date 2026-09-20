import type { FolderCreation } from "./folder_actions";
export class FolderMutationBlocked extends Error {}
const read = <T>(request: IDBRequest<T>) => new Promise<T>((resolve, reject) => {
  request.onsuccess = () => resolve(request.result);
  request.onerror = () => reject(request.error);
});

/** Admission and cache writes check the same durable account exclusion. */
export function folderRequests(tx: IDBTransaction) {
  return read<FolderCreation[]>(tx.objectStore("folderActions").index("active_id").getAll(IDBKeyRange.bound([1, ""], [1, "\uffff"]), 128));
}
export async function folderExclusions(tx: IDBTransaction) {
  const pending = await folderRequests(tx);
  return new Set(pending.filter(job => job.mutation).map(job => job.account));
}
export async function assertFolderAvailable(tx: IDBTransaction, accounts: Iterable<string>) {
  const excluded = await folderExclusions(tx);
  for (const account of accounts) if (excluded.has(account))
    throw new FolderMutationBlocked("A saved folder change owns this account. Finish or review it in Folder changes before changing its mail.");
}
