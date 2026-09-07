import type {
  MailboxRepository,
  MailboxQuery,
  MailboxPage,
  MailboxDetail,
} from "./mailbox_types";

export class MailboxWorkerClient
  extends EventTarget
  implements MailboxRepository
{
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
  constructor(user: string) {
    super();
    this.worker = new Worker(new URL("./mailbox_worker.ts", import.meta.url), {
      type: "module",
    });
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
  async page(query: MailboxQuery): Promise<MailboxPage> {
    await this.ready;
    if (this.closing) throw Error("Mailbox storage is closing. Reopen Shep.");
    return this.send({ query });
  }
  async detail(id: string): Promise<MailboxDetail> {
    await this.ready;
    if (this.closing) throw Error("Mailbox storage is closing. Reopen Shep.");
    return this.send({ detail: id });
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
