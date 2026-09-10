import { samePhysical, metadataIdentity } from "./mail_lineage";
import type { CacheMail } from "./cache_changes";
import { BrowserGroups } from "./bulk_client";
import { MailboxWorkerClient } from "./mailbox_worker_client";
import type {
  MailboxRepository,
  MailScanEntry,
  MailScanQuery,
} from "./mailbox_types";
import { PrintLoader } from "./printing_loader";
import {
  BulkJournal,
  type BulkIdentity,
  type BulkReceipt,
} from "./bulk_journal";
import { SelectionWorkerClient } from "./selection_worker_client";
import { senderName } from "./mail_query";
import type { SelectionCommand, SelectionRepository } from "./selection_types";
import { ForwardLoader } from "./forward_loader";
import type { ForwardPrepared } from "./forward_content";
import {
  removalPreview,
  reviewStores,
  type RemovalReview,
} from "./account_removal";
import { AttachmentReader } from "./attachments";
import { DocumentLoader } from "./document_loader";
import {
  resolveMail,
  localId,
  insertSentCandidate,
  aliasChanges,
  removeMailChanges,
  roleFolders,
  acknowledgeRole,
  type MailAlias,
  type MailRoles,
} from "./sent_cache";
import { recoverSent, type SentWork } from "./sent";
import { envelope, replyDraft, type ReplyEnvelope } from "./reply";
import type { Session } from "./auth";
import { MutationFailure } from "./model";
import { intentValues, type IntentLease } from "./mail_intents";
import type {
  Repository,
  Mail,
  Draft,
  DraftAttachment,
  Fields,
  CalendarEntry,
} from "./model";
import type { LocalStore, Change } from "./storage";

