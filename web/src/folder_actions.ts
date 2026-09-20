import type { Account } from "./provider";
import type { FolderMutation } from "./folder_mutations";
import { assertFolderAvailable } from "./folder_fences";

export interface Mailbox {
  name: string;
  delimiter: string | null;
  encoding: "Utf8" | "ImapUtf7";
  selectable: boolean;
  no_inferiors: boolean;
  non_existent: boolean;
  role: "Archive" | "Drafts" | "Junk" | "Sent" | "Trash" | null;
}
export type FolderStage = "Preparing" | "Queued" | "Waiting" | "Running" | "Checking" | "Repair" | "Rejected" | "Uncertain" | "Succeeded" | "Dismissed";
export interface FolderCreation {
  id: string;
  account: string;
  connection: string;
  parent: string | null;
  name: string;
  owner: string;
  status: FolderStage;
  active: 0 | 1;
  revision: number;
  target?: Mailbox;
  receipt?: Mailbox;
  receiptOrigin?: "acknowledged" | "observed" | "local";
  mutation?: FolderMutation;
  error?: string;
}
export interface FolderCatalog { account: string; mailboxes: Mailbox[]; complete?: boolean }
export function folderStatus(job: FolderCreation): string {
  switch (job.status) {
    case "Preparing": return "Saving the reviewed messages";
    case "Queued": return "Waiting to start";
    case "Waiting": return "Waiting for the account";
    case "Running": return job.mutation ? "Changing folder" : "Creating folder";
    case "Checking": return "Checking the server";
    case "Repair": return "Saving on this device";
    case "Rejected": return "Needs review";
    case "Uncertain": return "Needs checking";
    case "Succeeded": return "Complete";
    case "Dismissed": return "No longer tracked";
  }
}
export type FolderCreationReply = { state: "observed"; mailbox: Mailbox } | { state: "acknowledged"; target: Mailbox } | { state: "waiting" } | { state: "rejected" } | { state: "uncertain" };
export interface FolderProvider {
  plan(parent: string | null, name: string): Promise<Mailbox>;
  inspect(target: Mailbox): Promise<Mailbox | null>;
  create(target: Mailbox): Promise<FolderCreationReply>;
}
class FolderPlanRejected extends Error {}
export function readFolderPlan(value: unknown): Mailbox {
  if (value && typeof value === "object" && "state" in value && value.state === "rejected")
    throw new FolderPlanRejected("The folder name or parent is unavailable. Review this request before retrying.");
  if (!validMailbox(value)) throw Error("Invalid folder plan received.");
  return value;
}
export const folderConnection = (a: Account) => JSON.stringify([
  a.id, a.email, a.protocol, a.host, a.port, a.username,
  a.incoming_security ?? "Tls", a.incoming_auth ?? "Password",
]);
export function validMailbox(value: unknown): value is Mailbox {
  if (!value || typeof value !== "object") return false;
  const m = value as Mailbox;
  return typeof m.name === "string" && m.name.length > 0 && m.name.length <= 1024 && !/[\x00-\x1f\x7f]/.test(m.name) &&
    (m.delimiter === null || typeof m.delimiter === "string" && [...m.delimiter].length === 1) &&
    ["Utf8", "ImapUtf7"].includes(m.encoding) &&
    [m.selectable, m.no_inferiors, m.non_existent].every(v => typeof v === "boolean") &&
    (m.role === null || ["Archive", "Drafts", "Junk", "Sent", "Trash"].includes(m.role));
}

