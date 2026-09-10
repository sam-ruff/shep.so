import type {
  BulkIdentity,
  BulkJob,
  BulkJournal,
  BulkOriginal,
} from "../bulk_journal";
import type { IntentLease, IntentStore } from "../mail_intents";
import type { Fields } from "../model";
import type { GatewayRepository, MutationReceipt } from "../provider";

export function identity(i: number, account = "work"): BulkIdentity {
  return {
    id: `m${i}`,
    account,
    folder: "INBOX",
    remoteId: `${i}`,
    unread: true,
    starred: false,
  };
}
export function rows(count: number, from = 0): BulkOriginal[] {
  return Array.from({ length: count }, (_, i) => ({
    position: i,
    id: `m${from + i}`,
    account: "work",
    original: identity(from + i),
  }));
}
async function* chunks(all: BulkOriginal[]) {
  for (let i = 0; i < all.length; i += 50) yield all.slice(i, i + 50);
}
export function stage(
  journal: BulkJournal,
  id: string,
  members: BulkOriginal[],
  owner?: string,
  action: BulkJob["action"] = { kind: "flags", starred: true },
) {
  return journal.prepare(
    id,
    action,
    members.length,
    chunks(members),
    "epoch-1",
    owner,
  );
}
export async function pages(journal: BulkJournal, id: string) {
  let after = -1,
    total = 0;
  for (;;) {
    const page = await journal.page(id, after);
    if (!page.length) return total;
    total += page.length;
    after = page.at(-1)!.position;
  }
}

type Owner = { revision: number; origin?: number; value: unknown };
/** In-memory field ownership with the browser claim rules: a forward claim
 * takes fields owned by an older revision; an Undo claim takes only fields
 * still owned by the group it reverses. */
export class MemoryIntents implements IntentStore {
  clock = 0;
  owners = new Map<string, Partial<Record<keyof Fields, Owner>>>();
  async reserve() {
    return ++this.clock;
  }
  async register(): Promise<IntentLease> {
    throw Error("register is not used by the executor");
  }
  async claim(
    id: string,
    revision: number,
    fields: Fields,
    undoOf?: number,
  ): Promise<IntentLease> {
    const record = this.owners.get(id) ?? {};
    const accepted: Fields = {};
    for (const key of ["folder", "unread", "starred"] as const) {
      const value = fields[key];
      if (value === undefined) continue;
      const old = record[key];
      const owned =
        undoOf === undefined
          ? !old || old.revision < revision
          : old?.revision === undoOf;
      if (!owned) continue;
      record[key] = { revision, value, origin: undoOf };
      Object.assign(accepted, { [key]: value });
    }
    this.owners.set(id, record);
    return { id, account: "work", revision, fields: accepted };
  }
  async effective() {
    return {};
  }
  async uncached() {
    return {};
  }
  finished: [IntentLease, string][] = [];
  async finish(lease: IntentLease, status: string) {
    this.finished.push([lease, status]);
  }
}
export interface ProviderLog {
  calls: { id: string; fields: Fields }[];
}
/** Fictional provider: every receipt reflects the requested fields unless the
 * test overrides `respond`. */
export function operations(
  user: string,
  respond?: (
    id: string,
    fields: Fields,
    expected: BulkIdentity,
  ) => Promise<MutationReceipt> | MutationReceipt,
) {
  const intents = new MemoryIntents();
  const log: ProviderLog = { calls: [] };
  const ops: Pick<
    GatewayRepository,
    | "profileId"
    | "bulkIntents"
    | "mutateWithReceipt"
    | "repairMutation"
    | "bulkCacheEpoch"
    | "bulkUnavailable"
  > = {
    profileId: user,
    bulkIntents: intents,
    mutateWithReceipt: async (id, fields, _ack, expected) => {
      log.calls.push({ id, fields });
      if (respond) return respond(id, fields, expected!);
      return {
        receipt: { before: expected!, after: { ...expected!, ...fields } },
        cacheApplied: true,
        applied: fields,
      };
    },
    repairMutation: async (receipt) => receipt.after,
    bulkCacheEpoch: async () => "epoch-1",
    bulkUnavailable: async () => undefined,
  };
  return { ops, intents, log };
}
