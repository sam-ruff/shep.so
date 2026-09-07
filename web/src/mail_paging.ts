import type {
  MailboxPage,
  MailboxQuery,
  MailboxRepository,
} from "./mailbox_types";

/** One running query and one replacement request. Typing/navigation replaces
 * obsolete work instead of retaining a queue of pages or replaying old errors. */
export class MailPaging {
  loading = false;
  error: string | null = null;
  view = "";
  private wanted?: { key: string; view: string; query: MailboxQuery };
  private completed = "";
  private nonce = 0;
  private running?: Promise<void>;
  private closed = false;
  constructor(
    private repository: MailboxRepository,
    private current: () => MailboxQuery,
    private accept: (page: MailboxPage) => void,
    private changed: () => void,
  ) {}
  private target() {
    const query = this.current();
    const { projection: _projection, ...scope } = query.scope;
    const view = JSON.stringify([scope, query.offset]);
    return {
      query,
      view,
      key: JSON.stringify([
        query.scope,
        query.offset,
        query.generation,
        query.undo,
        this.nonce,
      ]),
    };
  }
  get currentView() {
    return this.target().view;
  }
  get ready() {
    return this.view === this.currentView;
  }
  sync() {
    if (this.closed) return;
    this.wanted = this.target();
    if (this.wanted.key === this.completed) return;
    this.loading = true;
    if (!this.running) {
      // Defer one microtask so running is installed before a synchronous fake
      // repository resolves or an acceptance callback requests another page.
      this.running = Promise.resolve()
        .then(() => this.pump())
        .finally(() => {
          this.running = undefined;
          if (!this.closed && this.wanted?.key !== this.completed) this.sync();
        });
    }
  }
  async reload() {
    this.nonce++;
    this.sync();
    // A completion callback can request a replacement during finally.
    // Follow that replacement too, so Refresh does not report completion early.
    do {
      await this.running;
    } while (!this.closed && this.running);
  }
  private async pump() {
    while (!this.closed && this.wanted && this.wanted.key !== this.completed) {
      const request = this.wanted;
      try {
        const page = await this.repository.page(request.query);
        if (this.closed || request.key !== this.wanted.key) continue;
        this.completed = request.key;
        this.view = request.view;
        this.error = null;
        this.accept(page);
      } catch (error) {
        if (this.closed || request.key !== this.wanted.key) continue;
        this.completed = request.key;
        this.error =
          error instanceof Error
            ? error.message
            : "Could not load this mailbox page. Retry.";
      }
      this.loading = this.wanted.key !== this.completed;
      this.changed();
    }
    this.loading = false;
  }
  close() {
    this.closed = true;
  }
}

/** Speculative body storage has an independent byte/cardinality budget. The
 * active reader may display a larger body without retaining it in this cache. */
export class MailBodies {
  private values = new Map<string, { body: string; bytes: number }>();
  private bytes = 0;
  get size() {
    return this.values.size;
  }
  get byteLength() {
    return this.bytes;
  }
  get(id: string) {
    const value = this.values.get(id);
    if (value) {
      this.values.delete(id);
      this.values.set(id, value);
    }
    return value?.body;
  }
  put(id: string, body: string) {
    const before = this.values.get(id);
    if (before) {
      this.values.delete(id);
      this.bytes -= before.bytes;
    }
    const bytes = body.length * 2;
    if (bytes > 32 * 1024 * 1024) return;
    this.values.set(id, { body, bytes });
    this.bytes += bytes;
    while (this.values.size > 8 || this.bytes > 32 * 1024 * 1024) {
      const first = this.values.entries().next().value!;
      this.values.delete(first[0]);
      this.bytes -= first[1].bytes;
    }
  }
  clear() {
    this.values.clear();
    this.bytes = 0;
  }
}