/** Called under the existing account action lock, with one saved request. */
export async function executeFolderCreation(journal: BrowserFolders, initial: FolderCreation, provider: FolderProvider, stopping: () => boolean) {
  let job = initial;
  const errorText = (error: unknown) => error instanceof Error ? error.message : "The folder change could not finish. Review it before continuing.";
  if (stopping()) return job;
  if (["Checking", "Repair"].includes(job.status)) {
    if (!job.target) return journal.update(job, { status: "Uncertain", error: "The exact folder target is unavailable. Review this change before stopping tracking." });
    try {
      const observed = await provider.inspect(job.target);
      if (observed) return await journal.finish(job, observed);
      return await journal.update(job, { status: job.receipt ? "Repair" : "Rejected", error: job.receipt ? "The server acknowledged creation, but this folder is now absent. Review the server before stopping tracking." : "The exact folder is absent. You can retry this saved request." });
    } catch (error) {
      return journal.update(job, { error: errorText(error) });
    }
  }
  if (!["Queued", "Waiting"].includes(job.status)) return job;
  try {
    if (!job.target) {
      const target = await provider.plan(job.parent, job.name);
      if (!validMailbox(target)) throw Error("The service returned an invalid folder target.");
      job = await journal.update(job, { target, error: undefined });
    }
  } catch (error) {
    return journal.update(job, { status: error instanceof FolderPlanRejected ? "Rejected" : "Waiting", error: errorText(error) });
  }
  if (stopping()) return job;
  job = await journal.update(job, { status: "Running", error: undefined });
  let reply: FolderCreationReply;
  try { reply = await provider.create(job.target!); }
  catch { return journal.update(job, { status: "Uncertain", error: "The server result could not be confirmed. Check this exact folder before retrying; creation will not be repeated automatically." }); }
  if (reply.state === "waiting") return journal.update(job, { status: "Waiting", error: "The server could not be checked before creation. Reconnect or retry this saved request." });
  if (reply.state === "rejected") return journal.update(job, { status: "Rejected", error: "The server refused this folder request. Review its target before retrying." });
  if (reply.state === "uncertain") return journal.update(job, { status: "Uncertain", error: "The server result is unknown. Check the exact folder before deciding what to do." });
  const receipt = reply.state === "observed" ? reply.mailbox : reply.target;
  if (!validMailbox(receipt) || receipt.name !== job.target!.name || receipt.encoding !== job.target!.encoding)
    return journal.update(job, { status: "Uncertain", error: "The service returned a different folder identity. Check the saved target." });
  job = await journal.update(job, { status: "Repair", receipt, receiptOrigin: reply.state, error: undefined });
  try {
    const observed = reply.state === "observed" ? receipt : await provider.inspect(job.target!);
    if (!observed) throw Error("Creation was acknowledged, but the folder catalogue has not confirmed it. Check this saved change.");
    return await journal.finish(job, observed);
  } catch (error) { return journal.update(job, { error: errorText(error) }); }
}
const request = <T>(value: IDBRequest<T>) => new Promise<T>((resolve, reject) => {
  value.onsuccess = () => resolve(value.result);
  value.onerror = () => reject(value.error);
});