export interface Account {
  id: string;
  name: string;
  email: string;
  protocol: "Imap" | "Pop3";
  host: string;
  port: number;
  username: string;
  incoming_security: "Tls" | "StartTls";
  incoming_auth: "Password" | "Plain";
  smtp_host: string;
  smtp_port: number;
  smtp_username: string;
  smtp_security: "Tls" | "StartTls";
  smtp_auth: "Automatic" | "Plain" | "Login";
  smtp_separate_password: boolean;
  sent_copy: "Automatic" | "ServerManaged" | "LocalOnly";
  sent_folder: string;
}
export interface Endpoint {
  host: string;
  port: number;
  service: "imap" | "pop3" | "smtp";
}
export interface Capabilities {
  mail: boolean;
  endpoints: Endpoint[];
  sent_copy: boolean;
}
export interface CoreMail {
  id: string;
  account_id: string;
  remote_id: string;
  folder: string;
  sender: string;
  recipient: string;
  subject: string;
  preview: string;
  timestamp: number;
  unread: boolean;
  starred: boolean;
  attachment_count: number;
}
interface StoredFile {
  draftId: string;
  info: DraftAttachment;
  blob: Blob;
  order: number;
}
function draftKey(d: Draft) {
  return JSON.stringify([
    d.accountId ?? "",
    d.to,
    d.cc,
    d.bcc,
    d.subject,
    d.body,
    d.inReplyTo ?? null,
    d.references ?? [],
    d.forward ?? null,
    d.forwardSource ?? null,
    d.attachments ?? [],
    d.revision ?? 0,
  ]);
}
async function base64(blob: Blob) {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let binary = "";
  for (let i = 0; i < bytes.length; i += 32768)
    binary += String.fromCharCode(...bytes.subarray(i, i + 32768));
  return btoa(binary);
}
export interface RecordMail {
  sentMessageId?: string | null;
  localEdited?: boolean;
  local?: boolean;
  reply?: ReplyEnvelope;
  core: CoreMail;
  text: string;
  moved?: boolean;
  localId?: string;
  pendingMove?: string;
  receipt?: {
    account: string;
    folder: string;
    current: CoreMail | null;
    fingerprint: { bytes: number; sha256: number[]; message_id: null };
  };
}
export interface MutationReceipt {
  receipt: BulkReceipt;
  cacheApplied: boolean;
  applied?: Fields;
}
export class MutationSuperseded extends MutationFailure {
  constructor() {
    super(
      "A newer choice replaced this change. Refresh the folder to display it.",
    );
  }
}
const displayFields = (fields: Fields): Fields => ({
  ...fields,
  ...(fields.folder === "INBOX" ? { folder: "Inbox" } : {}),
});
function physical(mail: RecordMail, origin?: BulkIdentity): BulkIdentity {
  return {
    id: localId(mail),
    account: mail.core.account_id,
    folder: mail.core.folder,
    remoteId: mail.moved ? "" : mail.core.remote_id,
    unread: mail.core.unread,
    starred: mail.core.starred,
    ...(origin?.lineage
      ? { lineage: origin.lineage, anchor: origin.anchor ?? origin.id }
      : {}),
  };
}
async function guardSource(
  store: LocalStore,
  mail: RecordMail,
  expected?: BulkIdentity,
) {
  const current = physical(mail);
  if (!expected) return current;
  let matches = samePhysical(expected, current);
  if (expected.lineage) {
    const metadata = await store.get<CacheMail>("mailMetadata", current.id);
    matches = false;
    if (
      metadata &&
      samePhysical(metadataIdentity(metadata), current) &&
      current.account === expected.account
    ) {
      matches = metadata.lineage === expected.lineage;
      if (!matches) {
        const alias = await store.get<MailAlias>(
          "mailAliases",
          expected.anchor ?? expected.id,
        );
        matches =
          !!alias &&
          alias.target === current.id &&
          alias.lineage === expected.lineage &&
          alias.targetLineage === metadata.lineage;
      }
    }
  }
  if (!matches)
    throw Error(
      "This message changed since the group review. Refresh the review before changing it.",
    );
  return physical(mail, expected);
}
interface PreparedWire {
  envelope: { from: string; to: string[] };
  raw: string;
}
export interface Outgoing {
  account?: Account;
  sent?: SentWork;
  sentError?: string;
  recovery?: { action: "returned" | "local" | "marked"; draftId?: string };
  wire?: PreparedWire;
  mail?: RecordMail;
  id: string;
  draft: Draft;
  state: string;
}
type Fetcher = typeof fetch;
type Lock = <T>(
  name: string,
  fn: () => Promise<T>,
  wait?: boolean,
) => Promise<T>;
async function browserLock<T>(
  name: string,
  fn: () => Promise<T>,
  wait = true,
): Promise<T> {
  if (!navigator.locks)
    throw new Error(
      "This browser cannot coordinate mail safely between tabs. Use a current browser.",
    );
  return await navigator.locks.request(
    name,
    { ifAvailable: !wait },
    async (lock) => {
      if (!lock)
        throw new Error(
          "This account has an operation in progress. Wait for it to finish, then check Outbox before removing it.",
        );
      return fn();
    },
  );
}
function serverId(mail: CoreMail) {
  return `${mail.account_id}:${mail.folder}:${mail.remote_id}`;
}
async function fingerprint(raw: string) {
  const bytes = Uint8Array.from(atob(raw), (c) => c.charCodeAt(0));
  return {
    bytes: bytes.length,
    sha256: [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))],
    message_id: null,
  };
}
function displayFolder(folder: string) {
  return folder.toUpperCase() === "INBOX" ? "Inbox" : folder;
}
function check(value: unknown): asserts value is Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("The mail service returned invalid data. Retry Refresh.");
}
function coreMail(value: unknown, account: string): CoreMail {
  check(value);
  for (const key of [
    "id",
    "account_id",
    "remote_id",
    "folder",
    "sender",
    "recipient",
    "subject",
    "preview",
  ])
    if (typeof value[key] !== "string")
      throw new Error("Invalid message metadata. Refresh was stopped.");
  if (
    value.account_id !== account ||
    !Number.isFinite(value.timestamp) ||
    typeof value.unread !== "boolean" ||
    typeof value.starred !== "boolean" ||
    !Number.isInteger(value.attachment_count)
  )
    throw new Error("Invalid message identity. Refresh was stopped.");
  return value as unknown as CoreMail;
}
async function* lines(response: Response): AsyncGenerator<unknown> {
  if (
    !response.body ||
    !response.headers.get("content-type")?.startsWith("application/x-ndjson")
  )
    throw new Error("The mail service did not return a sync stream.");
  const reader = response.body.getReader();
  const decoder = new TextDecoder("utf-8", { fatal: true });
  let buffer = "";
  try {
    while (true) {
      const { value, done } = await reader.read();
      buffer += decoder.decode(value, { stream: !done });
      // A production core message is currently limited to 25 MiB. Bound an
      // incomplete JSON event as well; large-mail streaming remains R23.
      if (buffer.length > 72 * 1024 * 1024)
        throw new Error("The sync response exceeded the message limit.");
      let end: number;
      while ((end = buffer.indexOf("\n")) >= 0) {
        const line = buffer.slice(0, end);
        buffer = buffer.slice(end + 1);
        if (line) yield JSON.parse(line);
      }
      if (done) {
        if (buffer.trim())
          throw new Error(
            "Sync ended with an incomplete message. Retry Refresh.",
          );
        break;
      }
    }
  } finally {
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}
export class GatewayRepository implements Repository, SelectionRepository {
  private groupClient?: BrowserGroups;
  get groups() {
    return (this.groupClient ??= new BrowserGroups(
      this,
      () => this.selectionWorker,
    ));
  }
  async bulkCacheEpoch() {
    const state = await this.store.get<{ epoch?: string }>(
      "cacheState",
      "mail",
    );
    if (!state?.epoch)
      throw Error("The device cache identity is unavailable. Reopen Shep.");
    return state.epoch;
  }
  async bulkUnavailable(id: string) {
    const mail = await resolveMail(this.store, id);
    if (!mail) return undefined; // Missing messages get their ordinary per-item result.
    const account = this.accounts.find((a) => a.id === mail.core.account_id);
    return account &&
      account.protocol !== "Pop3" &&
      !mail.local &&
      !this.connected(account.id)
      ? "Reconnect this account in Preferences, then retry the failed message and resume the group in History."
      : undefined;
  }
  private mailboxWorker?: MailboxWorkerClient;
  private mailboxStopped = false;
  private mailboxAdapter?: MailboxRepository;
  private queryWorker() {
    if (this.mailboxStopped)
      throw Error("Mailbox storage is closed. Reopen Shep.");
    if (this.mailboxWorker?.stopped) {
      this.mailboxWorker.terminate();
      this.mailboxWorker = undefined;
    }
    return (this.mailboxWorker ??= new MailboxWorkerClient(
      this.session.user_id,
    ));
  }
  get mailbox(): MailboxRepository | undefined {
    if (!this.store.profileId) return;
    return (this.mailboxAdapter ??= {
      page: async (query) => {
        if (!query.undo) await this.groupClient?.settledDecision();
        const page = await this.queryWorker().page(query);
        if (!query.previewOnly) {
          this.cached = page.rows;
          this.aliases = new Map(Object.entries(page.aliases));
        }
        return page;
      },
      detail: async (id) => this.queryWorker().detail(id),
      prefetch: async (id) => this.queryWorker().prefetch(id),
      metadata: async (id) => this.queryWorker().metadata(id),
      close: async () => {
        this.mailboxStopped = true;
        const worker = this.mailboxWorker;
        this.mailboxWorker = undefined;
        await worker?.close();
      },
    });
  }
  stopMailbox() {
    this.mailboxStopped = true;
    this.groupClient?.stop();
    this.mailboxWorker?.terminate();
    this.mailboxWorker = undefined;
  }
  private async *mailEntries(
    query: Omit<MailScanQuery, "after">,
  ): AsyncGenerator<MailScanEntry> {
    if (this.store.profileId) {
      let after: string | null = null;
      do {
        const page = await this.queryWorker().scan({ ...query, after });
        for (const row of page.rows) yield row;
        after = page.next;
      } while (after !== null);
    } else {
      // The in-memory protocol-test adapter has no browser worker/IndexedDB.
      for (const mail of await this.store.all<RecordMail>("mail")) {
        if (
          mail.core.account_id !== query.account ||
          (query.folder !== undefined && mail.core.folder !== query.folder) ||
          (query.serverId !== undefined && mail.core.id !== query.serverId)
        )
          continue;
        yield {
          key: localId(mail),
          core: mail.core,
          local: !!mail.local,
          moved: !!mail.moved,
          pendingMove: !!mail.pendingMove,
          syncReady: !!mail.reply && mail.sentMessageId !== undefined,
          sentMessageId: mail.sentMessageId,
        };
      }
    }
  }
  private selectionWorker?: SelectionWorkerClient;
  // Account scope uses the stable account ID, as does the native repository.
  selection(command: SelectionCommand, observed: string[] = []) {
    if (this.selectionWorker?.stopped) {
      this.selectionWorker = undefined;
      // A dead worker's private SQLite database and captures no longer exist.
      if (command.kind === "release") return Promise.resolve(null);
    }
    return (this.selectionWorker ??= new SelectionWorkerClient(
      this.session.user_id,
    )).selection(command, observed);
  }
  async closeSelections() {
    const worker = this.selectionWorker;
    this.selectionWorker = undefined;
    await worker?.close();
  }
  get accountIds() {
    return new Map(this.accounts.map((a) => [a.email, a.id]));
  }
  createPrinter() {
    return new PrintLoader(this.session.user_id);
  }
  preview = false;
  cached: Mail[] = [];
  events: CalendarEntry[] = [];
  drafts: Draft[] = [];
  accounts: Account[] = [];
  removedAccounts = new Set<string>();
  private attachmentReader?: AttachmentReader;
  private documentLoader?: DocumentLoader;
  private forwardLoader?: ForwardLoader;
  get formattedMessages() {
    return (this.documentLoader ??= new DocumentLoader(this.session.user_id));
  }
  get incomingAttachments() {
    return (this.attachmentReader ??= new AttachmentReader(
      this.session.user_id,
    ));
  }
  folders = new Map<string, string[]>();
  aliases = new Map<string, string>();
  folderRoles = new Map<string, Set<string>>();
  warning: string | null = null;
  private removalWarning: string | null = null;
  private secrets = new Map<string, { incoming: string; smtp: string }>();
  private records = new Map<string, RecordMail>();
  private sentAcknowledgments = new Map<string, SentWork>();
  constructor(
    private session: Session,
    private store: LocalStore,
    private request: Fetcher = (input, init) => fetch(input, init),
    private lock: Lock = browserLock,
    private prepareForward?: (id: string) => Promise<ForwardPrepared>,
  ) {
    if (store.profileId && store.profileId !== session.user_id)
      throw Error(
        "This mail cache belongs to another browser profile. Reopen Shep.",
      );
  }
  private exclusive<T>(scope: string, fn: () => Promise<T>, wait = true) {
    return this.lock(`shep.${this.session.user_id}.${scope}`, fn, wait);
  }
  async load() {
    [this.accounts, this.drafts] = await Promise.all([
      this.store.all<Account>("accounts"),
      this.store.all<Draft>("drafts"),
    ]);
    for (const draft of this.drafts)
      draft.attachments = (await this.files(draft.id)).map((f) => f.info);
    await this.reloadMail();
    if (this.store.profileId) {
      try {
        await BulkJournal.recoverAccounts(this.profileId);
        if (this.warning === this.removalWarning) this.warning = null;
        this.removalWarning = null;
      } catch (error) {
        this.warning = this.removalWarning =
          "Account removal history cleanup could not finish. " +
          (error instanceof Error ? error.message : "Reopen Shep to retry.");
      }
    }
  }
  private async reloadMail(changedId?: string) {
    if (this.mailbox) {
      const snapshot = await this.store.snapshot(["mailRoles"]);
      const roles = snapshot.mailRoles as MailRoles[];
      this.folderRoles = new Map(
        this.accounts.map((a) => [
          a.id,
          roleFolders(
            a,
            roles.find((r) => r.account === a.id),
          ),
        ]),
      );
      if (changedId) {
        const current = await this.mailbox.metadata(changedId);
        if (changedId !== current.id) this.aliases.set(changedId, current.id);
        this.cached = this.cached.filter(
          (m) => m.id !== changedId && m.id !== current.id,
        );
        if (current.mail) this.cached.push(current.mail);
        this.cached = this.cached.slice(-50);
      }
      return;
    }
    const snapshot = await this.store.snapshot([
      "mail",
      "mailRoles",
      "mailAliases",
    ]);
    const roles = snapshot.mailRoles as MailRoles[],
      aliases = snapshot.mailAliases as MailAlias[];
    this.aliases = new Map(aliases.map((a) => [a.alias, a.target]));
    this.folderRoles = new Map(
      this.accounts.map((a) => [
        a.id,
        roleFolders(
          a,
          roles.find((r) => r.account === a.id),
        ),
      ]),
    );
    this.records = new Map(
      (snapshot.mail as RecordMail[]).map((m) => [localId(m), m]),
    );
    this.cached = [...this.records.values()]
      .filter((m) => !m.moved)
      .map((record) => {
        const { core: m, text } = record;
        const address = m.sender.match(/<([^<>]+)>/)?.[1] ?? m.sender;
        return {
          id: localId(record),
          sender: senderName(m.sender),
          address,
          subject: m.subject,
          preview: m.preview,
          body: text,
          date: new Date(m.timestamp * 1000).toISOString(),
          account:
            this.accounts.find((a) => a.id === m.account_id)?.email ??
            m.account_id,
          folder: displayFolder(m.folder),
          unread: m.unread,
          starred: m.starred,
          attachments: Array.from(
            { length: Math.min(m.attachment_count, 100) },
            (_, i) => `Attachment ${i + 1}`,
          ),
          accountId: m.account_id,
        };
      });
  }
  private async response(path: string, body?: unknown) {
    const connected = (
      body as { connection?: { account?: Account } } | undefined
    )?.connection?.account;
    if (connected && (await this.store.get("removedAccounts", connected.id))) {
      this.secrets.delete(connected.id);
      throw new Error(
        "This account was removed in another tab. Reopen Preferences.",
      );
    }
    const response = await this.request(path, {
      method: body === undefined ? "GET" : "POST",
      credentials: "same-origin",
      cache: "no-store",
      redirect: "error",
      headers:
        body === undefined
          ? {}
          : {
              "content-type": "application/json",
              "x-shep-csrf": this.session.csrf,
            },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (response.status === 401) {
      this.secrets.clear();
      throw new Error(
        "Your beta session expired. Sign in again; cached mail and drafts stay on this browser.",
      );
    }
    return response;
  }
  private async json(path: string, body?: unknown) {
    const response = await this.response(path, body);
    const value: unknown = await response.json();
    check(value);
    if (!response.ok)
      throw new Error(
        typeof value.error === "string"
          ? value.error
          : "The mail service could not complete this request. Retry.",
      );
    return value;
  }
  async capabilities(): Promise<Capabilities> {
    const c = await this.json("/api/capabilities");
    if (typeof c.mail !== "boolean" || !Array.isArray(c.endpoints))
      throw new Error("Invalid service capabilities.");
    return c as unknown as Capabilities;
  }
  connected(id: string) {
    return this.secrets.has(id);
  }
  forgetPasswords() {
    this.secrets.clear();
  }
  async connect(account: Account, password: string, smtpPassword: string) {
    await this.exclusive(`account.${account.id}`, async () => {
      if (await this.store.get("removedAccounts", account.id))
        throw new Error(
          "This account was removed. Add it again as a new account in Preferences.",
        );
      // A connection edit cannot silently reassign cached remote identities.
      const existing = await this.store.get<Account>("accounts", account.id);
      if (
        existing &&
        JSON.stringify({
          ...existing,
          sent_copy: account.sent_copy,
          sent_folder: account.sent_folder,
        }) !== JSON.stringify(account)
      )
        throw new Error(
          "Add changed server settings as a new account to preserve this account's cached mail.",
        );
      if (existing)
        account = {
          ...account,
          sent_copy: existing.sent_copy,
          sent_folder: existing.sent_folder,
        };
      await this.json("/api/mail/probe", {
        connection: { account, password },
        smtp: false,
      });
      await this.json("/api/mail/probe", {
        connection: { account, password: smtpPassword },
        smtp: true,
      });
      await this.store.commit([
        { store: "accounts", key: account.id, value: account },
      ]);
      this.secrets.set(account.id, { incoming: password, smtp: smtpPassword });
      this.accounts = [
        ...this.accounts.filter((a) => a.id !== account.id),
        account,
      ];
    });
  }
  async removalPreview(id: string) {
    const review = removalPreview(await this.store.snapshot(reviewStores), id);
    if (this.store.profileId)
      review.groups = await BulkJournal.inspect(this.profileId, (journal) =>
        journal.accountReview(id),
      );
    return review;
  }
  async removeAccount(review: RemovalReview, discard: boolean) {
    if (!this.store.removeAccount)
      throw new Error(
        "Account removal is unavailable in this storage adapter.",
      );
    // Same draft -> account -> cache order as SMTP/Sent; a held operation
    // produces immediate feedback instead of trapping the user in this dialog.
    const lockDrafts = (index: number): Promise<void> =>
      index < review.draftIds.length
        ? this.exclusive(
            `draft.${review.draftIds[index]}`,
            () => lockDrafts(index + 1),
            false,
          )
        : this.exclusive(
            `account.${review.id}`,
            () =>
              this.exclusive(
                `cache.${review.id}`,
                () => this.store.removeAccount!(review, discard),
                false,
              ),
            false,
          );
    this.warning = null;
    if (this.store.profileId) {
      await BulkJournal.own(this.profileId, async (journal) => {
        const removed = await this.store.get<{ token: string }>(
          "removedAccounts",
          review.id,
        );
        if (removed?.token !== review.token) {
          if (!review.groups || review.groups.account !== review.id)
            throw Error(
              "Reload removal counts to include group changes before continuing.",
            );
          await journal.checkAccountReview(review.groups, discard);
        }
        try {
          await lockDrafts(0);
        } catch (error) {
          // An adapter can lose the reply after the mail transaction commits.
          // Its durable token is authoritative; never resurrect that account.
          if (
            (
              await this.store.get<{ token: string }>(
                "removedAccounts",
                review.id,
              )
            )?.token !== review.token
          )
            throw error;
        }
        try {
          await journal.reconcileAccounts();
        } catch {
          this.warning = this.removalWarning =
            "Account removed. Group history cleanup is pending. Reopen Shep to retry; queued group changes cannot run until cleanup finishes.";
        }
      });
    } else await lockDrafts(0);
    this.secrets.delete(review.id);
    this.accounts = this.accounts.filter((a) => a.id !== review.id);
    this.cached = this.cached.filter((m) => m.accountId !== review.id);
    this.drafts = this.drafts.filter((d) => d.accountId !== review.id);
    this.folders.delete(review.id);
    this.folderRoles.delete(review.id);
    this.records = new Map(
      [...this.records].filter(([, m]) => m.core.account_id !== review.id),
    );
    this.removedAccounts.add(review.id);
    for (const [key, work] of this.sentAcknowledgments)
      if (work.copyAccount?.id === review.id)
        this.sentAcknowledgments.delete(key);
  }
  private connection(account: Account, smtp = false) {
    const password = this.secrets.get(account.id)?.[smtp ? "smtp" : "incoming"];
    if (!password)
      throw new Error(
        `Reconnect ${account.email} in Preferences → Mail accounts. Passwords stay in this tab only.`,
      );
    return { account, password };
  }
  async refresh(folder = "Inbox", selected?: string | null): Promise<Mail[]> {
    this.warning = this.removalWarning;
    const removed = await this.store.all<{ id: string }>("removedAccounts");
    this.removedAccounts = new Set(removed.map((r) => r.id));
    if (this.accounts.some((a) => this.removedAccounts.has(a.id))) {
      for (const id of this.removedAccounts) this.secrets.delete(id);
      await this.load();
      this.warning = [
        "An account was removed in another tab. Its local mail and drafts were removed.",
        this.removalWarning,
      ]
        .filter(Boolean)
        .join(" ");
      return this.cached;
    }
    const failures: string[] = [];
    const accounts = this.accounts.filter(
      (a) => !selected || selected === a.id || selected === a.email,
    );
    if (!accounts.length)
      throw new Error("Add a mail account in Preferences to refresh.");
    for (const account of accounts) {
      try {
        await this.exclusive(`account.${account.id}`, async () => {
          const roles = await this.store.get<MailRoles>(
            "mailRoles",
            account.id,
          );
          const destinations =
            folder === "Sent" && account.protocol === "Imap"
              ? [...roleFolders(account, roles)]
              : [folder];
          if (!destinations.length && account.protocol === "Imap")
            destinations.push("Sent");
          for (const destination of destinations)
            await this.sync(account, destination);
        });
      } catch (error) {
        failures.push(
          error instanceof Error
            ? error.message
            : "Sync failed. Retry Refresh.",
        );
      }
    }
    await this.reloadMail();
    // Partial successes are visible, accompanied by a persistent error rather
    // than a false all-accounts success acknowledgment.
    this.warning =
      [...failures, this.warning].filter(Boolean).join(" ") || null;
    return structuredClone(this.cached);
  }
  private async sync(account: Account, folder: string) {
    if (account.protocol === "Pop3" && folder !== "Inbox") return;
    const connection = this.connection(account);
    const before = new Map<string, string>();
    const knownIds: string[] = [];
    for await (const entry of this.mailEntries({ account: account.id })) {
      if (entry.moved) continue;
      before.set(entry.core.id, entry.key);
      if (!entry.local && entry.syncReady) knownIds.push(entry.core.id);
    }
    const response = await this.response("/api/mail/sync", {
      connection,
      folder: folder === "Inbox" ? "INBOX" : folder,
      known: knownIds,
    });
    if (!response.ok) {
      const e = await response.json();
      throw new Error(e.error ?? "Could not refresh mail.");
    }
    let complete = false;
    let reconcile: { folder: string; ids: Set<string> } | undefined;
    for await (const value of lines(response)) {
      check(value);
      if (complete) throw new Error("Unexpected data after sync completion.");
      await this.exclusive(`cache.${account.id}`, async () => {
        switch (value.kind) {
          case "message": {
            check(value.mail);
            const core = coreMail(value.mail.summary, account.id);
            if (
              displayFolder(core.folder) !== displayFolder(folder) ||
              typeof value.mail.text !== "string" ||
              typeof value.mail.raw !== "string"
            )
              throw new Error("Invalid sync message.");
            let matching: MailScanEntry | undefined;
            for await (const candidate of this.mailEntries({
              account: account.id,
              serverId: core.id,
            })) {
              if (!candidate.moved) {
                matching = candidate;
                break;
              }
            }
            const key = matching?.key ?? core.id;
            const current = await this.store.get<RecordMail>("mail", key);
            // POP3's server has no folders or flags; preserve this device's edits.
            if (account.protocol === "Pop3" && current)
              Object.assign(core, {
                folder: current.core.folder,
                unread: current.core.unread,
                starred: current.core.starred,
              });
            const incoming: RecordMail = {
              core,
              text: value.mail.text,
              localId: key,
              receipt: current?.receipt,
              pendingMove: current?.pendingMove,
              localEdited: current?.localEdited,
              reply: value.reply ? envelope(value.reply) : undefined,
              sentMessageId:
                typeof value.sent_message_id === "string"
                  ? value.sent_message_id
                  : null,
            };
            await this.store.commit(
              await insertSentCandidate(this.store, incoming, value.mail.raw),
            );
            break;
          }
          case "flags": {
            if (!Array.isArray(value.flags))
              throw new Error("Invalid sync flags.");
            for (const entry of value.flags) {
              if (
                !Array.isArray(entry) ||
                typeof entry[0] !== "string" ||
                typeof entry[1] !== "boolean" ||
                typeof entry[2] !== "boolean"
              )
                throw new Error("Invalid sync flags.");
            }
            for (const entry of value.flags) {
              const m = await resolveMail(
                this.store,
                before.get(entry[0]) ?? entry[0],
              );
              if (
                m &&
                m.core.account_id === account.id &&
                !m.moved &&
                account.protocol === "Imap"
              ) {
                Object.assign(m.core, { unread: entry[1], starred: entry[2] });
                // Each transaction retains one body, independent of the
                // provider's metadata batch size. Validate the batch first.
                await this.store.commit([
                  { store: "mail", key: localId(m), value: m },
                ]);
              }
            }
            break;
          }
          case "reconcile": {
            if (
              value.account !== account.id ||
              typeof value.folder !== "string" ||
              displayFolder(value.folder) !== displayFolder(folder) ||
              !Array.isArray(value.live_ids) ||
              value.live_ids.some((id) => typeof id !== "string")
            )
              throw new Error("Invalid sync reconciliation.");
            reconcile = {
              folder: value.folder,
              ids: new Set(value.live_ids as string[]),
            };
            break;
          }
          case "folders": {
            if (
              value.account !== account.id ||
              !Array.isArray(value.folders) ||
              value.folders.some((f) => typeof f !== "string")
            )
              throw new Error("Invalid mail folders.");
            this.folders.set(account.id, value.folders.map(displayFolder));
            break;
          }
          case "sent_folder": {
            if (
              value.account !== account.id ||
              (value.folder !== null && typeof value.folder !== "string")
            )
              throw new Error("Invalid Sent folder role.");
            const roles = (await this.store.get<MailRoles>(
              "mailRoles",
              account.id,
            )) ?? { account: account.id, acknowledged: [] };
            roles.discovered = value.folder as string | null;
            await this.store.commit([
              { store: "mailRoles", key: account.id, value: roles },
            ]);
            break;
          }
          case "skipped_large":
            this.warning =
              "Some messages exceed the current 25 MiB limit and were not downloaded.";
            break;
          case "error":
            throw new Error(
              typeof value.error === "string"
                ? value.error
                : "Mail sync failed. Retry Refresh.",
            );
          case "done":
            complete = true;
            break;
          default:
            throw new Error("Unknown sync event. Update Shep before retrying.");
        }
      });
    }
    if (!complete)
      throw new Error(
        "Mail sync was interrupted. Cached mail was kept; retry Refresh.",
      );
    if (reconcile && account.protocol === "Imap")
      await this.exclusive(`cache.${account.id}`, async () => {
        const changes: Change[] = [];
        const removed = new Set<string>();
        for await (const entry of this.mailEntries({
          account: account.id,
          folder: reconcile!.folder,
        })) {
          const current = entry;
          if (
            current.core.account_id !== account.id ||
            current.moved ||
            current.local ||
            current.core.folder !== reconcile!.folder
          )
            continue;
          if (!reconcile!.ids.has(current.core.id)) removed.add(current.key);
          else if (current.pendingMove) {
            const saved = await this.store.get<RecordMail>("mail", current.key);
            if (
              saved?.pendingMove &&
              !saved.moved &&
              saved.core.id === current.core.id
            ) {
              delete saved.pendingMove;
              await this.store.commit([
                { store: "mail", key: current.key, value: saved },
              ]);
            }
          }
        }
        changes.push(...(await removeMailChanges(this.store, removed)));
        await this.store.commit(changes);
      });
  }

  private async saveMoved(
    id: string,
    latest: RecordMail,
    intent?: IntentLease,
    before?: BulkIdentity,
  ) {
    const changes: Change[] = [];
    for await (const other of this.mailEntries({
      account: latest.core.account_id,
      serverId: latest.core.id,
    })) {
      if (
        other.key === id ||
        latest.moved ||
        other.moved ||
        other.core.id !== latest.core.id
      )
        continue;
      const [raw, otherRaw] = await Promise.all([
        this.store.get<string>("raw", id),
        this.store.get<string>("raw", other.key),
      ]);
      if (!raw || raw !== otherRaw)
        throw new Error(
          "The destination cache contains conflicting content. Refresh before another action.",
        );
      changes.push(
        ...(await aliasChanges(this.store, other.key, id)),
        { store: "mail", key: other.key },
        { store: "raw", key: other.key },
      );
    }
    changes.push({
      store: "mail",
      key: id,
      value: latest,
      ...(before
        ? { identity: { before, after: physical(latest, before) } }
        : {}),
    });
    await this.store.commit(changes, intent);
  }
  private async resolveMoved(id: string, latest: RecordMail, account: Account) {
    if (!latest.receipt)
      throw new Error(
        "This old move needs a destination refresh before another action.",
      );
    const result = await this.json("/api/mail/resolve-move", {
      connection: this.connection(account),
      receipt: latest.receipt,
    });
    const resolved = coreMail(result.mail, account.id);
    if (
      resolved.folder !== latest.receipt.folder ||
      resolved.id !== serverId(resolved)
    )
      throw new Error(
        "The recovered message has an invalid identity. Refresh before another action.",
      );
    const before = physical(latest);
    latest.core = resolved;
    latest.localId = id;
    latest.moved = false;
    delete latest.receipt;
    await this.exclusive(`cache.${account.id}`, () =>
      this.saveMoved(id, latest, undefined, before),
    );
  }
  async registerMutation(id: string, fields: Fields) {
    return this.store.intents?.register(id, fields);
  }
  get bulkIntents() {
    if (!this.store.intents)
      throw Error("Mail action storage is unavailable. Reopen Shep.");
    return this.store.intents;
  }
  get profileId() {
    return this.session.user_id;
  }
  async cancelMutation(lease: IntentLease) {
    await this.store.intents?.finish(lease, "failed");
  }
  /** Replay an acknowledged result into the cache, never into IMAP/SMTP. A
   * field already cached by this or a newer action keeps its current value. */
  async repairMutation(
    receipt: BulkReceipt,
    lease: IntentLease,
  ): Promise<BulkIdentity> {
    const intents = this.store.intents;
    if (!intents)
      throw Error("Mail action storage is unavailable. Reopen Shep.");
    const fields = intentValues(lease.fields),
      before = receipt.before,
      after = { ...receipt.after };
    if (
      before.id !== after.id ||
      before.account !== lease.account ||
      after.account !== lease.account ||
      Object.entries(fields).some(
        ([key, value]) => after[key as keyof BulkIdentity] !== value,
      )
    )
      throw Error(
        "The saved receipt does not match this action. Reopen its history.",
      );
    const account = this.accounts.find((a) => a.id === lease.account);
    if (!account) throw Error("This account was removed. Reopen Preferences.");
    return this.exclusive(`account.${account.id}`, async () => {
      const initial = await resolveMail(this.store, lease.id);
      if (!initial || initial.core.account_id !== account.id)
        throw Error(
          "The message identity changed. Refresh its recovery review.",
        );
      const source = await resolveMail(this.store, before.id);
      if (!source || localId(source) !== localId(initial))
        throw Error(
          "The saved receipt belongs to another message. Refresh its recovery review.",
        );
      await guardSource(
        this.store,
        initial,
        before.lineage ? before : undefined,
      );
      if (
        !Object.keys(await intents.uncached(lease)).length &&
        (after.remoteId || !receipt.recovery)
      ) {
        await intents.finish(lease, "applied");
        return after;
      }
      if (receipt.recovery) {
        const raw = await this.store.get<string>("raw", localId(initial));
        if (!raw)
          throw Error(
            "The original message is missing. Refresh before recovering this move.",
          );
        const actual = await fingerprint(raw);
        if (
          actual.bytes !== receipt.recovery.bytes ||
          actual.sha256.some((n, i) => n !== receipt.recovery!.sha256[i])
        )
          throw Error(
            "The cached content does not match this move. Refresh its recovery review.",
          );
        if (!after.remoteId) {
          if (
            !initial.moved &&
            initial.core.folder === after.folder &&
            /^[1-9]\d*\.[1-9]\d*$/.test(initial.core.remote_id)
          )
            after.remoteId = initial.core.remote_id;
          else {
            const response = await this.json("/api/mail/resolve-move", {
              connection: this.connection(account),
              receipt: {
                account: account.id,
                folder: after.folder,
                current: null,
                fingerprint: actual,
              },
            });
            const current = coreMail(response.mail, account.id);
            if (
              current.folder !== after.folder ||
              current.id !== serverId(current) ||
              !/^[1-9]\d*\.[1-9]\d*$/.test(current.remote_id)
            )
              throw Error(
                "The recovered message has an invalid identity. Refresh before another action.",
              );
            after.remoteId = current.remote_id;
          }
        }
      }
      await this.exclusive(`cache.${account.id}`, async () => {
        const latest = await resolveMail(this.store, lease.id);
        if (
          !latest ||
          localId(latest) !== localId(initial) ||
          latest.core.account_id !== account.id
        )
          throw Error(
            "The message identity changed. Refresh its recovery review.",
          );
        await guardSource(
          this.store,
          latest,
          before.lineage ? before : undefined,
        );
        const cachedBefore = physical(latest);
        const needed = await intents.uncached(lease);
        if (!Object.keys(needed).length) return;
        if (needed.folder !== undefined) {
          const current = physical(latest),
            same = (v: BulkIdentity) =>
              current.folder === v.folder && current.remoteId === v.remoteId;
          if (
            !same(before) &&
            !same(after) &&
            !(latest.moved && latest.receipt?.folder === after.folder)
          )
            throw Error(
              "This message moved again. Review its current folder before recovering the saved change.",
            );
          latest.core.folder = after.folder;
          if (account.protocol === "Imap" && !latest.local) {
            if (!/^[1-9]\d*\.[1-9]\d*$/.test(after.remoteId))
              throw Error(
                "Recover the destination identity before saving this move.",
              );
            latest.core.remote_id = after.remoteId;
            latest.core.id = serverId(latest.core);
          } else latest.localEdited = true;
          latest.localId = localId(initial);
          delete latest.pendingMove;
          delete latest.moved;
          delete latest.receipt;
        }
        Object.assign(latest.core, needed);
        await this.saveMoved(
          localId(latest),
          latest,
          { ...lease, fields: needed },
          cachedBefore,
        );
      });
      await intents.finish(lease, "applied");
      return after;
    });
  }
  async mutate(
    id: string,
    fields: Fields,
    lease?: IntentLease,
  ): Promise<Fields> {
    lease ??= await this.registerMutation(id, fields);
    let result: MutationReceipt;
    try {
      result = await this.mutateWithReceipt(
        id,
        fields,
        undefined,
        undefined,
        lease,
      );
    } catch (error) {
      if (error instanceof MutationFailure && error.applied)
        error.applied = displayFields(error.applied);
      if (!(error instanceof MutationSuperseded)) throw error;
      try {
        await this.reloadMail(id);
      } catch {
        throw error;
      }
      return {};
    }
    try {
      await this.reloadMail(id);
    } catch {
      throw new MutationFailure(
        "The change was saved, but the message list could not refresh. Refresh the folder to display it.",
        true,
        result.receipt,
        result.cacheApplied,
        displayFields(result.applied ?? fields),
      );
    }
    // Mail presentation calls the canonical IMAP INBOX folder "Inbox".
    return displayFields(result.applied ?? fields);
  }
  /** The durable runner receives server acknowledgment before fallible cache
   * work. Ordinary controls also retain committed status after display failure.
   * No list reload is part of the provider commit contract. */
  async mutateWithReceipt(
    id: string,
    fields: Fields,
    acknowledged?: (result: MutationReceipt) => Promise<void>,
    expected?: BulkIdentity,
    lease?: IntentLease,
  ): Promise<MutationReceipt> {
    let result: MutationReceipt;
    try {
      result = await this.performMutation(
        id,
        fields,
        acknowledged,
        expected,
        lease,
      );
    } catch (error) {
      if (lease) {
        // An uncertain wire result remains pending for explicit recovery. A
        // known receipt dominates a later cache/display/intent-save failure.
        const status =
          error instanceof MutationFailure
            ? error.committed
              ? "applied"
              : undefined
            : "failed";
        if (status) {
          try {
            await this.store.intents?.finish(lease, status);
          } catch {
            /* Retain the original recovery instruction. */
          }
        }
      }
      throw error;
    }
    if (lease) {
      try {
        await this.store.intents?.finish(lease, "applied");
      } catch {
        throw new MutationFailure(
          "The change was saved, but its local action record could not finish. Refresh before another action.",
          true,
          result.receipt,
          result.cacheApplied,
          result.applied,
        );
      }
    }
    return result;
  }
  private async performMutation(
    id: string,
    fields: Fields,
    acknowledged?: (result: MutationReceipt) => Promise<void>,
    expected?: BulkIdentity,
    lease?: IntentLease,
  ): Promise<MutationReceipt> {
    const effective = async () => {
      if (!lease) return;
      if (!this.store.intents)
        throw Error("Mail action storage is unavailable. Reopen Shep.");
      fields = await this.store.intents.effective(lease, id);
      if (!Object.keys(fields).length) throw new MutationSuperseded();
    };
    const m = await resolveMail(this.store, id);
    if (!m)
      throw new Error("This message is no longer cached. Refresh its folder.");
    const account = this.accounts.find((a) => a.id === m.core.account_id);
    if (!account)
      throw new Error("This account was removed. Reopen Preferences.");
    let receipt: BulkReceipt | undefined,
      cacheApplied = false;
    try {
      const local = await this.exclusive(`cache.${account.id}`, async () => {
        const latest = await resolveMail(this.store, id);
        if (!latest)
          throw new Error(
            "This message is no longer cached. Refresh its folder.",
          );
        if (!latest.local && account.protocol !== "Pop3") return false;
        await effective();
        if (latest.pendingMove || latest.receipt || latest.moved)
          throw new Error(
            "This message needs move recovery before another action.",
          );
        const before = await guardSource(this.store, latest, expected);
        const folder = fields.folder === "Inbox" ? "INBOX" : fields.folder;
        Object.assign(latest.core, fields, folder ? { folder } : {});
        latest.localEdited = true;
        await this.store.commit(
          [
            {
              store: "mail",
              key: localId(latest),
              value: latest,
              identity: { before, after: physical(latest, before) },
            },
          ],
          lease ? { ...lease, fields } : undefined,
        );
        receipt = { before, after: physical(latest, before) };
        cacheApplied = true;
        // Local cache commit is the acknowledgment for POP3/local Sent actions.
        await acknowledged?.({ receipt, cacheApplied, applied: fields });
        return true;
      });
      if (local) return { receipt: receipt!, cacheApplied, applied: fields };
      await this.exclusive(`account.${account.id}`, async () => {
        const latest = await resolveMail(this.store, id);
        if (!latest)
          throw new Error(
            "This message is no longer cached. Refresh its folder.",
          );
        id = localId(latest);
        await effective();
        if (latest.pendingMove)
          throw new Error(
            "A previous move has no saved acknowledgment. Refresh both folders and choose the current message; it was not moved again.",
          );
        if (
          account.protocol === "Imap" &&
          !latest.local &&
          (latest.receipt || latest.moved)
        )
          await this.resolveMoved(id, latest, account);
        await effective();
        if (
          account.protocol === "Imap" &&
          fields.folder &&
          (fields.unread !== undefined || fields.starred !== undefined)
        )
          throw new Error("Move and flag changes must be separate actions.");
        const before = await guardSource(this.store, latest, expected);
        const folder = fields.folder === "Inbox" ? "INBOX" : fields.folder;
        if (
          folder === latest.core.folder &&
          fields.unread === undefined &&
          fields.starred === undefined
        ) {
          receipt = { before, after: physical(latest, before) };
          cacheApplied = true;
          await acknowledged?.({ receipt, cacheApplied, applied: fields });
          return;
        }
        let recovery: BulkReceipt["recovery"];
        if (account.protocol === "Imap" && !latest.local) {
          const connection = this.connection(account);
          if (folder) {
            const raw = await this.store.get<string>("raw", id);
            if (!raw)
              throw new Error(
                "The original message is missing from this cache. Refresh before moving it.",
              );
            const proof = await fingerprint(raw);
            latest.pendingMove = folder;
            await this.store.commit([
              { store: "mail", key: id, value: latest },
            ]);
            const result = await this.mutation("/api/mail/move", {
              connection,
              mail: latest.core,
              folder,
            });
            const remote =
              typeof result.remote_id === "string" &&
              /^[1-9]\d*\.[1-9]\d*$/.test(result.remote_id)
                ? result.remote_id
                : null;
            delete latest.pendingMove;
            latest.core = {
              ...latest.core,
              folder,
              remote_id: remote ?? latest.core.remote_id,
            };
            if (remote) latest.core.id = serverId(latest.core);
            latest.localId = id;
            latest.moved = !remote;
            latest.receipt = {
              account: account.id,
              folder,
              current: remote ? { ...latest.core } : null,
              fingerprint: proof,
            };
            recovery = { bytes: proof.bytes, sha256: proof.sha256 };
          } else
            await this.mutation("/api/mail/flags", {
              connection,
              mail: latest.core,
              ...fields,
            });
          Object.assign(latest.core, fields, folder ? { folder } : {});
          receipt = {
            before,
            after: physical(latest, before),
            ...(recovery ? { recovery } : {}),
          };
          await acknowledged?.({
            receipt: structuredClone(receipt),
            cacheApplied: false,
            applied: fields,
          });
        } else {
          Object.assign(latest.core, fields, folder ? { folder } : {});
          latest.localEdited = true;
        }
        await this.exclusive(`cache.${account.id}`, () =>
          this.saveMoved(
            id,
            latest,
            lease ? { ...lease, fields } : undefined,
            before,
          ),
        );
        cacheApplied = true;
        // An account may have adopted a local Sent identity while waiting for
        // its lock. In that case the actual cache write acknowledges the action.
        if (!receipt) {
          receipt = { before, after: physical(latest, before) };
          await acknowledged?.({
            receipt: structuredClone(receipt),
            cacheApplied,
            applied: fields,
          });
        }
        if (latest.moved) {
          try {
            await this.resolveMoved(id, latest, account);
          } catch {
            throw new MutationFailure(
              "The message moved, but its destination identity needs recovery. Refresh the destination before another action.",
              true,
              receipt,
              cacheApplied,
            );
          }
          receipt = { ...receipt, after: physical(latest, before) };
        }
      });
      return { receipt: receipt!, cacheApplied, applied: fields };
    } catch (error) {
      if (receipt && !(error instanceof MutationFailure && error.committed))
        throw new MutationFailure(
          "The change was acknowledged, but its local progress could not be saved. Refresh before another action.",
          true,
          receipt,
          cacheApplied,
          fields,
        );
      if (error instanceof MutationFailure && error.committed)
        error.applied = fields;
      throw error;
    }
  }
  private async mutation(path: string, body: unknown) {
    try {
      const response = await this.json(path, body);
      if (response.committed !== true)
        throw new Error("The mail server did not acknowledge the change.");
      return response;
    } catch (error) {
      throw new MutationFailure(
        `${error instanceof Error ? error.message : "The change could not be confirmed."} Refresh to check the server before another action.`,
      );
    }
  }
  private async files(id: string): Promise<StoredFile[]> {
    return (await this.store.all<StoredFile>("draftFiles"))
      .filter((f) => f.draftId === id)
      .sort((a, b) => a.order - b.order);
  }
  async attachments(id: string) {
    return (await this.files(id)).map((f) => f.info);
  }
  private async editable(id: string) {
    if (await this.store.get("outgoing", id))
      throw new Error(
        "This draft has a delivery record. Its files cannot be changed.",
      );
    if (!(await this.store.get("drafts", id)))
      throw new Error("Save the draft before attaching files.");
  }
  async addFiles(id: string, files: File[]): Promise<DraftAttachment[]> {
    return this.exclusive(`draft.${id}`, async () => {
      await this.editable(id);
      const current = await this.files(id);
      if (current.length + files.length > 32)
        throw new Error("Attach at most 32 files to one message.");
      if (
        current.reduce((n, f) => n + f.info.size, 0) +
          files.reduce((n, f) => n + f.size, 0) >
        18 * 1024 * 1024
      )
        throw new Error("Attachments must total 18 MiB or less.");
      const start = Math.max(-1, ...current.map((f) => f.order)) + 1;
      const incoming = files.map((file, index) => {
        if (
          !file.name ||
          file.name.length > 1024 ||
          /[\x00-\x1f\x7f]/.test(file.name)
        )
          throw new Error("Choose a file with a valid name.");
        return {
          draftId: id,
          order: start + index,
          blob: file,
          info: {
            id: crypto.randomUUID(),
            name: file.name,
            media_type: file.type || "application/octet-stream",
            size: file.size,
          },
        };
      });
      await this.store.commit(
        incoming.map((f) => ({
          store: "draftFiles",
          key: f.info.id,
          value: f,
        })),
      );
      return [...current, ...incoming].map((f) => f.info);
    });
  }
  async removeFile(id: string, file: string): Promise<DraftAttachment[]> {
    return this.exclusive(`draft.${id}`, async () => {
      await this.editable(id);
      const current = await this.files(id);
      if (current.some((f) => f.info.id === file))
        await this.store.commit([{ store: "draftFiles", key: file }]);
      return current.filter((f) => f.info.id !== file).map((f) => f.info);
    });
  }
  async forward(id: string, draftId: string): Promise<Draft> {
    return this.exclusive(`draft.${draftId}`, async () => {
      const outgoing = await this.store.get<Outgoing>("outgoing", draftId);
      if (outgoing)
        throw new Error(
          "This forward has a delivery record. Open Drafts or review Outbox before forwarding again.",
        );
      const existing = await this.store.get<Draft>("drafts", draftId);
      if (existing) {
        if (existing.forwardSource !== id)
          throw new Error(
            "This forward belongs to another message. Open it from Drafts.",
          );
        return { ...existing, attachments: await this.attachments(draftId) };
      }
      const prepared = await (this.prepareForward
        ? this.prepareForward(id)
        : (this.forwardLoader ??= new ForwardLoader(this.session.user_id)).load(
            id,
          ));
      if (!(await this.store.get<Account>("accounts", prepared.accountId)))
        throw new Error(
          "This account was removed. Forward from a connected account.",
        );
      const files: StoredFile[] = prepared.files.map((file, order) => ({
        draftId,
        order,
        info: { ...file.info, id: crypto.randomUUID() },
        blob: new Blob([file.bytes as Uint8Array<ArrayBuffer>], {
          type: file.info.media_type,
        }),
      }));
      const draft: Draft = {
        id: draftId,
        accountId: prepared.accountId,
        to: "",
        cc: "",
        bcc: "",
        subject: prepared.subject,
        body: prepared.body,
        revision: 1,
        forward: prepared.forward,
        forwardSource: id,
        attachments: files.map((f) => f.info),
      };
      await this.store.commit([
        { store: "drafts", key: draftId, value: draft },
        ...files.map((file) => ({
          store: "draftFiles" as const,
          key: file.info.id,
          value: file,
        })),
      ]);
      this.drafts = [...this.drafts.filter((d) => d.id !== draftId), draft];
      return draft;
    });
  }
  async reply(id: string, all: boolean): Promise<Draft> {
    const record = await resolveMail(this.store, id);
    if (!record)
      throw new Error(
        "The original message is no longer cached. Refresh before replying.",
      );
    return replyDraft(
      record.core,
      record.text,
      envelope(record.reply),
      this.accounts.map((a) => a.email),
      all,
    );
  }
  async saveDraft(draft: Draft) {
    await this.exclusive(`draft.${draft.id}`, async () => {
      const outgoing = await this.store.get<Outgoing>("outgoing", draft.id);
      if (outgoing?.recovery)
        throw new Error(
          "This submission was already reviewed. Open its recovered draft or Sent copy.",
        );
      if (outgoing && outgoing.state === "delivered")
        throw new Error(
          "This draft was already delivered. Close it and compose a new message.",
        );
      if (outgoing && draftKey(outgoing.draft) !== draftKey(draft))
        throw new Error(
          "This draft has a delivery record. Check its status before editing or sending a new message.",
        );
      const current = await this.store.get<Draft>("drafts", draft.id);
      if (current && (current.revision ?? 0) > (draft.revision ?? 0))
        throw new Error(
          "This draft has newer text in another editor. Reopen it before saving.",
        );
      const saved = {
        ...draft,
        forward: current?.forward,
        forwardSource: current?.forwardSource,
        attachments: await this.attachments(draft.id),
      };
      await this.store.commit([
        { store: "drafts", key: draft.id, value: saved },
      ]);
    });
  }
  private receipt(value: unknown, id: string): string {
    check(value);
    if (
      value.id !== id ||
      ![
        "reserved",
        "submitting",
        "delivered",
        "rejected",
        "uncertain",
        "cancelled",
      ].includes(String(value.state))
    )
      throw new Error(
        "Delivery status could not be verified. Check status before sending again.",
      );
    return String(value.state);
  }
  async send(draft: Draft): Promise<void> {
    await this.exclusive(`draft.${draft.id}`, async () => {
      let record = await this.store.get<Outgoing>("outgoing", draft.id);
      if (record) {
        if (record.recovery)
          throw new Error(
            "This submission was already reviewed. Open its recovered draft or Sent copy; it was not resent.",
          );
        if (draftKey(record.draft) !== draftKey(draft))
          throw new Error(
            "This draft already has a delivery record. Reopen the saved draft to check its status.",
          );
        if (record.state === "delivered") return;
        // Once a browser may have submitted, subsequent Send clicks only check
        // status. A lost/expired server reservation cannot create a new send.
        const response = await this.response(`/api/mail/outgoing/${record.id}`);
        if (response.status === 404)
          throw new Error(
            "Delivery status is no longer available. Check Sent or the recipient before composing a new message; this draft was not resent.",
          );
        record.state = this.receipt(await response.json(), record.id);
      } else {
        const account = this.accounts.find((a) => a.id === draft.accountId);
        if (!account) throw new Error("Choose a sending account.");
        const connection = this.connection(account, true);
        const saved = await this.store.get<Draft>("drafts", draft.id);
        if (saved && (saved.revision ?? 0) > (draft.revision ?? 0))
          throw new Error(
            "This draft changed in another editor. Reopen it before sending.",
          );
        if (
          JSON.stringify(saved?.forward ?? null) !==
            JSON.stringify(draft.forward ?? null) ||
          saved?.forwardSource !== draft.forwardSource
        )
          throw new Error(
            "The original forward changed. Reopen the draft before sending.",
          );
        const attachments = await this.files(draft.id);
        if (
          JSON.stringify(attachments.map((f) => f.info)) !==
          JSON.stringify(draft.attachments ?? [])
        )
          throw new Error(
            "The attachments changed. Reopen the draft and review its files.",
          );
        const files = await Promise.all(
          attachments.map(async (f) => ({
            ...f.info,
            data: await base64(f.blob),
          })),
        );
        const reservation = await this.json("/api/mail/outgoing/reserve", {});
        if (
          typeof reservation.id !== "string" ||
          !/^[A-Za-z0-9_-]{43}$/.test(reservation.id) ||
          reservation.state !== "reserved"
        )
          throw new Error(
            "Could not reserve delivery. Your message was not sent.",
          );
        record = {
          id: reservation.id,
          draft: structuredClone(draft),
          state: "preparing",
          account: structuredClone(account),
          sent: { state: "pending" },
        };
        // Save the immutable content and submission ID atomically before POST.
        await this.store.commit([
          { store: "drafts", key: draft.id, value: draft },
          { store: "outgoing", key: draft.id, value: record },
        ]);
        try {
          const prepared = await this.json("/api/mail/outgoing/prepare", {
            id: record.id,
            connection,
            draft: {
              id: draft.id,
              account_id: account.id,
              to: draft.to,
              cc: draft.cc,
              bcc: draft.bcc,
              subject: draft.subject,
              body: draft.body,
              in_reply_to: draft.inReplyTo ?? null,
              references: draft.references ?? [],
              forward: draft.forward ?? null,
              revision: draft.revision ?? 0,
            },
            files: files.map(({ size, ...file }) => file),
          });
          if (prepared.id !== record.id || prepared.state !== "reserved")
            throw new Error("Invalid prepared message identity.");
          check(prepared.wire);
          check(prepared.wire.envelope);
          check(prepared.mail);
          const wire = prepared.wire;
          check(wire.envelope);
          if (
            typeof wire.raw !== "string" ||
            !wire.raw.length ||
            wire.raw.length > Math.ceil((25 * 1024 * 1024) / 3) * 4 ||
            typeof wire.envelope.from !== "string" ||
            !Array.isArray(wire.envelope.to) ||
            !wire.envelope.to.every((v) => typeof v === "string")
          )
            throw new Error("Invalid prepared message.");
          const local = prepared.mail as unknown as RecordMail;
          if (
            !local.local ||
            local.core.account_id !== account.id ||
            local.core.folder !== "Sent" ||
            local.core.remote_id !== `local-sent-${record.id}` ||
            local.core.id !== `${account.id}:Sent:local-sent-${record.id}` ||
            typeof local.text !== "string"
          )
            throw new Error("Invalid prepared Sent copy.");
          record.wire = wire as unknown as PreparedWire;
          record.mail = local;
          record.state = "submitting";
          // Exact MIME must commit before SMTP. Failure here leaves only the
          // unused reservation; a status check never starts it automatically.
          await this.store.commit([
            { store: "outgoing", key: draft.id, value: record },
          ]);
          const response = await this.response("/api/mail/send", {
            id: record.id,
            connection,
            wire: record.wire,
          });
          record.state = this.receipt(await response.json(), record.id);
        } catch {
          throw new Error(
            "Delivery is not confirmed. Your draft and submission ID are saved; use Check delivery status before trying again.",
          );
        }
      }
      await this.store.commit([
        { store: "outgoing", key: draft.id, value: record },
        ...(record.state === "delivered"
          ? [{ store: "drafts" as const, key: draft.id }]
          : []),
      ]);
      if (record.state === "delivered" && record.mail && record.wire) {
        try {
          await this.finishSentLocal(record);
        } catch {
          throw new Error(
            "Delivery is confirmed, but the local Sent cache needs repair. Open Outbox to finish; do not send again.",
          );
        }
      }
      if (record.state !== "delivered")
        throw new Error(
          record.state === "rejected"
            ? "SMTP rejected this message. It was not sent. Correct the account or recipients in a new draft; this delivery record is kept."
            : record.state === "reserved"
              ? "The server has not started this submission. Its reservation is kept; this draft was not resent."
              : "Delivery remains unconfirmed. Check Sent or the recipient before composing a new message. This draft was not resent.",
        );
    });
    // Independent Sent work cannot make acknowledged SMTP depend on another
    // cache read. Its recovery record remains visible in Outbox on failure.
    void this.store
      .get<Outgoing>("outgoing", draft.id)
      .then((delivered) => {
        if (
          delivered?.state === "delivered" &&
          delivered.account &&
          !delivered.recovery
        )
          return this.recoverSent(delivered.id, "copy", false, true);
      })
      .catch((error) => {
        this.warning =
          error instanceof Error
            ? error.message
            : "Sent needs attention. Open Outbox to retry.";
      });
  }
  async saveSentPreferences(
    id: string,
    policy: Account["sent_copy"],
    folder: string,
  ) {
    if (
      !["Automatic", "ServerManaged", "LocalOnly"].includes(policy) ||
      folder.length > 1024 ||
      /[\r\n\0]/.test(folder)
    )
      throw new Error("Choose a valid Sent policy and folder.");
    await this.exclusive(`account.${id}`, async () => {
      const current = await this.store.get<Account>("accounts", id);
      if (!current)
        throw new Error("This account was removed. Reopen Preferences.");
      const value = {
        ...current,
        sent_copy: policy,
        sent_folder: folder.trim(),
      };
      await this.store.commit([{ store: "accounts", key: id, value }]);
      this.accounts = this.accounts.map((a) => (a.id === id ? value : a));
    });
    await this.reloadMail();
  }
  private async finishSentLocal(record: Outgoing) {
    if (!record.mail || !record.wire)
      throw new Error(
        "The original Sent message is unavailable. Keep this delivery record.",
      );
    const original = record.mail,
      wire = record.wire;
    await this.exclusive(`cache.${original.core.account_id}`, async () => {
      const id = original.core.id;
      const existing = await this.store.get<RecordMail>("mail", id);
      const changes: Change[] = [{ store: "drafts", key: record.draft.id }];
      if (!existing)
        changes.push(
          { store: "mail", key: id, value: original },
          { store: "raw", key: id, value: wire.raw },
        );
      if (record.sent?.state === "saved" && record.sent.receipt) {
        changes.push(
          await acknowledgeRole(
            this.store,
            original.core.account_id,
            record.sent.receipt.folder,
          ),
        );
        const candidates: string[] = [];
        for await (const candidate of this.mailEntries({
          account: original.core.account_id,
          folder: record.sent.receipt.folder,
        })) {
          if (
            !candidate.local &&
            candidate.sentMessageId === `<${record.id}@shep.so>`
          )
            candidates.push(candidate.key);
          if (candidates.length > 1) break;
        }
        const incoming =
          candidates.length === 1
            ? await this.store.get<RecordMail>("mail", candidates[0])
            : undefined;
        if (incoming) {
          const raw = await this.store.get<string>("raw", localId(incoming));
          if (raw)
            changes.push(
              ...(await insertSentCandidate(
                this.store,
                incoming,
                raw,
                existing ?? original,
              )),
            );
        }
      }
      await this.store.commit(changes);
    });
    await this.reloadMail();
  }
  async recoverSent(
    id: string,
    action: "check" | "copy",
    confirmed = false,
    automatic = false,
  ) {
    const initial = (await this.store.all<Outgoing>("outgoing")).find(
      (r) => r.id === id,
    );
    if (!initial)
      throw new Error("This Outbox entry is unavailable. Refresh Outbox.");
    await this.exclusive(`draft.${initial.draft.id}`, async () => {
      const record = await this.store.get<Outgoing>(
        "outgoing",
        initial.draft.id,
      );
      if (!record || record.id !== id)
        throw new Error("This Outbox entry changed. Refresh Outbox.");
      await this.exclusive(`account.${record.draft.accountId}`, async () => {
        try {
          await recoverSent(
            {
              store: this.store,
              pending: this.sentAcknowledgments,
              response: (path, body) => this.response(path, body),
              json: (path, body) => this.json(path, body),
              connection: (account) => this.connection(account),
              finishLocal: (record) => this.finishSentLocal(record),
            },
            record,
            action,
            confirmed,
            automatic,
          );
          // Read the committed result again: an acknowledgment may still be
          // pending after a storage failure and must not be hidden by metadata.
          const current = await this.store.get<Outgoing>(
            "outgoing",
            record.draft.id,
          );
          if (current?.sentError) {
            delete current.sentError;
            await this.store.commit([
              { store: "outgoing", key: current.draft.id, value: current },
            ]);
          }
        } catch (error) {
          const message =
            error instanceof Error
              ? error.message
              : "Could not finish Sent. Retry from Outbox.";
          this.warning = message;
          const current = await this.store
            .get<Outgoing>("outgoing", record.draft.id)
            .catch(() => undefined);
          if (current) {
            current.sentError = message;
            await this.store
              .commit([
                { store: "outgoing", key: current.draft.id, value: current },
              ])
              .catch(() => {});
          }
          throw error;
        }
      });
    });
  }
  async outgoing(): Promise<Outgoing[]> {
    return (await this.store.all<Outgoing>("outgoing")).filter(
      (r) =>
        !r.recovery ||
        (r.recovery.action === "marked" &&
          r.sent?.state !== "saved" &&
          r.sent?.state !== "local"),
    );
  }
  async recoverOutgoing(
    id: string,
    action: "check" | "return" | "mark" | "local",
    confirmed = false,
  ): Promise<Draft | undefined> {
    const records = await this.store.all<Outgoing>("outgoing");
    const initial = records.find((r) => r.id === id);
    if (!initial)
      throw new Error(
        "This Outbox entry is no longer available. Refresh Outbox.",
      );
    return this.exclusive(`draft.${initial.draft.id}`, async () => {
      const record = await this.store.get<Outgoing>(
        "outgoing",
        initial.draft.id,
      );
      if (!record || record.id !== id)
        throw new Error("This Outbox entry changed. Refresh Outbox.");
      if (record.sent?.state === "saved" && action === "return")
        throw new Error(
          "A matching provider Sent copy is acknowledged. Keep it instead of returning this message to drafts.",
        );
      if (record.recovery && record.recovery.action !== "marked") {
        if (record.recovery.draftId) {
          const draft = await this.store.get<Draft>(
            "drafts",
            record.recovery.draftId,
          );
          if (draft)
            return { ...draft, attachments: await this.attachments(draft.id) };
        }
        return;
      }
      if (
        action !== "local" &&
        !["delivered", "rejected", "cancelled"].includes(record.state)
      ) {
        // A stored terminal receipt remains authoritative after the server's
        // transient receipt expires or restarts. Never downgrade known delivery.
        // Cancellation, unlike a GET of 'reserved', closes the race with a
        // delayed send POST. A running SMTP task cannot be cancelled/released.
        const response = await this.response(
          `/api/mail/outgoing/${id}${action === "check" ? "" : "/cancel"}`,
          action === "check" ? undefined : {},
        );
        record.state =
          response.status === 404
            ? "unknown"
            : this.receipt(await response.json(), id);
        await this.store.commit([
          { store: "outgoing", key: record.draft.id, value: record },
        ]);
      }
      if (action === "check" && record.state !== "delivered") return;
      if (record.state === "submitting")
        throw new Error(
          "SMTP is still running. Check status before making a recovery decision.",
        );
      if (action === "return") {
        if (
          !["rejected", "cancelled", "uncertain", "unknown"].includes(
            record.state,
          )
        )
          throw new Error(
            "This message was delivered or its reservation is still open. Check Outbox before returning it to drafts.",
          );
        if (["uncertain", "unknown"].includes(record.state) && !confirmed)
          throw new Error(
            "Review delivery and confirm that another send could create a duplicate.",
          );
        const draft = {
          ...structuredClone(record.draft),
          id: crypto.randomUUID(),
          revision: 0,
          attachments: [] as DraftAttachment[],
        };
        const files = await this.files(record.draft.id);
        const changes: Change[] = [];
        for (const saved of files) {
          const file = {
            ...saved,
            draftId: draft.id,
            info: { ...saved.info, id: crypto.randomUUID() },
          };
          draft.attachments.push(file.info);
          changes.push({ store: "draftFiles", key: file.info.id, value: file });
        }
        record.recovery = { action: "returned", draftId: draft.id };
        changes.push(
          { store: "drafts", key: record.draft.id },
          { store: "drafts", key: draft.id, value: draft },
          { store: "outgoing", key: record.draft.id, value: record },
        );
        await this.store.commit(changes);
        return draft;
      }
      if (action === "mark") {
        if (
          !["uncertain", "unknown", "delivered"].includes(record.state) ||
          !confirmed
        )
          throw new Error(
            "Confirm your delivery review before recording this message as sent.",
          );
      } else if (
        record.state !== "delivered" &&
        record.sent?.state !== "saved" &&
        record.recovery?.action !== "marked"
      )
        throw new Error(
          "Delivery has not been confirmed. Review it before keeping a Sent copy.",
        );
      if (!record.mail || !record.wire)
        throw new Error(
          "This older submission has no exact saved MIME. Its original draft is retained for review.",
        );
      if (action !== "check")
        record.recovery = { action: action === "mark" ? "marked" : "local" };
      await this.store.commit([
        { store: "outgoing", key: record.draft.id, value: record },
      ]);
      await this.finishSentLocal(record);
    });
  }
  async delivery(draft: Draft) {
    return this.store.get<Outgoing>("outgoing", draft.id);
  }
  async saveEvent(): Promise<void> {
    throw new Error("Calendar providers are not connected in this beta yet.");
  }
}
