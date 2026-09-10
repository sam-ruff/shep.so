// Browser profile history: one WASM in-memory journal per Google binding,
// owned by a worker. The page persists every accepted record itself, so the
// worker holds no IndexedDB connection and can be restarted from the store.
import {
  HistoryError,
  type Binding,
  type HistoryCommand,
  type HistoryReply,
  type HistoryErrorKind,
  type Overview,
  type StoredRecord,
} from "./profile_types";

export interface HistoryModule {
  ProfileHistory: new (
    binding: string,
    device: string,
    records: string,
  ) => {
    execute(command: string): string;
    overview(): string;
    record(operation: string): string;
    device(): string;
    free(): void;
  };
  validate_profile_operation(bytes: Uint8Array): Uint8Array;
}
export type HostRequest =
  | {
      kind: "open";
      key: string;
      binding: Binding;
      device: string;
      records: StoredRecord[];
    }
  | { kind: "execute"; key: string; command: HistoryCommand }
  | { kind: "overview"; key: string }
  | { kind: "record"; key: string; operation: string }
  | { kind: "validate"; record: string }
  | { kind: "close"; key: string };
export type HostReply =
  | { status: "ok"; value: unknown }
  | { status: "error"; kind: HistoryErrorKind; message: string };

const MAX_PENDING = 32;

/// Pure command dispatcher over the WASM module; shared by the real worker
/// and the inline test port.
export class HistoryHost {
  private journals = new Map<
    string,
    InstanceType<HistoryModule["ProfileHistory"]>
  >();
  constructor(private wasm: HistoryModule) {}
  handle(request: HostRequest): HostReply {
    try {
      switch (request.kind) {
        case "open": {
          this.journals.get(request.key)?.free();
          this.journals.delete(request.key);
          const journal = new this.wasm.ProfileHistory(
            JSON.stringify(request.binding),
            request.device,
            JSON.stringify(request.records),
          );
          this.journals.set(request.key, journal);
          return { status: "ok", value: journal.device() };
        }
        case "execute": {
          const reply = JSON.parse(
            this.journal(request.key).execute(JSON.stringify(request.command)),
          ) as HostReply;
          return reply;
        }
        case "overview":
          return {
            status: "ok",
            value: JSON.parse(this.journal(request.key).overview()),
          };
        case "record":
          return {
            status: "ok",
            value: JSON.parse(
              this.journal(request.key).record(request.operation),
            ),
          };
        case "validate":
          return {
            status: "ok",
            value: new TextDecoder().decode(
              this.wasm.validate_profile_operation(
                new TextEncoder().encode(request.record),
              ),
            ),
          };
        case "close":
          this.journals.get(request.key)?.free();
          this.journals.delete(request.key);
          return { status: "ok", value: null };
      }
    } catch (error) {
      return classify(error);
    }
  }
  private journal(key: string) {
    const journal = this.journals.get(key);
    if (!journal)
      throw new HistoryError(
        "stopped",
        "The profile history is not open. Reopen it and retry this same request.",
      );
    return journal;
  }
}
function classify(error: unknown): HostReply {
  if (error instanceof HistoryError)
    return { status: "error", kind: error.kind, message: error.message };
  const message = error instanceof Error ? error.message : String(error);
  // WASM constructor failures carry "kind:message"; codec failures carry the
  // shared record messages.
  const prefixed =
    /^(binding|storage|identity|cycle|invalid|too_large|upgrade|local_data|removed|incomplete):(.*)$/s.exec(
      message,
    );
  if (prefixed)
    return {
      status: "error",
      kind: prefixed[1] as HistoryErrorKind,
      message: prefixed[2],
    };
  if (message.includes("unsupported version or capability"))
    return { status: "error", kind: "upgrade", message };
  if (message.includes("record size"))
    return { status: "error", kind: "too_large", message };
  if (message.includes("Device state or credentials"))
    return { status: "error", kind: "local_data", message };
  return { status: "error", kind: "invalid", message };
}

export interface HistoryPort {
  request(message: HostRequest): Promise<HostReply>;
  dispose(): void;
}
/// Direct dispatch for unit tests and single-threaded fallbacks.
export class InlineHistoryPort implements HistoryPort {
  private host: HistoryHost;
  constructor(wasm: HistoryModule) {
    this.host = new HistoryHost(wasm);
  }
  request(message: HostRequest): Promise<HostReply> {
    return Promise.resolve(this.host.handle(message));
  }
  dispose() {}
}
/// Bounded request channel to the module worker: at most 32 outstanding
/// requests, in order, with a lost worker failing every pending request.
export class WorkerHistoryPort implements HistoryPort {
  private worker: Worker;
  private pending = new Map<
    number,
    { resolve: (reply: HostReply) => void; reject: (error: Error) => void }
  >();
  private next = 1;
  constructor(create: () => Worker) {
    this.worker = create();
    this.worker.onmessage = (event: MessageEvent) => {
      const { id, reply } = event.data as { id: number; reply: HostReply };
      const waiter = this.pending.get(id);
      this.pending.delete(id);
      waiter?.resolve(reply);
    };
    this.worker.onerror = () => this.fail();
  }
  request(message: HostRequest): Promise<HostReply> {
    if (this.pending.size >= MAX_PENDING)
      return Promise.resolve({
        status: "error",
        kind: "busy",
        message: "The profile worker is busy. Retry this same request shortly.",
      });
    const id = this.next++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.worker.postMessage({ id, request: message });
    });
  }
  dispose() {
    this.fail();
    this.worker.terminate();
  }
  private fail() {
    for (const waiter of this.pending.values())
      waiter.resolve({
        status: "error",
        kind: "stopped",
        message:
          "The profile worker stopped. Reopen it and retry this same request.",
      });
    this.pending.clear();
  }
}

type ReplyValue<T extends HistoryReply["kind"]> = Extract<
  HistoryReply,
  { kind: T }
>["value"];
/// Typed facade for one opened journal. Every write returns the record the
/// caller must persist before treating the write as acknowledged.
export class ProfileJournal {
  constructor(
    private port: HistoryPort,
    readonly key: string,
    readonly binding: Binding,
  ) {}
  static async open(
    port: HistoryPort,
    key: string,
    binding: Binding,
    device: string,
    records: StoredRecord[],
  ): Promise<ProfileJournal> {
    unwrap(await port.request({ kind: "open", key, binding, device, records }));
    return new ProfileJournal(port, key, binding);
  }
  async execute<T extends HistoryReply["kind"]>(
    command: HistoryCommand,
    expected: T,
  ): Promise<ReplyValue<T>> {
    const reply = unwrap(
      await this.port.request({ kind: "execute", key: this.key, command }),
    ) as HistoryReply;
    if (reply.kind !== expected)
      throw new HistoryError("storage", "Unexpected profile history reply.");
    return reply.value as ReplyValue<T>;
  }
  state() {
    return this.execute({ kind: "state" }, "state");
  }
  async overview(): Promise<Overview> {
    return unwrap(
      await this.port.request({ kind: "overview", key: this.key }),
    ) as Overview;
  }
  async record(operation: string): Promise<StoredRecord> {
    return unwrap(
      await this.port.request({ kind: "record", key: this.key, operation }),
    ) as StoredRecord;
  }
  async close() {
    await this.port.request({ kind: "close", key: this.key });
  }
}
export async function validateRecord(
  port: HistoryPort,
  record: string,
): Promise<string> {
  return unwrap(await port.request({ kind: "validate", record })) as string;
}
export function unwrap(reply: HostReply): unknown {
  if (reply.status === "error")
    throw new HistoryError(reply.kind, reply.message);
  return reply.value;
}