/** The folder domain journal is consumed by the existing account action owner. */
export class BrowserFolders {
  constructor(private readonly db: IDBDatabase) {}
  private transaction<T>(mode: IDBTransactionMode, work: (tx: IDBTransaction) => Promise<T>) {
    return new Promise<T>((resolve, reject) => {
      const tx = this.db.transaction(["folderActions", "folderCatalogs", "accounts", "removedAccounts"], mode, { durability: "strict" });
      let result: T, failure: unknown;
      tx.oncomplete = () => resolve(result);
      tx.onabort = () => reject(failure ?? Error("Folder changes could not be saved on this browser. Keep Shep open and retry."));
      void work(tx).then(value => { result = value; }, error => {
        failure = error;
        try { tx.abort(); } catch { reject(error); }
      });
    });
  }
  private async account(tx: IDBTransaction, id: string, expected?: string) {
    const account = await request<Account | undefined>(tx.objectStore("accounts").get(id));
    if (!account || await request(tx.objectStore("removedAccounts").get(id))) throw Error("This account was removed. Reopen Preferences.");
    if (expected !== undefined && folderConnection(account) !== expected) throw Error("The account connection changed. Review this folder request before continuing.");
    return account;
  }
  page(after?: string, completed = false) {
    return this.transaction("readonly", async tx => {
      const active = completed ? 0 : 1;
      const records = await request<FolderCreation[]>(tx.objectStore("folderActions").index("active_id").getAll(IDBKeyRange.bound([active, after ?? ""], [active, "\uffff"], !!after), 21));
      const more = records.length > 20;
      records.length = Math.min(records.length, 20);
      return { rows: records, next: more ? records.at(-1)!.id : undefined };
    });
  }
  get(id: string) {
    return this.transaction("readonly", tx => request<FolderCreation | undefined>(tx.objectStore("folderActions").get(id)));
  }
  admit(input: Pick<FolderCreation, "id" | "account" | "connection" | "name" | "parent" | "owner">) {
    return this.transaction("readwrite", async tx => {
      if (!input.name.trim() || input.name.length > 1024 || /[\x00-\x1f\x7f]/.test(input.name)) throw Error("Enter a valid folder name.");
      const account = await this.account(tx, input.account, input.connection);
      const actions = tx.objectStore("folderActions");
      const previous = await request<FolderCreation | undefined>(actions.get(input.id));
      if (previous) {
        if (["account", "connection", "name", "parent"].some(key => previous[key as keyof FolderCreation] !== input[key as keyof typeof input])) throw Error("This folder request identity was already used.");
        return previous;
      }
      await assertFolderAvailable(tx, [account.id]);
      if (await request(actions.index("active_id").count(IDBKeyRange.bound([1, ""], [1, "\uffff"]))) >= 128) throw Error("Review pending folder changes before adding another.");
      const job: FolderCreation = { ...input, status: "Queued", active: 1, revision: 1 };
      if (account.protocol === "Pop3") {
        const catalog = await request<FolderCatalog | undefined>(tx.objectStore("folderCatalogs").get(account.id));
        const mailboxes = catalog?.mailboxes ?? [];
        if (input.name.includes("/")) throw Error("Enter one folder name without the hierarchy separator.");
        const parent = input.parent ? mailboxes.find(m => m.name === input.parent) : undefined;
        if (input.parent && (!parent || parent.non_existent || parent.no_inferiors || parent.delimiter !== "/")) throw Error("Refresh the parent folder before creating a child.");
        const target: Mailbox = { name: input.parent ? `${input.parent}/${input.name}` : input.name, delimiter: "/", encoding: "Utf8", selectable: true, no_inferiors: false, non_existent: false, role: null };
        if (!validMailbox(target)) throw Error("The full folder name is too long.");
        const existing = mailboxes.find(m => m.name === target.name);
        if (existing && (!existing.selectable || existing.non_existent)) throw Error("This destination cannot receive mail.");
        job.target = job.receipt = existing ?? target;
        job.receiptOrigin = existing ? "observed" : "local";
        job.status = "Succeeded"; job.active = 0;
        if (!existing) {
          if (mailboxes.length >= 4096) throw Error("This folder catalog is too large to extend.");
          tx.objectStore("folderCatalogs").put({ ...catalog, account: account.id, mailboxes: [...mailboxes, target] }, account.id);
        }
      }
      actions.put(job, job.id);
      return job;
    });
  }
  update(expected: FolderCreation, changes: Partial<Pick<FolderCreation, "status" | "owner" | "target" | "receipt" | "receiptOrigin" | "mutation" | "error">>) {
    return this.transaction("readwrite", async tx => {
      const current = await request<FolderCreation | undefined>(tx.objectStore("folderActions").get(expected.id));
      if (!current || current.revision !== expected.revision) throw Error("This folder request changed. Review its current status.");
      await this.account(tx, current.account, changes.status === "Dismissed" ? undefined : current.connection);
      const next = { ...current, ...changes, revision: current.revision + 1 };
      next.active = ["Succeeded", "Dismissed"].includes(next.status) ? 0 : 1;
      tx.objectStore("folderActions").put(next, next.id);
      return next;
    });
  }
  finish(expected: FolderCreation, observed: Mailbox) {
    return this.transaction("readwrite", async tx => {
      await this.account(tx, expected.account, expected.connection);
      const current = await request<FolderCreation | undefined>(tx.objectStore("folderActions").get(expected.id));
      if (!current || current.revision !== expected.revision || !["Repair", "Checking"].includes(current.status)) throw Error("This folder request changed. Review its current status.");
      if (!validMailbox(observed) || !observed.selectable || observed.non_existent || observed.name !== current.target?.name || observed.encoding !== current.target.encoding) throw Error("The saved folder target was not confirmed.");
      const catalog = await request<FolderCatalog | undefined>(tx.objectStore("folderCatalogs").get(current.account));
      const mailboxes = (catalog?.mailboxes ?? []).filter(m => m.name !== observed.name);
      if (mailboxes.length >= 4096) throw Error("This folder catalog is too large to update.");
      tx.objectStore("folderCatalogs").put({ ...catalog, account: current.account, mailboxes: [...mailboxes, observed] }, current.account);
      const next: FolderCreation = { ...current, status: "Succeeded", active: 0, receipt: observed, receiptOrigin: current.receiptOrigin ?? "observed", error: undefined, revision: current.revision + 1 };
      tx.objectStore("folderActions").put(next, next.id);
      return next;
    });
  }
}
