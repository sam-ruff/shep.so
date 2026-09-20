import { folderConnection, type FolderCatalog, type FolderCreation, type Mailbox } from "./folder_actions";
import type { Account, RecordMail } from "./provider";
import { cacheStores, recordCacheChanges, type CacheMail, type CacheState } from "./cache_changes";
import { metadataIdentity, prepareLineage, samePhysical } from "./mail_lineage";
import type { MailAction } from "./mail_activity";
import type { Change } from "./storage";
import { roleFolders, type MailRoles } from "./sent_cache";
import { reviewFiling } from "./folder_filing";

export type FolderAction = "Delete" | { Move: { parent: string | null } } | { Rename: { name: string } };
export interface FolderMember {
  path: string;
  mailbox: Mailbox;
  listed: boolean;
  depth: number;
  destination: string | null;
}
export interface FolderPlan { source: string; action: FolderAction; members: FolderMember[]; parent: Mailbox | null }
export type FolderStep = { Rename: { source: string; destination: string } } | { Delete: { source: string } } | { Forget: { source: string } };
export interface FolderMutationReview {
  account: string;
  connection: string;
  plan: FolderPlan;
  epoch: string;
  revision: number;
  messages: number;
  outgoingRevision: number;
}
export interface FolderReceipt { step: FolderStep; origin: "acknowledged" | "observed" | "local" }
export interface FolderMutation {
  review: FolderMutationReview;
  prepared: number;
  completed: number;
  receipts: FolderReceipt[];
  receipt?: FolderReceipt;
  checked?: "applied" | "original" | "changed";
  checkedCache?: { epoch: string; revision: number };
}
export interface FrozenFolderMail {
  job: string;
  id: string;
  account: string;
  folder: string;
  remoteId: string;
  lineage: string;
}
export function folderSteps(plan: FolderPlan): FolderStep[] {
  if (plan.action !== "Delete") {
    const root = plan.members.find(member => member.path === plan.source);
    if (!root?.destination) throw Error("The reviewed destination is missing. Review the folder again.");
    return [{ Rename: { source: root.mailbox.name, destination: root.destination + root.mailbox.name.slice(root.path.length) } }];
  }
  return plan.members.map(member => member.listed && !member.mailbox.non_existent ? { Delete: { source: member.mailbox.name } } : { Forget: { source: member.mailbox.name } });
}
export function mutationJob(job: FolderCreation) {
  if (!job.mutation) throw Error("This saved request is not a folder subtree change.");
  return job.mutation;
}
export function localFolderPlan(catalog: Mailbox[], source: string, action: FolderAction): FolderPlan {
  const root = catalog.find(mailbox => mailbox.name === source);
  if (!root || source.toUpperCase() === "INBOX" || root.encoding !== "Utf8" || root.delimiter !== "/") throw Error("Refresh this local folder before changing it.");
  const members = catalog.filter(mailbox => mailbox.name === source || mailbox.name.startsWith(`${source}/`)).map(mailbox => ({ path: mailbox.name, mailbox, listed: true, depth: mailbox.name.slice(source.length).split("/").length - 1, destination: null as string | null })).sort((a, b) => b.depth - a.depth || a.path.localeCompare(b.path));
  let parent: Mailbox | null = null;
  if (action !== "Delete") {
    const originalParent = source.includes("/") ? source.slice(0, source.lastIndexOf("/")) : null;
    const parentName = "Move" in action ? action.Move.parent : originalParent;
    parent = parentName ? catalog.find(mailbox => mailbox.name === parentName) ?? null : null;
    if (parentName && (!parent || parent.no_inferiors || parent.non_existent || members.some(member => member.path === parentName))) throw Error("Choose an available parent outside this subtree.");
    const leaf = "Rename" in action ? action.Rename.name : source.slice(source.lastIndexOf("/") + 1);
    if (!leaf.trim() || /[\x00-\x1f\x7f/]/.test(leaf)) throw Error("Enter one valid folder name.");
    const destination = parentName ? `${parentName}/${leaf}` : leaf;
    if (destination === source || destination.toUpperCase() === "INBOX") throw Error("Choose a different destination.");
    for (const member of members) {
      member.destination = destination + member.path.slice(source.length);
      if (member.destination.length > 1024 || catalog.some(mailbox => mailbox.name === member.destination && !members.some(member => member.path === mailbox.name))) throw Error("The destination exists or its name is too long.");
    }
  }
  return { source, action, members, parent };
}

