import { MoveFeedback, MoveRecord } from "./move-feedback";
import { mailMatches } from "./mail_query";
import { MailSelection } from "./mail_selection";
import type { SelectionRepository, SelectionScope } from "./selection_types";
import type { BulkReceipt } from "./bulk_journal";
import type { IntentLease } from "./mail_intents";
export interface Mail {
  id: string;
  sender: string;
  address: string;
  subject: string;
  preview: string;
  body: string;
  date: string;
  account: string;
  folder: string;
  unread: boolean;
  starred: boolean;
  attachments: string[];
  accountId?: string;
}
export interface CalendarEntry {
  id: string;
  title: string;
  start: string;
  end: string;
  calendar: string;
  location: string;
  readOnly: boolean;
}
export interface ForwardQuote {
  text: string;
  html_head: string;
  html_attributes: string;
  html_body: string;
}
export interface DraftAttachment {
  id: string;
  name: string;
  media_type: string;
  size: number;
  content_id?: string | null;
}
export interface Draft {
  id: string;
  accountId?: string;
  forward?: ForwardQuote | null;
  forwardSource?: string;
  inReplyTo?: string | null;
  references?: string[];
  revision?: number;
  attachments?: DraftAttachment[];
  to: string;
  cc: string;
  bcc: string;
  subject: string;
  body: string;
}
export type Fields = Partial<Pick<Mail, "unread" | "starred" | "folder">>;
export type Action = "archive" | "trash" | "read" | "star" | "move";
export class MutationFailure extends Error {
  constructor(
    message: string,
    public committed = false,
    public receipt?: BulkReceipt,
    public cacheApplied = false,
    public applied?: Fields,
  ) {
    super(message);
  }
}
export interface Repository {
  preview: boolean;
  cached: Mail[];
  events: CalendarEntry[];
  drafts?: Draft[];
  warning?: string | null;
  aliases?: Map<string, string>;
  folderRoles?: Map<string, Set<string>>;
  removedAccounts?: Set<string>;
  accountIds?: Map<string, string>;
  selection?: SelectionRepository["selection"];
  closeSelections?(): Promise<void>;
  refresh(folder?: string, account?: string | null): Promise<Mail[]>;
  registerMutation?(
    id: string,
    fields: Fields,
  ): Promise<IntentLease | undefined>;
  cancelMutation?(lease: IntentLease): Promise<void>;
  mutate(
    id: string,
    fields: Fields,
    lease?: IntentLease,
  ): Promise<Fields | void>;
  saveDraft(draft: Draft): Promise<void>;
  send(draft: Draft): Promise<void>;
  saveEvent(event: CalendarEntry): Promise<void>;
}
export interface Preferences {
  appearance: "system" | "light" | "dark";
  previewLines: number;
  avatars: boolean;
  quoteMode: "Collapsed" | "Expanded" | "Latest only";
  sidebarWidth: number;
  listWidth: number;
  shortcuts: Record<
    | "archive"
    | "trash"
    | "move"
    | "reply"
    | "forward"
    | "print"
    | "search"
    | "reader"
    | "find"
    | "selectAll",
    string
  >;
}
export const defaults: Preferences = {
  appearance: "system",
  previewLines: 2,
  avatars: true,
  quoteMode: "Collapsed",
  sidebarWidth: 218,
  listWidth: 370,
  shortcuts: {
    archive: "Backspace",
    trash: "Control+d",
    move: "m",
    reply: "r",
    forward: "f",
    print: "Control+p",
    search: "Control+k",
    reader: "Enter",
    find: "Control+f",
    selectAll: "Control+a",
  },
};
export interface SettingsStore {
  read(): Preferences;
  write(value: Preferences): void;
}
export class BrowserSettings implements SettingsStore {
  constructor(private key = "shep.preferences.v1") {}
  read(): Preferences {
    const raw = localStorage.getItem(this.key);
    if (!raw) return structuredClone(defaults);
    const p = JSON.parse(raw);
    if (p.version !== 1) throw new Error("Unknown preferences version");
    return {
      appearance: ["system", "light", "dark"].includes(p.appearance)
        ? p.appearance
        : "system",
      previewLines: Number.isInteger(p.previewLines)
        ? Math.max(0, Math.min(4, p.previewLines))
        : 2,
      avatars: typeof p.avatars === "boolean" ? p.avatars : true,
      quoteMode: ["Collapsed", "Expanded", "Latest only"].includes(p.quoteMode)
        ? p.quoteMode
        : "Collapsed",
      sidebarWidth: Number.isFinite(p.sidebarWidth)
        ? Math.max(180, Math.min(320, p.sidebarWidth))
        : 218,
      listWidth: Number.isFinite(p.listWidth)
        ? Math.max(280, Math.min(560, p.listWidth))
        : 370,
      shortcuts: Object.fromEntries(
        Object.entries(defaults.shortcuts).map(([key, value]) => [
          key,
          typeof p.shortcuts?.[key] === "string"
            ? p.shortcuts[key]
            : Object.values(p.shortcuts ?? {}).includes(value)
              ? ""
              : value,
        ]),
      ) as Preferences["shortcuts"],
    };
  }
  write(value: Preferences) {
    localStorage.setItem(this.key, JSON.stringify({ ...value, version: 1 }));
  }
}
export class UnconnectedRepository implements Repository {
  preview = false;
  cached: Mail[] = [];
  events: CalendarEntry[] = [];
  async refresh(): Promise<Mail[]> {
    throw new Error("Connect a supported provider to refresh.");
  }
  async mutate(): Promise<void> {
    throw new Error("No connected provider.");
  }
  async saveDraft(): Promise<void> {
    throw new Error("Draft storage has not been connected.");
  }
  async send(): Promise<void> {
    throw new Error("Connect a provider before sending.");
  }
  async saveEvent(): Promise<void> {
    throw new Error("Connect a calendar before saving.");
  }
}
export class Workspace extends EventTarget {
  mail: Mail[];
  events: CalendarEntry[];
  preferences: Preferences = structuredClone(defaults);
  folder = "Inbox";
  account: string | null = null;
  filter = "All";
  query = "";
  newestFirst = true;
  page = 0;
  private readCandidate: Mail | null = null;
  beginReading(id: string) {
    id = this.canonical(id);
    const mail = this.message(id);
    if (!mail) return;
    if (!this.readCandidate || this.canonical(this.readCandidate.id) !== id) {
      void this.finishReading();
      if (mail.unread) this.readCandidate = structuredClone(mail);
    }
    this.selected = id;
    this.changed();
  }
  async finishReading() {
    const candidate = this.readCandidate;
    this.readCandidate = null;
    if (!candidate) return;
    const id = this.canonical(candidate.id);
    if ((this.message(id) ?? this.confirmed.get(id))?.unread) {
      await this.change(id, { unread: false }, false, true);
    }
  }
  private selectedId: string | null = null;
  private retainedReader: Mail | null = null;
  get selected() {
    return this.selectedId;
  }
  set selected(id: string | null) {
    this.selectedId = id === null ? null : this.canonical(id);
    this.retainedReader =
      id === null
        ? null
        : structuredClone(
            this.mail.find((m) => m.id === this.selectedId) ??
              this.retainedReader,
          );
  }
  get readerMessage(): Mail | null {
    return (
      this.mail.find((m) => m.id === this.selectedId) ??
      (this.retainedReader?.id === this.selectedId ? this.retainedReader : null)
    );
  }
  readonly selection: MailSelection;
  drafts = new Map<string, Draft>();
  private removedAccountIds = new Set<string>();
  private errorValue: string | null = null;
  private undoErrorOwner?: MoveRecord;
  get error() {
    return this.errorValue;
  }
  set error(value: string | null) {
    this.errorValue = value;
    this.undoErrorOwner = undefined;
  }
  private statusNotice: string | null = null;
  get notice() {
    return this.statusNotice;
  }
  set notice(value: string | null) {
    this.statusNotice = value;
  }
  readonly moves = new MoveFeedback(() => this.changed());
  readonly undoFailures = new Set<MoveRecord>();
  private flagUndo: (() => void) | null = null;
  private flagUndoRevision = 0;
  private undoSnapshot?: MoveRecord[];
  private moveUndo?: () => void;
  get undo(): (() => void) | null {
    if (!this.moves.visible) return this.flagUndo;
    if (!this.moves.canUndo) return null;
    if (this.undoSnapshot !== this.moves.records) {
      const snapshot = this.moves.records;
      this.undoSnapshot = snapshot;
      this.moveUndo = () => this.undoMoves(snapshot);
    }
    return this.moveUndo!;
  }
  undoMoves(expected: MoveRecord[]) {
    for (const record of this.moves.restore(expected))
      void this.change(
        record.id,
        { folder: record.originalFolder },
        false,
        true,
        record,
      );
    this.changed();
  }
  dismissUndoFailures() {
    this.undoFailures.clear();
    this.changed();
  }
  retryUndos() {
    for (const record of [...this.undoFailures].filter(
      (r) => !r.restoreCommitted,
    )) {
      this.undoFailures.delete(record);
      this.moves.retryRestore(record);
      void this.change(
        record.id,
        { folder: record.originalFolder },
        false,
        true,
        record,
        true,
      );
    }
    this.changed();
  }
  async refreshRestored() {
    const reviewing = [...this.undoFailures].filter((r) => r.restoreCommitted);
    await this.refresh();
    for (const record of reviewing) {
      const saved = this.repository.cached.find(
        (m) => m.id === this.canonical(record.id),
      );
      if (saved?.folder === record.originalFolder)
        this.undoFailures.delete(record);
    }
    this.changed();
  }
  dispose() {
    this.moves.dispose();
    this.selection.dispose();
    void this.repository.closeSelections?.().catch(() => {});
  }
  retry: (() => void) | null = null;
  syncing = false;
  private refreshAgain = false;
  private revision = 0;
  private confirmed = new Map<string, Mail>();
  private versions = new Map<string, number>();
  private queues = new Map<string, Promise<void>>();
  private aliases = new Map<string, string>();
  private canonical(id: string) {
    return this.aliases.get(id) ?? id;
  }
  private isPending(id: string) {
    return [...this.queues.keys()].some((key) => this.canonical(key) === id);
  }
  private message(id: string) {
    return (
      this.mail.find((m) => m.id === this.canonical(id)) ??
      (this.readerMessage?.id === this.canonical(id)
        ? this.readerMessage
        : undefined)
    );
  }
  private paint(id: string, fields: Fields) {
    this.mail = this.mail.map((m) => (m.id === id ? { ...m, ...fields } : m));
    if (this.retainedReader?.id === id)
      this.retainedReader = { ...this.retainedReader, ...fields };
  }
  private acceptAliases(messages: Mail[], since: number) {
    const incomingAliases =
      this.repository.aliases ?? new Map<string, string>();
    const targets = new Set(
      [...incomingAliases]
        .filter(([alias, target]) => this.aliases.get(alias) !== target)
        .map(([, target]) => target),
    );
    if (!targets.size) return;
    const old = this.mail;
    for (const [alias, target] of incomingAliases) {
      for (const [key, previous] of this.aliases)
        if (previous === alias) this.aliases.set(key, target);
      this.aliases.set(alias, target);
    }
    const fields = ["folder", "unread", "starred"] as const;
    const versions = new Map<string, number>();
    const overlays = new Map<string, { revision: number; value: unknown }>();
    for (const [key, version] of this.versions) {
      const colon = key.lastIndexOf(":");
      const next = `${this.canonical(key.slice(0, colon))}${key.slice(colon)}`;
      if ((versions.get(next) ?? -1) < version) versions.set(next, version);
    }
    for (const m of old)
      for (const field of fields) {
        const version = this.versions.get(`${m.id}:${field}`);
        const id = this.canonical(m.id),
          key = `${id}:${field}`;
        if (
          version !== undefined &&
          (version > since || this.isPending(id)) &&
          (overlays.get(key)?.revision ?? -1) < version
        )
          overlays.set(key, { revision: version, value: m[field] });
      }
    this.versions = versions;
    const confirmed = new Map<string, Mail>();
    for (const [id, m] of this.confirmed)
      confirmed.set(this.canonical(id), { ...m, id: this.canonical(id) });
    const incoming = new Map(messages.map((m) => [m.id, m]));
    const merged = new Map<string, Mail>();
    for (const oldMail of old) {
      const id = this.canonical(oldMail.id);
      const source = targets.has(id) ? (incoming.get(id) ?? oldMail) : oldMail;
      const next = { ...source, id };
      if (targets.has(id) && incoming.has(id))
        confirmed.set(id, structuredClone(incoming.get(id)!));
      for (const field of fields)
        if (overlays.has(`${id}:${field}`))
          Object.assign(next, {
            [field]: overlays.get(`${id}:${field}`)!.value,
          });
      merged.set(id, next);
    }
    this.confirmed = confirmed;
    this.mail = [...merged.values()];
    if (this.selectedId) this.selectedId = this.canonical(this.selectedId);
    if (this.retainedReader)
      this.retainedReader = this.mail.find(
        (m) => m.id === this.canonical(this.retainedReader!.id),
      ) ?? {
        ...this.retainedReader,
        id: this.canonical(this.retainedReader.id),
      };
  }
  constructor(
    public repository: Repository,
    private settings: SettingsStore,
  ) {
    super();
    this.selection = new MailSelection(
      {
        selection: (command, observed) =>
          repository.selection
            ? repository.selection(command, observed)
            : Promise.reject(
                Error(
                  "Selection storage is unavailable. Reopen Shep and retry.",
                ),
              ),
      },
      () => this.selectionScope(),
      () => this.matching.length,
      () => this.changed(),
    );
    this.mail = structuredClone(repository.cached);
    this.aliases = new Map(repository.aliases ?? []);
    this.events = structuredClone(repository.events);
    this.drafts = new Map(
      (repository.drafts ?? []).map((d) => [d.id, structuredClone(d)]),
    );
    this.mail.forEach((m) => this.confirmed.set(m.id, structuredClone(m)));
    try {
      this.preferences = settings.read();
    } catch {
      this.error =
        "Could not load preferences. Saved data was kept; reopen the app to retry.";
    }
  }
  get pending() {
    return this.queues.size;
  }
  private selectionScope(): SelectionScope {
    const projection: Record<string, Fields> = {};
    for (const id of this.queues.keys()) {
      const m = this.message(this.canonical(id));
      if (m)
        projection[m.id] = {
          folder: m.folder,
          unread: m.unread,
          starred: m.starred,
        };
    }
    return {
      folder: this.folder,
      account: this.account
        ? (this.repository.accountIds?.get(this.account) ?? this.account)
        : null,
      query: this.query,
      filter: this.filter,
      oldest: !this.newestFirst,
      projection,
    };
  }
  get unread() {
    return this.mail.filter((m) => m.folder === "Inbox" && m.unread).length;
  }
  get matching() {
    const scope = {
      folder: this.folder,
      query: this.query,
      filter: this.filter,
    };
    return this.mail
      .filter(
        (m) =>
          (!this.account || m.account === this.account) &&
          mailMatches(
            m,
            scope,
            this.repository.folderRoles?.get(m.accountId ?? m.account),
          ),
      )
      .sort(
        (a, b) =>
          (this.newestFirst
            ? b.date.localeCompare(a.date)
            : a.date.localeCompare(b.date)) ||
          (a.id < b.id ? -1 : a.id > b.id ? 1 : 0),
      );
  }
  get visible() {
    return this.matching.slice(this.page * 50, (this.page + 1) * 50);
  }
  changed() {
    this.selection.reconcileScope();
    this.dispatchEvent(new Event("change"));
  }
  navigate(folder: string, account: string | null = null) {
    void this.finishReading();
    this.folder = folder;
    this.account = account;
    this.page = 0;
    this.selection.done(false);
    this.selected = null;
    this.changed();
  }
  search(value: string) {
    void this.finishReading();
    this.query = value;
    this.page = 0;
    this.selection.done(false);
    this.changed();
  }
  savePreferences(value: Preferences) {
    this.preferences = value;
    try {
      this.settings.write(value);
      this.notice = "Preferences saved";
      this.error = null;
    } catch {
      this.error =
        "Preferences could not be saved. Retry to keep changes after restarting.";
      this.retry = () => this.savePreferences(this.preferences);
    }
    this.changed();
  }
  addCachedMail(messages: Mail[]) {
    this.acceptAliases(messages, this.revision);
    // A newly saved Sent copy must not overwrite other pending mail actions or
    // disappear behind a refresh started before the submission completed.
    const known = new Set(this.mail.map((m) => m.id));
    for (const message of messages)
      if (
        !known.has(message.id) &&
        !this.removedAccountIds.has(message.accountId ?? "")
      ) {
        const saved = structuredClone(message);
        this.mail.push(saved);
        this.confirmed.set(saved.id, structuredClone(saved));
        known.add(saved.id);
        this.revision++;
      }
    this.selection.refresh();
  }
  rememberDraft(draft: Draft) {
    if (draft.accountId && this.removedAccountIds.has(draft.accountId))
      throw new Error(
        "This account was removed. Copy this text into a new draft with a connected account.",
      );
    this.drafts.set(draft.id, structuredClone(draft));
  }
  accountRemoved(id: string) {
    if (this.removedAccountIds.has(id)) return;
    this.removedAccountIds.add(id);
    this.moves.removeAccount(id);
    for (const record of this.undoFailures)
      if (record.account === id) this.undoFailures.delete(record);
    if (this.readCandidate?.accountId === id) this.readCandidate = null;
    for (const [key, draft] of this.drafts)
      if (draft.accountId === id) this.drafts.delete(key);
    if (this.readerMessage?.accountId === id) this.selected = null;
    this.mail = this.mail.filter((m) => m.accountId !== id);
    for (const [key, m] of this.confirmed)
      if (m.accountId === id) this.confirmed.delete(key);
    this.account = null;
    this.folder = "Inbox";
    this.page = 0;
    this.flagUndo = null;
    this.revision++;
    this.selection.refresh();
    this.changed();
  }
  async refresh() {
    if (this.syncing) {
      this.refreshAgain = true;
      this.notice = "Refresh queued";
      this.changed();
      return;
    }
    this.syncing = true;
    this.changed();
    do {
      this.refreshAgain = false;
      let rev = this.revision;
      try {
        const result = await this.repository.refresh(this.folder, this.account);
        for (const id of this.repository.removedAccounts ?? []) {
          const unchanged = rev === this.revision;
          this.accountRemoved(id);
          if (unchanged) rev = this.revision;
        }
        this.acceptAliases(result, rev);
        if (rev === this.revision && !this.pending) {
          if (this.readerMessage)
            this.retainedReader = structuredClone(this.readerMessage);
          this.mail = result;
          result.forEach((m) => this.confirmed.set(m.id, structuredClone(m)));
        }
        this.error = this.repository.warning ?? null;
        this.retry = this.error ? () => void this.refresh() : null;
        this.notice = this.error
          ? null
          : this.repository.preview
            ? "Preview refreshed"
            : "Mail refreshed";
      } catch (error) {
        this.error =
          error instanceof Error
            ? error.message
            : "Could not refresh. Cached mail is still available; retry.";
        this.retry = () => void this.refresh();
      }
    } while (this.refreshAgain);
    this.syncing = false;
    this.selection.refresh();
    this.changed();
  }
  action(id: string, action: Action, destination?: string) {
    id = this.canonical(id);
    const m = this.message(id);
    if (!m) return Promise.resolve();
    const fields: Fields =
      action === "archive"
        ? { folder: "Archive" }
        : action === "trash"
          ? { folder: "Trash" }
          : action === "read"
            ? { unread: !m.unread }
            : action === "star"
              ? { starred: !m.starred }
              : destination
                ? { folder: destination }
                : {};
    return this.change(id, fields);
  }
  async change(
    id: string,
    fields: Fields,
    offerUndo = true,
    quiet = false,
    restoring?: MoveRecord,
    force = false,
  ) {
    id = this.canonical(id);
    if (
      fields.folder &&
      this.readCandidate &&
      this.canonical(this.readCandidate.id) === id
    )
      void this.finishReading();
    if (
      !quiet &&
      fields.unread !== undefined &&
      this.readCandidate &&
      this.canonical(this.readCandidate.id) === id
    )
      this.readCandidate = null;
    const current =
      this.message(id) ?? (!offerUndo ? this.confirmed.get(id) : undefined);
    if (!current || !Object.keys(fields).length) return;
    if (
      !force &&
      Object.entries(fields).every(
        ([key, value]) => current[key as keyof Mail] === value,
      )
    )
      return;
    const move =
      fields.folder && offerUndo
        ? this.moves.add(
            id,
            current.accountId || current.account,
            current.folder,
            fields.folder,
          )
        : undefined;
    const previous = Object.fromEntries(
      Object.keys(fields).map((key) => [key, current[key as keyof Mail]]),
    ) as Fields;
    const revision = ++this.revision;
    for (const key of Object.keys(fields))
      this.versions.set(`${id}:${key}`, revision);
    this.paint(id, fields);
    if (move) {
      this.flagUndo = null;
      this.statusNotice = null;
    } else if (offerUndo) {
      this.flagUndoRevision = revision;
      this.flagUndo = () => {
        if (this.flagUndoRevision !== revision) return;
        this.flagUndo = null;
        void this.change(id, previous, false);
      };
    }
    if (!quiet) {
      if (!move) this.statusNotice = "Message updated";
      this.error = null;
      this.retry = null;
    }
    // Reserve in input order, before waiting for older provider work. Consume a
    // rejected reservation immediately; report it through the ordered action.
    const reservation = Promise.resolve(
      this.repository.registerMutation?.(id, fields),
    ).then(
      (lease) => ({ lease }),
      (error) => ({ error }),
    );
    const accept = (applied: Fields) => {
      this.acceptAliases(this.repository.cached, revision);
      const key = this.canonical(id),
        saved = this.repository.cached.find((m) => m.id === key);
      const superseded = Object.fromEntries(
        Object.keys(fields)
          .filter((field) => !(field in applied) && saved)
          .map((field) => [field, saved![field as keyof Mail]]),
      ) as Fields;
      this.confirmed.set(key, {
        ...this.confirmed.get(key)!,
        ...superseded,
        ...applied,
      });
      this.paint(
        key,
        Object.fromEntries(
          Object.entries(superseded).filter(
            ([field]) => this.versions.get(`${key}:${field}`) === revision,
          ),
        ) as Fields,
      );
      if (fields.folder && applied.folder === undefined) {
        if (move) this.moves.failed(move);
        if (restoring) this.moves.failed(restoring);
      }
      if (!Object.keys(applied).length && this.flagUndoRevision === revision) {
        this.flagUndo = null;
        this.statusNotice = null;
      }
    };
    const previousJobs = [...this.queues]
      .filter(([key]) => this.canonical(key) === id)
      .map(([, job]) => job);
    const job = Promise.all(previousJobs).then(async () => {
      try {
        const registered = await reservation;
        if (!this.confirmed.has(this.canonical(id))) return;
        if ("error" in registered) throw registered.error;
        if (move?.cancelled || (restoring && !restoring.committed)) {
          if (registered.lease)
            await this.repository.cancelMutation?.(registered.lease);
          return;
        }
        if (move) move.started = true;
        const applied =
          (await this.repository.mutate(
            this.canonical(id),
            fields,
            registered.lease,
          )) ?? fields;
        if (move) move.committed = applied.folder !== undefined;
        if (restoring) {
          this.undoFailures.delete(restoring);
          if (this.undoErrorOwner === restoring) {
            this.error = null;
            this.retry = null;
          }
        }
        if (!this.confirmed.has(this.canonical(id))) return;
        accept(applied);
      } catch (error) {
        if (!this.confirmed.has(this.canonical(id))) return;
        const acknowledged =
          error instanceof MutationFailure && error.committed;
        if (move && !move.undoRequested && !acknowledged)
          this.moves.failed(move);
        if (restoring) {
          if (!acknowledged) this.moves.failed(restoring);
          this.undoFailures.add(restoring);
        }
        if (error instanceof MutationFailure && error.committed) {
          if (restoring) restoring.restoreCommitted = true;
          accept(error.applied ?? fields);
          if (move) {
            move.committed = true;
            move.blocked =
              !error.cacheApplied ||
              !error.receipt ||
              !!(error.receipt.recovery && !error.receipt.after.remoteId);
          }
          this.error = error.message;
          if (restoring) this.undoErrorOwner = restoring;
          if (
            !error.cacheApplied &&
            !quiet &&
            !move &&
            this.flagUndoRevision === revision
          ) {
            this.notice = null;
            this.flagUndo = null;
          }
          this.retry = () => void this.refresh();
          return;
        }
        const rollback = Object.fromEntries(
          Object.keys(fields)
            .filter(
              (key) =>
                this.versions.get(`${this.canonical(id)}:${key}`) === revision,
            )
            .map((key) => [
              key,
              this.confirmed.get(this.canonical(id))![key as keyof Mail],
            ]),
        ) as Fields;
        this.paint(this.canonical(id), rollback);
        if (Object.keys(rollback).length || move || restoring) {
          this.error = `Could not confirm the update to ${current.subject}. The affected display was restored. ${error instanceof Error ? error.message : "The affected change was restored. Retry."}`;
          if (restoring) this.undoErrorOwner = restoring;
          if (!quiet && !move && this.flagUndoRevision === revision) {
            this.notice = null;
            this.flagUndo = null;
          }
          this.retry =
            error instanceof MutationFailure
              ? () => void this.refresh()
              : () =>
                  void this.change(
                    id,
                    fields,
                    !quiet,
                    quiet,
                    restoring,
                    !!restoring,
                  );
        }
      }
    });
    this.queues.set(id, job);
    this.changed();
    await job;
    if (this.queues.get(id) === job) this.queues.delete(id);
    this.selection.refresh();
    this.changed();
  }
}
