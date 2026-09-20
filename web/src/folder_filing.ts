import type { Outgoing } from "./provider";
import type { FolderPlan } from "./folder_mutations";
import { folderRequests, FolderMutationBlocked } from "./folder_fences";

export const filingIndexes = [
  ["filingConfigured", ["account.id", "account.sent_folder", "draft.id"]],
  ["filingSelected", ["sent.copyAccount.id", "sent.folder", "draft.id"]],
  ["filingCurrent", ["account.id", "sent.folder", "draft.id"]],
  ["filingReceipt", ["account.id", "sent.receipt.folder", "draft.id"]],
] as const;
export const filingClock = "folder-outgoing-clock";
export const filingRetiredIndexes = filingIndexes.map(([name, fields]) => [`${name}Retired`, [fields[0], "recovery.action", fields[1], fields[2]]] as const);
const read = <T>(request: IDBRequest<T>) => new Promise<T>((resolve, reject) => {
  request.onsuccess = () => resolve(request.result);
  request.onerror = () => reject(request.error);
});
async function pendingTargets(tx: IDBTransaction, name: string, account: string, from: string, to: string, draft?: string) {
  const outgoing = tx.objectStore("outgoing"), lower = draft ?? "", upper = draft ?? "\uffff";
  let count = await read(outgoing.index(name).count(IDBKeyRange.bound([account, from, lower], [account, to, upper])));
  for (const decision of ["returned", "local"]) count -= await read(outgoing.index(`${name}Retired`).count(IDBKeyRange.bound([account, decision, from, lower], [account, decision, to, upper])));
  return count;
}

/** Index keys contain filing metadata; reviews never read outgoing MIME. */
export async function reviewFiling(tx: IDBTransaction, account: string, plan: FolderPlan) {
  const member = plan.members.find(member => member.path === plan.source);
  if (!member) throw Error("The reviewed subtree root is missing.");
  for (const [name] of filingIndexes) {
    if (await pendingTargets(tx, name, account, member.mailbox.name, member.mailbox.name)) throw new FolderMutationBlocked("An Outbox entry owns a filing destination in this subtree. Finish or review its Sent copy before changing these folders.");
    if (member.mailbox.delimiter) {
      const prefix = member.path + member.mailbox.delimiter;
      if (await pendingTargets(tx, name, account, prefix, prefix + "\uffff")) throw new FolderMutationBlocked("An Outbox entry owns a filing destination below this folder. Finish or review its Sent copy before changing this subtree.");
    }
  }
  return await read<number | undefined>(tx.objectStore("intentState").get(filingClock)) ?? 0;
}

/** Existing reservations may save their receipt even while a review is open. */
export async function assertFilingAvailable(tx: IDBTransaction, value: Outgoing) {
  if (value.recovery?.action === "returned" || value.recovery?.action === "local") return;
  const targets = [
    [value.account?.id, value.account?.sent_folder],
    [value.sent?.copyAccount?.id, value.sent?.folder],
    [value.account?.id, value.sent?.folder],
    [value.account?.id, value.sent?.receipt?.folder],
  ];
  const pending = await folderRequests(tx);
  for (const [account, folder] of targets) {
    if (!account || !folder) continue;
    const blocked = pending.some(job => job.account === account && job.mutation?.review.plan.members.some(member => member.mailbox.name === folder || member.mailbox.delimiter && folder.startsWith(member.path + member.mailbox.delimiter)));
    if (!blocked) continue;
    let reserved = false;
    for (const [name] of filingIndexes) {
      if (await pendingTargets(tx, name, account, folder, folder, value.draft.id)) { reserved = true; break; }
    }
    if (!reserved) throw new FolderMutationBlocked("A saved folder change owns this filing destination. Finish or review it in Folder changes before queuing this message or choosing its Sent folder.");
  }
}
