import type {
  SelectionCommand,
  SelectionRepository,
  SelectionResult,
} from "./selection_types";

/** One temporary SQLite connection per worker. The UI sends gestures and
 * rendered IDs only; membership and ranking stay in worker-owned storage. */
export class SelectionWorkerClient
  extends EventTarget
  implements SelectionRepository
{
  private worker: Worker;
  private sequence = 0;
  private closed = false;
  private closing?: Promise<void>;
  get stopped() {
    return this.closed;
  }
  private ready: Promise<SelectionResult>;
  private pending = new Map<
    number,
    {
      resolve: (value: SelectionResult) => void;
      reject: (reason: Error) => void;
    }
  >();
  constructor(user: string) {
    super();
    this.worker = new Worker(
      new URL("./selection_worker.ts", import.meta.url),
      { type: "module" },
    );
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
      this.terminate(
        "Selection storage stopped. Choose Done, then select the messages again.",
      );
    this.worker.onmessageerror = () =>
      this.terminate(
        "Could not read the selection result. Choose Done and select again.",
      );
    this.ready = this.send({ initialize: user });
    // A lazy caller may close before awaiting initialization.
    void this.ready.catch(() =>
      this.terminate(
        "Could not open selection storage. Select messages again to retry.",
      ),
    );
  }
  private send(
    value: Record<string, unknown>,
    closing = false,
  ): Promise<SelectionResult> {
    if (this.closed)
      return Promise.reject(
        Error("Selection storage is closed. Select the messages again."),
      );
    if (this.pending.size >= 32 && !closing)
      return Promise.reject(
        Error("Selection is catching up. Retry that action shortly."),
      );
    const id = this.sequence++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      try {
        this.worker.postMessage({ ...value, id });
      } catch {
        this.pending.delete(id);
        reject(Error("Could not send this selection action. Retry."));
      }
    });
  }
  async selection(
    command: SelectionCommand,
    observed: string[] = [],
  ): Promise<SelectionResult> {
    if (observed.length > 50) throw Error("Observe one mail page at a time.");
    await this.ready;
    if (this.closing)
      throw Error("Selection is closing. Select messages again.");
    return this.send({ command, observed });
  }
  close(): Promise<void> {
    return (this.closing ??= this.finishClose());
  }
  private async finishClose(): Promise<void> {
    if (this.closed) return;
    try {
      await this.ready;
      await this.send({ close: true }, true);
    } finally {
      this.terminate("Selection closed.");
    }
  }
  // Abrupt tab/worker loss destroys its private temporary database.
  // A replacement worker requires an explicit new capture.
  terminate(message = "Selection closed.") {
    this.closed = true;
    this.worker.terminate();
    for (const request of this.pending.values()) request.reject(Error(message));
    this.pending.clear();
  }
}
