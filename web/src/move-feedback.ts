export class MoveRecord {
  started = false;
  cancelled = false;
  committed = false;
  undoRequested = false;
  blocked = false;
  restoreCommitted = false;
  constructor(
    public id: string,
    readonly account: string,
    readonly originalFolder: string,
    /** The other account a transfer moved this message to. */
    readonly destination?: string,
  ) {}
  /** Fields that put the message back where it was. */
  get restoreFields(): { folder: string; accountId?: string } {
    return {
      folder: this.originalFolder,
      ...(this.destination ? { accountId: this.account } : {}),
    };
  }
}

/** Six-second desktop move feedback. Provider completion never recreates it. */
export class MoveFeedback {
  private timer?: ReturnType<typeof setTimeout>;
  private folder?: string;
  private account?: string;
  private restored = false;
  records: MoveRecord[] = [];
  constructor(private changed: () => void) {}
  get visible() {
    return this.records.length > 0;
  }
  get canUndo() {
    return !this.restored && this.records.some((r) => !r.blocked);
  }
  get label() {
    if (!this.visible) return null;
    const count = this.records.length,
      noun = count === 1 ? "message" : "messages";
    if (this.restored) return `Restored ${count} ${noun}`;
    if (this.folder?.toLowerCase() === "archive")
      return `Archived ${count} ${noun}`;
    if (this.folder?.toLowerCase() === "trash")
      return `Deleted ${count} ${noun}`;
    const folder =
      this.folder?.toLowerCase() === "inbox" ? "Inbox" : this.folder;
    return `Moved ${count} ${noun} to ${folder}`;
  }
  private schedule() {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.dismiss(), 6000);
  }
  add(
    id: string,
    account: string,
    original: string,
    folder: string,
    destination?: string,
  ) {
    const target = destination ?? account;
    const standard = ["archive", "trash"].includes(folder.toLowerCase());
    const same = standard
      ? this.folder?.toLowerCase() === folder.toLowerCase()
      : this.folder === folder;
    if (this.restored || !same || (!standard && this.account !== target))
      this.records = [];
    this.folder = folder;
    this.account = target;
    this.restored = false;
    const record = new MoveRecord(id, account, original, destination);
    this.records = [...this.records, record];
    this.schedule();
    return record;
  }
  restore(expected: MoveRecord[]) {
    if (!this.canUndo || expected !== this.records) return [];
    const records = this.records.filter((r) => !r.blocked).reverse();
    for (const record of records) {
      record.undoRequested = true;
      if (!record.started) record.cancelled = true;
    }
    this.records = records;
    this.restored = true;
    this.schedule();
    return records;
  }
  failed(record: MoveRecord) {
    if (!this.records.includes(record)) return;
    this.records = this.records.filter((r) => r !== record);
    if (!this.visible) clearTimeout(this.timer);
  }
  retryRestore(record: MoveRecord) {
    if (!this.restored) this.records = [];
    this.restored = true;
    if (!this.records.includes(record))
      this.records = [...this.records, record];
    this.schedule();
  }
  removeAccount(account: string) {
    const involved = (r: MoveRecord) =>
      r.account === account || r.destination === account;
    if (!this.records.some(involved)) return;
    this.records = this.records.filter((r) => !involved(r));
    if (!this.visible) clearTimeout(this.timer);
  }
  dismiss() {
    clearTimeout(this.timer);
    this.records = [];
    this.changed();
  }
  dispose() {
    clearTimeout(this.timer);
  }
}
