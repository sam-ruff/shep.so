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
export interface DraftAttachment {
  id: string;
  name: string;
  media_type: string;
  size: number;
}
export interface Draft {
  id: string;
  accountId?: string;
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
  refresh(folder?: string, account?: string | null): Promise<Mail[]>;
  mutate(id: string, fields: Fields): Promise<void>;
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
    "archive" | "trash" | "move" | "reply" | "search" | "reader",
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
    search: "Control+k",
    reader: "Enter",
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
          typeof p.shortcuts?.[key] === "string" ? p.shortcuts[key] : value,
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
  selection = new Set<string>();
  drafts = new Map<string, Draft>();
  error: string | null = null;
  notice: string | null = null;
  undo: (() => void) | null = null;
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
    this.selection = new Set(
      [...this.selection].map((id) => this.canonical(id)),
    );
  }
  constructor(
    public repository: Repository,
    private settings: SettingsStore,
  ) {
    super();
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
  get unread() {
    return this.mail.filter((m) => m.folder === "Inbox" && m.unread).length;
  }
  get matching() {
    const words = this.query.toLowerCase().trim().split(/\s+/);
    return this.mail
      .filter(
        (m) =>
          (m.folder === this.folder ||
            (this.folder === "Sent" &&
              this.repository.folderRoles
                ?.get(m.accountId ?? m.account)
                ?.has(m.folder))) &&
          (!this.account || m.account === this.account) &&
          (this.filter !== "Unread" || m.unread) &&
          (this.filter !== "Flagged" || m.starred) &&
          words.every((q) =>
            `${m.sender} ${m.subject} ${m.body}`.toLowerCase().includes(q),
          ),
      )
      .sort((a, b) =>
        this.newestFirst
          ? b.date.localeCompare(a.date)
          : a.date.localeCompare(b.date),
      );
  }
  get visible() {
    return this.matching.slice(this.page * 50, (this.page + 1) * 50);
  }
  changed() {
    this.dispatchEvent(new Event("change"));
  }
  navigate(folder: string, account: string | null = null) {
    this.folder = folder;
    this.account = account;
    this.page = 0;
    this.selection.clear();
    this.selected = null;
    this.changed();
  }
  search(value: string) {
    this.query = value;
    this.page = 0;
    this.selection.clear();
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
      if (!known.has(message.id)) {
        const saved = structuredClone(message);
        this.mail.push(saved);
        this.confirmed.set(saved.id, structuredClone(saved));
        known.add(saved.id);
        this.revision++;
      }
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
      const rev = this.revision;
      try {
        const result = await this.repository.refresh(this.folder, this.account);
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
  async change(id: string, fields: Fields, offerUndo = true) {
    id = this.canonical(id);
    const current = this.message(id);
    if (!current || !Object.keys(fields).length) return;
    const previous = Object.fromEntries(
      Object.keys(fields).map((key) => [key, current[key as keyof Mail]]),
    ) as Fields;
    const revision = ++this.revision;
    for (const key of Object.keys(fields))
      this.versions.set(`${id}:${key}`, revision);
    this.paint(id, fields);
    this.selection.delete(id);
    if (offerUndo)
      this.undo = () => {
        this.undo = null;
        void this.change(id, previous, false);
      };
    this.notice = fields.folder
      ? `Moved to ${fields.folder}`
      : "Message updated";
    this.error = null;
    this.retry = null;
    const previousJobs = [...this.queues]
      .filter(([key]) => this.canonical(key) === id)
      .map(([, job]) => job);
    const job = Promise.all(previousJobs).then(async () => {
      try {
        await this.repository.mutate(this.canonical(id), fields);
        this.acceptAliases(this.repository.cached, revision);
        const key = this.canonical(id);
        this.confirmed.set(key, { ...this.confirmed.get(key)!, ...fields });
      } catch (error) {
        if (error instanceof MutationFailure && error.committed) {
          const key = this.canonical(id);
          this.confirmed.set(key, { ...this.confirmed.get(key)!, ...fields });
          this.error = error.message;
          this.notice = null;
          this.undo = null;
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
        if (Object.keys(rollback).length) {
          this.error = `Could not confirm the update to ${current.subject}. The affected display was restored. ${error instanceof Error ? error.message : "The affected change was restored. Retry."}`;
          this.notice = null;
          this.undo = null;
          this.retry =
            error instanceof MutationFailure
              ? () => void this.refresh()
              : () => void this.change(id, fields);
        }
      }
    });
    this.queues.set(id, job);
    this.changed();
    await job;
    if (this.queues.get(id) === job) this.queues.delete(id);
    this.changed();
  }
}