const read = <T>(request: IDBRequest<T>) => new Promise<T>((resolve, reject) => {
  request.onsuccess = () => resolve(request.result);
  request.onerror = () => reject(request.error);
});
const names = ["folderActions", "folderCatalogs", "folderMembers", "accounts", "removedAccounts", "mailActions", "mail", "raw", "mailAliases", "mailRoles", "outgoing", "intentState", ...cacheStores];

/** Subtree metadata and receipts belong to the existing folderActions journal. */
export class BrowserFolderMutations {
  constructor(private readonly db: IDBDatabase) {}
  private transaction<T>(mode: IDBTransactionMode, work: (tx: IDBTransaction) => Promise<T>) {
    return new Promise<T>((resolve, reject) => {
      const tx = this.db.transaction(names, mode, { durability: "strict" });
      let result: T, error: unknown;
      tx.oncomplete = () => resolve(result);
      tx.onabort = () => reject(error ?? Error("The saved folder change could not be updated. Keep it for review and retry."));
      void work(tx).then(value => { result = value; }, cause => { error = cause; try { tx.abort(); } catch { reject(cause); } });
    });
  }
  private async account(tx: IDBTransaction, id: string, connection?: string) {
    const account = await read<Account | undefined>(tx.objectStore("accounts").get(id));
    if (!account || await read(tx.objectStore("removedAccounts").get(id))) throw Error("This account was removed. Reopen Preferences.");
    if (connection !== undefined && folderConnection(account) !== connection) throw Error("The account connection changed. Keep this folder change for review.");
    return account;
  }
  private async available(tx: IDBTransaction, account: string) {
    const folders = await read<FolderCreation[]>(tx.objectStore("folderActions").index("active_id").getAll(IDBKeyRange.bound([1, ""], [1, "\uffff"]), 128));
    if (folders.some(job => job.account === account)) throw Error("Finish or review the saved folder changes on this account first.");
    const mail = await read<MailAction[]>(tx.objectStore("mailActions").getAll(undefined, 148));
    if (mail.some(job => job.account === account && job.status !== "Succeeded")) throw Error("Finish or review saved mail actions on this account first.");
  }
  private async protectedFolders(tx: IDBTransaction, account: Account, plan: FolderPlan) {
    const roles = await read<MailRoles | undefined>(tx.objectStore("mailRoles").get(account.id));
    const sent = roleFolders(account, roles);
    if (plan.members.some(member => member.mailbox.role !== null || sent.has(member.mailbox.name))) throw Error("Special-use folders need their account settings reviewed before they can be renamed, moved or deleted.");
  }
  review(accountId: string, plan: FolderPlan) {
    return this.transaction("readonly", async tx => {
      const account = await this.account(tx, accountId);
      await this.available(tx, accountId);
      if (!plan.members.length || plan.members.length > 128) throw Error("Review at most 128 folders at once.");
      await this.protectedFolders(tx, account, plan);
      const state = await read<CacheState>(tx.objectStore("cacheState").get("mail"));
      let messages = 0;
      for (const member of plan.members) messages += await read(tx.objectStore("mailMetadata").index("accountFolderId").count(IDBKeyRange.bound([accountId, member.mailbox.name, ""], [accountId, member.mailbox.name, "\uffff"])));
      const outgoingRevision = await reviewFiling(tx, accountId, plan);
      return { account: accountId, connection: folderConnection(account), plan, epoch: state.epoch!, revision: state.revision, messages, outgoingRevision } satisfies FolderMutationReview;
    });
  }
  admit(id: string, owner: string, review: FolderMutationReview) {
    return this.transaction("readwrite", async tx => {
      const existing = await read<FolderCreation | undefined>(tx.objectStore("folderActions").get(id));
      if (existing) {
        if (JSON.stringify(existing.mutation?.review) !== JSON.stringify(review)) throw Error("This folder request identity was already used.");
        return existing;
      }
      const account = await this.account(tx, review.account, review.connection);
      await this.available(tx, review.account);
      await this.protectedFolders(tx, account, review.plan);
      if (account.protocol === "Pop3") {
        const catalog = await read<FolderCatalog | undefined>(tx.objectStore("folderCatalogs").get(account.id));
        if (JSON.stringify(localFolderPlan(catalog?.mailboxes ?? [], review.plan.source, review.plan.action)) !== JSON.stringify(review.plan)) throw Error("The local subtree changed after this review. Review its folders again.");
      }
      if (await reviewFiling(tx, review.account, review.plan) !== review.outgoingRevision) throw Error("Outbox changed after this review. Review the folder again before confirming.");
      const state = await read<CacheState>(tx.objectStore("cacheState").get("mail"));
      if (state.epoch !== review.epoch || state.revision !== review.revision) throw Error("Mail changed after this review. Review the folder and its messages again.");
      if (await read(tx.objectStore("folderActions").index("active_id").count(IDBKeyRange.bound([1, ""], [1, "\uffff"]))) >= 128) throw Error("Review pending folder changes before adding another.");
      const job: FolderCreation = { id, owner, account: review.account, connection: review.connection, parent: null, name: review.plan.source, status: "Preparing", active: 1, revision: 1, mutation: { review, prepared: 0, completed: 0, receipts: [] } };
      tx.objectStore("folderActions").add(job, id);
      return job;
    });
  }
  private async current(tx: IDBTransaction, expected: FolderCreation) {
    const job = await read<FolderCreation | undefined>(tx.objectStore("folderActions").get(expected.id));
    if (!job || job.revision !== expected.revision || !job.mutation) throw Error("The saved folder change needs a fresh review.");
    await this.account(tx, job.account, job.connection);
    const state = await read<CacheState>(tx.objectStore("cacheState").get("mail"));
    if (state.epoch !== job.mutation.review.epoch) throw Error("The source cache was replaced. Keep the folder receipt for explicit review.");
    return job;
  }
  async prepare(initial: FolderCreation) {
    let job = initial;
    for (const member of mutationJob(job).review.plan.members) {
      let after = "", more = true;
      while (more) {
        const page = await this.transaction("readwrite", async tx => {
          const current = await this.current(tx, job), mutation = mutationJob(current);
          if (current.status !== "Preparing") throw Error("This folder preparation already finished or changed.");
          const state = await read<CacheState>(tx.objectStore("cacheState").get("mail"));
          if (state.epoch !== mutation.review.epoch) throw Error("The cache was replaced while preparing the folder review. Keep this request for review.");
          const rows = await read<CacheMail[]>(tx.objectStore("mailMetadata").index("accountFolderId").getAll(IDBKeyRange.bound([job.account, member.mailbox.name, after], [job.account, member.mailbox.name, "\uffff"], !!after), 50));
          for (const row of rows) {
            if (row.moved || !row.lineage) throw Error("A message needs identity recovery before changing this folder.");
            const saved: FrozenFolderMail = { job: job.id, id: row.id, account: job.account, folder: row.core.folder, remoteId: row.core.remote_id, lineage: row.lineage };
            tx.objectStore("folderMembers").add(saved, [job.id, row.id]);
          }
          mutation.prepared += rows.length;
          current.revision++;
          tx.objectStore("folderActions").put(current, current.id);
          return { job: current, after: rows.at(-1)?.id, more: rows.length === 50 };
        });
        job = page.job; after = page.after ?? after; more = page.more;
      }
    }
    return this.transaction("readwrite", async tx => {
      const current = await this.current(tx, job), mutation = mutationJob(current);
      if (current.status !== "Preparing" || mutation.prepared !== mutation.review.messages) throw Error("The prepared folder membership changed. Review it again.");
      current.status = "Queued"; current.revision++;
      tx.objectStore("folderActions").put(current, current.id);
      return current;
    });
  }
  receipt(expected: FolderCreation, receipt: FolderReceipt) {
    return this.transaction("readwrite", async tx => {
      const job = await this.current(tx, expected), mutation = mutationJob(job);
      if (!(["Running", "Checking"].includes(job.status) || job.status === "Uncertain" && mutation.checked === "applied" && mutation.review.plan.action === "Delete") || JSON.stringify(folderSteps(mutation.review.plan)[mutation.completed]) !== JSON.stringify(receipt.step)) throw Error("The returned folder receipt does not match its saved step.");
      mutation.receipt = receipt;
      job.status = "Repair"; job.error = undefined; job.revision++;
      tx.objectStore("folderActions").put(job, job.id);
      return job;
    });
  }
  checkedReceipt(expected: FolderCreation, checked: "applied" | "original" | "changed") {
    return this.transaction("readwrite", async tx => {
      const job = await read<FolderCreation | undefined>(tx.objectStore("folderActions").get(expected.id));
      if (!job || job.revision !== expected.revision || job.status !== "Checking" || !job.mutation?.receipt) throw Error("The folder receipt changed. Check it again.");
      const account = await this.account(tx, job.account, job.connection);
      const state = await read<CacheState>(tx.objectStore("cacheState").get("mail"));
      job.mutation.checked = checked;
      job.mutation.checkedCache = { epoch: state.epoch!, revision: state.revision };
      job.status = "Repair"; job.revision++;
      job.error = `${account.protocol === "Pop3" ? "The local folders were checked." : "The server was checked."} The acknowledged change remains. You can retry saving here, or keep the cached mail and stop tracking; this cache may still need refreshing.`;
      tx.objectStore("folderActions").put(job, job.id);
      return job;
    });
  }
  stopCheckedReceipt(expected: FolderCreation) {
    return this.transaction("readwrite", async tx => {
      const job = await read<FolderCreation | undefined>(tx.objectStore("folderActions").get(expected.id));
      if (!job || job.revision !== expected.revision || job.status !== "Repair" || !job.mutation?.receipt || !job.mutation.checkedCache) throw Error("Check this acknowledged folder change before stopping tracking.");
      await this.account(tx, job.account, job.connection);
      const state = await read<CacheState>(tx.objectStore("cacheState").get("mail")), observed = job.mutation.checkedCache;
      if (state.epoch !== observed.epoch || state.revision !== observed.revision) throw Error("The cache changed after the check. Check this folder receipt again before stopping tracking.");
      job.status = "Dismissed"; job.active = 0; job.revision++;
      job.error = "Tracking stopped after checking. The acknowledged change remains; cached mail was kept and may need refreshing. The receipt is retained here.";
      tx.objectStore("folderActions").put(job, job.id);
      return job;
    });
  }
  repair(expected: FolderCreation) {
    return this.transaction("readwrite", async tx => {
      const job = await this.current(tx, expected), mutation = mutationJob(job), receipt = mutation.receipt;
      if (job.status !== "Repair" || !receipt) throw Error("This folder step has no saved receipt to repair.");
      const step = receipt.step, renamed = "Rename" in step;
      const source = renamed ? undefined : "Delete" in step ? step.Delete.source : step.Forget.source;
      const members = tx.objectStore("folderMembers");
      const pending = renamed
        ? await read<FrozenFolderMail[]>(members.index("jobId").getAll(IDBKeyRange.bound([job.id, ""], [job.id, "\uffff"]), 50))
        : await read<FrozenFolderMail[]>(members.index("jobFolderId").getAll(IDBKeyRange.bound([job.id, source!, ""], [job.id, source!, "\uffff"]), 50));
      const metadata = new Map<string, CacheMail | undefined>(), changes: Change[] = [];
      let budget = 50;
      for (const frozen of pending) {
        if (!budget) break;
        const current = await read<CacheMail | undefined>(tx.objectStore("mailMetadata").get(frozen.id));
        if (!current || current.lineage !== frozen.lineage || !samePhysical(metadataIdentity(current), { id: frozen.id, account: frozen.account, folder: frozen.folder, remoteId: frozen.remoteId })) throw Error("A reviewed message changed identity. Keep this receipt and review the cache before continuing.");
        if (renamed) {
          const member = mutation.review.plan.members.find(member => member.mailbox.name === frozen.folder);
          if (!member?.destination) throw Error("The reviewed folder mapping is missing.");
          const mail = await read<RecordMail>(tx.objectStore("mail").get(frozen.id));
          mail.core.folder = member.destination + member.mailbox.name.slice(member.path.length);
          mail.core.id = `${job.account}:${mail.core.folder}:${mail.core.remote_id}`;
          mail.localId = frozen.id;
          const collision = await read<CacheMail | undefined>(tx.objectStore("mailMetadata").get(mail.core.id));
          const alias = await read<{ target: string } | undefined>(tx.objectStore("mailAliases").get(mail.core.id));
          if (collision && collision.id !== frozen.id || alias && alias.target !== frozen.id) throw Error("The destination cache has another identity. Keep this receipt and review both messages.");
          const change: Change = { store: "mail", key: frozen.id, value: mail, identity: { before: metadataIdentity(current), after: { id: frozen.id, account: job.account, folder: mail.core.folder, remoteId: mail.core.remote_id } } };
          const prepared = await prepareLineage(tx, [change]);
          metadata.set(frozen.id, prepared.get(frozen.id));
          tx.objectStore("mail").put(mail, frozen.id);
          if (mail.core.id !== frozen.id) {
            tx.objectStore("mailAliases").put({ ...alias, alias: mail.core.id, target: frozen.id }, mail.core.id);
            changes.push({ store: "mailAliases", key: mail.core.id });
          }
        } else {
          const aliases = await read<IDBValidKey[]>(tx.objectStore("mailAliases").index("target").getAllKeys(frozen.id, budget));
          for (const key of aliases) {
            tx.objectStore("mailAliases").delete(key);
            changes.push({ store: "mailAliases", key: String(key) });
            budget--;
          }
          if (!budget) break;
          tx.objectStore("mail").delete(frozen.id); tx.objectStore("raw").delete(frozen.id);
          metadata.set(frozen.id, undefined);
        }
        changes.push({ store: "mail", key: frozen.id });
        members.delete([job.id, frozen.id]);
        budget--;
      }
      recordCacheChanges(tx, changes, false, metadata);
      const more = renamed
        ? await read(members.index("jobId").count(IDBKeyRange.bound([job.id, ""], [job.id, "\uffff"])))
        : await read(members.index("jobFolderId").count(IDBKeyRange.bound([job.id, source!, ""], [job.id, source!, "\uffff"])));
      if (!more) {
        const catalog = await read<FolderCatalog | undefined>(tx.objectStore("folderCatalogs").get(job.account));
        let mailboxes = catalog?.mailboxes ?? mutation.review.plan.members.filter(member => member.listed).map(member => member.mailbox);
        mailboxes = renamed ? mailboxes.map(mailbox => {
          const member = mutation.review.plan.members.find(member => member.mailbox.name === mailbox.name);
          return member?.destination ? { ...mailbox, name: member.destination + member.mailbox.name.slice(member.path.length) } : mailbox;
        }) : mailboxes.filter(mailbox => mailbox.name !== source);
        tx.objectStore("folderCatalogs").put({ ...catalog, account: job.account, mailboxes }, job.account);
        mutation.receipts.push(receipt); mutation.receipt = undefined; mutation.completed++;
        job.status = mutation.completed === folderSteps(mutation.review.plan).length ? "Succeeded" : "Queued";
        job.active = job.status === "Succeeded" ? 0 : 1;
      }
      job.revision++; job.error = undefined;
      tx.objectStore("folderActions").put(job, job.id);
      return job;
    });
  }
}
