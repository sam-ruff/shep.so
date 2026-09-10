import type {
  MailboxRepository,
  MailboxQuery,
  MailboxPage,
  MailboxDetail,
  MailboxMetadata,
  MailScanQuery,
  MailScanPage,
} from "./mailbox_types";

class MailboxPort extends EventTarget {
  private worker: Worker;
  private sequence = 0;
  private closed = false;
  private closing?: Promise<void>;
  private ready: Promise<void>;
  private pending = new Map<
    number,
    { resolve: (value: unknown) => void; reject: (reason: Error) => void }
  >();
  get stopped() {
    return this.closed;
  }
  get load() {
    return this.pending.size;
  }
  constructor(user: string, worker: Worker) {
    super();
    this.worker = worker;
    this.worker.onmessage = ({ data }) => {
      const request = this.pending.get(data.id);
      if (!request) return;
      if (data.phase === "snapshot") {
        this.dispatchEvent(
          new CustomEvent("activity", {
            detail: { id: data.id, phase: data.phase },
          }),
        );
        return;
      }
      this.pending.delete(data.id);
      if (data.error) request.reject(Error(data.error));
      else request.resolve(data.result);
    };
    this.worker.onerror = () =>
      this.terminate("Mailbox storage stopped. Retry Refresh.");
    this.worker.onmessageerror = () =>
      this.terminate("Could not read the mailbox result. Retry Refresh.");
    this.ready = this.send({ initialize: user });
    void this.ready.catch(() =>
      this.terminate("Could not open mailbox storage. Reopen Shep to retry."),
    );
  }
  private send<T>(value: Record<string, unknown>, closing = false): Promise<T> {
    if (this.closed)
      return Promise.reject(Error("Mailbox storage is closed. Reopen Shep."));
    if (this.pending.size >= 32 && !closing)
      return Promise.reject(
        Error("The mailbox is catching up. Retry shortly."),
      );
    const id = this.sequence++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve: (value) => resolve(value as T), reject });
      try {
        this.worker.postMessage({ ...value, id });
      } catch {
        this.pending.delete(id);
        reject(Error("Could not request cached mail. Retry Refresh."));
      }
    });
  }
  async request<T>(value: Record<string, unknown>): Promise<T> {
    await this.ready;
    if (this.closing) throw Error("Mailbox storage is closing. Reopen Shep.");
    return this.send(value);
  }
  close(): Promise<void> {
    return (this.closing ??= this.finishClose());
  }
  private async finishClose() {
    if (this.closed) return;
    try {
      await this.ready;
      await this.send({ close: true }, true);
    } finally {
      this.terminate("Mailbox storage closed.");
    }
  }
  terminate(message = "Mailbox storage closed.") {
    this.closed = true;
    this.worker.terminate();
    for (const request of this.pending.values()) request.reject(Error(message));
    this.pending.clear();
  }
}

/** Queries, foreground bodies/metadata and scans never share a worker FIFO. */
export class MailboxWorkerClient
  extends EventTarget
  implements MailboxRepository
{
  private queries: MailboxPort;
  private readers: MailboxPort[] = [];
  private scanner?: MailboxPort;
  private speculative?: MailboxPort;
  private nextReader = 0;
  private closed = false;
  constructor(private user: string) {
    super();
    this.queries = new MailboxPort(
      user,
      new Worker(new URL("./mailbox_worker.ts", import.meta.url), {
        type: "module",
      }),
    );
    this.queries.addEventListener("activity", (event) =>
      this.dispatchEvent(
        new CustomEvent("activity", { detail: (event as CustomEvent).detail }),
      ),
    );
  }
  get stopped() {
    return this.closed || this.queries.stopped;
  }
  private makeReader() {
    if (this.stopped) throw Error("Mailbox storage is closed. Reopen Shep.");
    return new MailboxPort(
      this.user,
      new Worker(new URL("./mailbox_read_worker.ts", import.meta.url), {
        type: "module",
      }),
    );
  }
  private reader() {
    if (this.stopped) throw Error("Mailbox storage is closed. Reopen Shep.");
    for (let i = 0; i < 2; i++)
      if (!this.readers[i] || this.readers[i].stopped)
        this.readers[i] = this.makeReader();
    const index = this.nextReader++ % 2;
    return this.readers[index].load <= this.readers[1 - index].load
      ? this.readers[index]
      : this.readers[1 - index];
  }
  async prefetch(id: string): Promise<MailboxDetail> {
    if (this.stopped) throw Error("Mailbox storage is closed. Reopen Shep.");
    if (this.speculative?.stopped) this.speculative = undefined;
    return (this.speculative ??= this.makeReader()).request({ detail: id });
  }
  page(query: MailboxQuery): Promise<MailboxPage> {
    return this.queries.request({ query });
  }
  async detail(id: string): Promise<MailboxDetail> {
    return this.reader().request({ detail: id });
  }
  async metadata(id: string): Promise<MailboxMetadata> {
    return this.reader().request({ metadata: id });
  }
  async scan(query: MailScanQuery): Promise<MailScanPage> {
    if (this.stopped) throw Error("Mailbox storage is closed. Reopen Shep.");
    if (this.scanner?.stopped) this.scanner = undefined;
    return (this.scanner ??= this.makeReader()).request({ scan: query });
  }
  async close(): Promise<void> {
    this.closed = true;
    await Promise.all(
      [
        this.queries,
        ...this.readers.filter(Boolean),
        ...(this.scanner ? [this.scanner] : []),
        ...(this.speculative ? [this.speculative] : []),
      ].map((port) => port.close()),
    );
  }
  terminate(message = "Mailbox storage closed.") {
    this.closed = true;
    for (const port of [
      this.queries,
      ...this.readers.filter(Boolean),
      ...(this.scanner ? [this.scanner] : []),
      ...(this.speculative ? [this.speculative] : []),
    ])
      port.terminate(message);
  }
}
