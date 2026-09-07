import { selectionToken } from "./selection_types";
import { intentValues, type IntentLease } from "./mail_intents";
import type { Fields } from "./model";

// Durable metadata only. The runner must revalidate current identities, order
// individual intent and retain ownership until each provider receipt is saved.
export type BulkAction =
  | { kind: "move"; folder: string; account: string | null }
  | { kind: "flags"; unread?: boolean; starred?: boolean };
export interface BulkIdentity {
  id: string;
  account: string;
  folder: string;
  remoteId: string;
  unread: boolean;
  starred: boolean;
}
export interface BulkOriginal {
  position: number;
  id: string;
  account: string;
  original: BulkIdentity | null;
}
export type BulkStatus =
  | "pending"
  | "running"
  | "done"
  | "undo_running"
  | "restored"
  | "failed"
  | "uncertain"
  | "missing"
  | "skipped";
// A superseded/cancelled operation is distinct from provider success.
export interface BulkReceipt {
  before: BulkIdentity;
  after: BulkIdentity;
  // MOVE may acknowledge without a destination UID. Preserve its exact-content
  // proof, never an obsolete source UID pretending to be the destination.
  recovery?: { bytes: number; sha256: number[] };
}
export interface BulkItem extends BulkOriginal {
  job: string;
  status: BulkStatus;
  phase: "forward" | "undo";
  attempt?: string;
  receipt?: BulkReceipt;
  inverse?: BulkReceipt;
  error?: string;
  cache?: number;
  intent?: IntentLease;
  undoIntent?: IntentLease;
}
export interface BulkJob {
  id: string;
  action: BulkAction;
  state: "preparing" | "interrupted" | "review" | "ready";
  created: number;
  revision: number;
  total: number;
  staged: number;
  lastPosition: number;
  paused: boolean;
  undo: boolean;
  counts: Record<BulkStatus, number>;
  pendingCache?: number;
  runnable?: number;
  forwardIntent?: number;
  undoIntent?: number;
}
export type BulkOutcome =
  | {
      kind: "committed";
      receipt: BulkReceipt;
      cacheApplied?: boolean;
      applied?: Fields;
    }
  | { kind: "rejected" | "uncertain" | "skipped"; error: string };
const statuses: BulkStatus[] = [
  "pending",
  "running",
  "done",
  "undo_running",
  "restored",
  "failed",
  "uncertain",
  "missing",
  "skipped",
];
const request = <T>(r: IDBRequest<T>) =>
  new Promise<T>((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
const changed = () =>
  Error("This group changed. Refresh its review before continuing.");
function bounds(n: number) {
  if (!Number.isSafeInteger(n) || n < 0)
    throw Error("Invalid group position or count.");
}
function text(value: string) {
  if (
    typeof value !== "string" ||
    !value.length ||
    value.length > 4096 ||
    /[\x00-\x1f\x7f]/.test(value)
  )
    throw Error("Invalid group identity or folder.");
  return value;
}
function identity(v: BulkIdentity): BulkIdentity {
  if (typeof v.unread !== "boolean" || typeof v.starred !== "boolean")
    throw Error("Invalid message flags in group review.");
  return {
    id: text(v.id),
    account: text(v.account),
    folder: text(v.folder),
    remoteId: v.remoteId === "" ? "" : text(v.remoteId),
    unread: v.unread,
    starred: v.starred,
  };
}
function proof(v: NonNullable<BulkReceipt["recovery"]>) {
  bounds(v.bytes);
  if (
    !Array.isArray(v.sha256) ||
    v.sha256.length !== 32 ||
    v.sha256.some((n) => !Number.isInteger(n) || n < 0 || n > 255)
  )
    throw Error("Invalid move recovery proof.");
  return { bytes: v.bytes, sha256: [...v.sha256] };
}
function action(v: BulkAction): BulkAction {
  if (v.kind === "move")
    return {
      kind: "move",
      folder: text(v.folder),
      account: v.account === null ? null : text(v.account),
    };
  if (
    v.kind !== "flags" ||
    (typeof v.unread !== "boolean" && typeof v.starred !== "boolean")
  )
    throw Error("Choose a group action before reviewing messages.");
  return {
    kind: "flags",
    ...(typeof v.unread === "boolean" ? { unread: v.unread } : {}),
    ...(typeof v.starred === "boolean" ? { starred: v.starred } : {}),
  };
}
async function open(user: string): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    let abandoned = false;
    const r = indexedDB.open(`shep.bulk.v1.${user}`, 3);
    r.onupgradeneeded = (event) => {
      const tx = r.transaction!;
      if (event.oldVersion < 1) {
        const jobs = r.result.createObjectStore("jobs", { keyPath: "id" });
        jobs.createIndex("created", ["created", "id"]);
        jobs.createIndex("state", "state");
        const items = r.result.createObjectStore("items", {
          keyPath: ["job", "position"],
        });
        items.createIndex("status", ["job", "status", "position"]);
        items.createIndex("recovery", "status");
        items.createIndex("identity", ["job", "id"], { unique: true });
      }
      const jobs = tx.objectStore("jobs"),
        items = tx.objectStore("items");
      if (event.oldVersion < 2) {
        jobs.createIndex("queue", ["runnable", "created", "id"]);
        items.createIndex("cache", ["cache", "job", "position"]);
      }
      const cursor = jobs.openCursor();
      cursor.onsuccess = () => {
        if (!cursor.result) return;
        const job = cursor.result.value as BulkJob;
        job.pendingCache ??= 0;
        job.counts.skipped ??= 0;
        job.runnable = runnable(job);
        cursor.result.update(job);
        cursor.result.continue();
      };
    };
    r.onblocked = () => {
      abandoned = true;
      reject(Error("Close other Shep tabs to finish updating group storage."));
    };
    r.onerror = () =>
      reject(Error("Could not open local group storage. Retry."));
    r.onsuccess = () => {
      if (abandoned) r.result.close();
      else {
        r.result.onversionchange = () => r.result.close();
        resolve(r.result);
      }
    };
  });
}

/** Exclusive ownership spans network work and receipt persistence, so another
 * tab cannot recover an active step. Mail/draft writes use a separate database.
 * Storage APIs alone do not authorize or execute any provider operation. */
export class BulkJournal {
  private active = true;
  private constructor(
    private db: IDBDatabase,
    private owned: boolean,
  ) {}
  /** Read current progress without taking execution ownership or recovering a
   * live owner's step. Mutation methods refuse this observational connection. */
  static async inspect<T>(
    user: string,
    work: (journal: BulkJournal) => Promise<T>,
  ): Promise<T> {
    if (!/^[A-Za-z0-9_-]{43}$/.test(user))
      throw Error("Invalid browser profile identity.");
    const journal = new BulkJournal(await open(user), false);
    try {
      return await work(journal);
    } finally {
      journal.active = false;
      journal.db.close();
    }
  }
  static async own<T>(
    user: string,
    work: (journal: BulkJournal) => Promise<T>,
  ): Promise<T> {
    if (!/^[A-Za-z0-9_-]{43}$/.test(user))
      throw Error("Invalid browser profile identity.");
    return navigator.locks.request(
      `shep.bulk.v1.${user}`,
      { ifAvailable: true },
      async (lock) => {
        if (!lock)
          throw Error(
            "Group work is active in another tab. Return to that tab or retry when it finishes.",
          );
        const journal = new BulkJournal(await open(user), true);
        try {
          await journal.recover();
          return await work(journal);
        } finally {
          journal.active = false;
          journal.db.close();
        }
      },
    );
  }
  private transaction<T>(
    mode: IDBTransactionMode,
    work: (tx: IDBTransaction) => Promise<T>,
  ): Promise<T> {
    if (!this.active)
      return Promise.reject(
        Error("Group ownership expired. Reopen its review."),
      );
    if (mode === "readwrite" && !this.owned)
      return Promise.reject(
        Error(
          "This view only observes group progress. Use the active owner to change it.",
        ),
      );
    return new Promise((resolve, reject) => {
      const tx = this.db.transaction(["jobs", "items"], mode, {
        durability: "strict",
      });
      let result: T, cause: unknown;
      tx.oncomplete = () => resolve(result);
      tx.onabort = () =>
        reject(
          cause ??
            Error(
              "Could not save group progress. Reopen its history before retrying.",
            ),
        );
      // Only await IDB requests inside this transaction.
      void work(tx).then(
        (value) => {
          result = value;
        },
        (error) => {
          cause = error;
          try {
            tx.abort();
          } catch {
            reject(error);
          }
        },
      );
    });
  }
  private async job(
    tx: IDBTransaction,
    id: string,
    expected?: number,
  ): Promise<BulkJob> {
    selectionToken(id);
    const job: BulkJob | undefined = await request(
      tx.objectStore("jobs").get(id),
    );
    if (!job)
      throw Error("This group is no longer available. Select messages again.");
    if (expected !== undefined && job.revision !== expected) throw changed();
    return job;
  }
  private save(tx: IDBTransaction, job: BulkJob) {
    bounds(job.revision + 1);
    job.revision++;
    job.runnable = runnable(job);
    tx.objectStore("jobs").put(job);
    return job;
  }
  private transition(
    tx: IDBTransaction,
    job: BulkJob,
    item: BulkItem,
    status: BulkStatus,
  ) {
    job.counts[item.status]--;
    job.counts[status]++;
    item.status = status;
    tx.objectStore("items").put(item);
    this.save(tx, job);
  }
  private async recover() {
    // A durable running marker is an unknown outcome, never proof of rejection.
    for (const status of ["running", "undo_running"] as const) {
      while (
        await this.transaction("readwrite", async (tx) => {
          const rows: BulkItem[] = await request(
            tx.objectStore("items").index("recovery").getAll(status, 50),
          );
          for (const item of rows) {
            const job = await this.job(tx, item.job);
            item.error =
              "Shep closed before this result was confirmed. Check the server folders before resolving it.";
            job.paused = true;
            this.transition(tx, job, item, "uncertain");
          }
          return rows.length === 50;
        })
      ) {
        /* bounded recovery page */
      }
    }
    while (
      await this.transaction("readwrite", async (tx) => {
        const rows: BulkJob[] = await request(
          tx.objectStore("jobs").index("state").getAll("preparing", 20),
        );
        for (const job of rows) {
          job.state = "interrupted";
          this.save(tx, job);
        }
        return rows.length === 20;
      })
    ) {
      /* interrupted staging cannot execute */
    }
  }
  get(id: string) {
    return this.transaction("readonly", (tx) => this.job(tx, id));
  }
  getItem(id: string, position: number): Promise<BulkItem> {
    selectionToken(id);
    bounds(position);
    return this.transaction("readonly", async (tx) => {
      const item = await request<BulkItem | undefined>(
        tx.objectStore("items").get([id, position]),
      );
      if (!item) throw changed();
      return item;
    });
  }
  next(): Promise<BulkJob | null> {
    return this.transaction("readonly", async (tx) => {
      const jobs: BulkJob[] = await request(
        tx
          .objectStore("jobs")
          .index("queue")
          .getAll(
            IDBKeyRange.bound(
              [1, 0, ""],
              [1, Number.MAX_SAFE_INTEGER, "\uffff"],
            ),
            1,
          ),
      );
      return jobs[0] ?? null;
    });
  }
  pendingCache(): Promise<BulkItem[]> {
    return this.transaction("readonly", (tx) =>
      request(
        tx
          .objectStore("items")
          .index("cache")
          .getAll(
            IDBKeyRange.bound(
              [1, "", 0],
              [1, "\uffff", Number.MAX_SAFE_INTEGER],
            ),
            50,
          ),
      ),
    );
  }
  history(before?: [number, string]): Promise<BulkJob[]> {
    if (before) {
      bounds(before[0]);
      selectionToken(before[1]);
    }
    return this.transaction(
      "readonly",
      (tx) =>
        new Promise((resolve, reject) => {
          const result: BulkJob[] = [];
          const cursor = tx
            .objectStore("jobs")
            .index("created")
            .openCursor(
              before ? IDBKeyRange.upperBound(before, true) : undefined,
              "prev",
            );
          cursor.onerror = () => reject(cursor.error);
          cursor.onsuccess = () => {
            if (!cursor.result || result.length === 20) {
              resolve(result);
              return;
            }
            result.push(cursor.result.value);
            cursor.result.continue();
          };
        }),
    );
  }
  page(id: string, after = -1): Promise<BulkItem[]> {
    selectionToken(id);
    if (after !== -1) bounds(after);
    return this.transaction("readonly", (tx) =>
      request(
        tx
          .objectStore("items")
          .getAll(
            IDBKeyRange.bound([id, after], [id, Number.MAX_SAFE_INTEGER], true),
            50,
          ),
      ),
    );
  }
  async prepare(
    id: string,
    requested: BulkAction,
    total: number,
    chunks: AsyncIterable<BulkOriginal[]>,
  ): Promise<BulkJob> {
    selectionToken(id);
    bounds(total);
    const chosen = action(requested);
    if (!total) throw Error("Select at least one message to review.");
    await this.transaction("readwrite", async (tx) => {
      const counts = Object.fromEntries(
        statuses.map((s) => [s, 0]),
      ) as BulkJob["counts"];
      await request(
        tx.objectStore("jobs").add({
          id,
          action: chosen,
          state: "preparing",
          created: Date.now(),
          revision: 0,
          total,
          staged: 0,
          lastPosition: -1,
          paused: false,
          undo: false,
          counts,
        } satisfies BulkJob),
      );
    });
    // Failure retains an interrupted record. Never replace a partial group with
    // newly captured membership using the same job identity.
    for await (const chunk of chunks) {
      if (!chunk.length || chunk.length > 50)
        throw Error("Stage one group page at a time.");
      await this.transaction("readwrite", async (tx) => {
        const job = await this.job(tx, id);
        if (job.state !== "preparing" || job.staged + chunk.length > job.total)
          throw changed();
        for (const row of chunk) {
          bounds(row.position);
          if (row.position <= job.lastPosition)
            throw Error("Group ranks must remain unique and ordered.");
          const original = row.original ? identity(row.original) : null;
          if (
            original &&
            (original.id !== row.id || original.account !== row.account)
          )
            throw changed();
          const status = original ? "pending" : "missing";
          tx.objectStore("items").add({
            job: id,
            position: row.position,
            id: text(row.id),
            account: text(row.account),
            original,
            status,
            phase: "forward",
          } satisfies BulkItem);
          job.counts[status]++;
          job.staged++;
          job.lastPosition = row.position;
        }
        this.save(tx, job);
      });
    }
    return this.transaction("readwrite", async (tx) => {
      const job = await this.job(tx, id);
      if (job.state !== "preparing" || job.staged !== job.total)
        throw Error(
          "The complete group was not saved. Select and review it again.",
        );
      job.state = "review";
      return this.save(tx, job);
    });
  }
  decide(
    id: string,
    expected: number,
    decision: "approve" | "pause" | "resume" | "undo",
    intentRevision?: number,
  ) {
    if (intentRevision !== undefined) {
      bounds(intentRevision);
      if (!intentRevision) throw changed();
    }
    return this.transaction("readwrite", async (tx) => {
      const job = await this.job(tx, id, expected);
      if (decision === "approve") {
        if (job.state !== "review") throw changed();
        job.state = "ready";
        job.forwardIntent = intentRevision;
      } else {
        if (job.state !== "ready") throw changed();
        if (decision === "undo") {
          if (
            job.undo &&
            intentRevision !== undefined &&
            intentRevision !== job.undoIntent
          )
            throw changed();
          if (
            intentRevision !== undefined &&
            (!job.forwardIntent || intentRevision <= job.forwardIntent)
          )
            throw changed();
          job.undo = true;
          job.undoIntent ??= intentRevision;
        } else if (decision === "pause") job.paused = true;
        else if (decision === "resume") job.paused = false;
        else throw Error("Unsupported group decision.");
      }
      return this.save(tx, job);
    });
  }
  claim(id: string): Promise<BulkItem | null> {
    return this.transaction("readwrite", async (tx) => {
      const job = await this.job(tx, id);
      if (
        job.state !== "ready" ||
        job.paused ||
        job.counts.running ||
        job.counts.undo_running
      )
        return null;
      // One provider step across all groups in this profile, including Undo.
      const running = tx.objectStore("items").index("recovery");
      if (
        (await request(running.getKey("running"))) !== undefined ||
        (await request(running.getKey("undo_running"))) !== undefined
      )
        return null;
      if (
        (await request(
          tx
            .objectStore("items")
            .index("cache")
            .getKey(
              IDBKeyRange.bound(
                [1, "", 0],
                [1, "\uffff", Number.MAX_SAFE_INTEGER],
              ),
            ),
        )) !== undefined
      )
        return null;
      const status = job.undo ? "done" : "pending";
      const rows: BulkItem[] = await request(
        tx
          .objectStore("items")
          .index("status")
          .getAll(
            IDBKeyRange.bound(
              [id, status, 0],
              [id, status, Number.MAX_SAFE_INTEGER],
            ),
            1,
          ),
      );
      const item = rows[0];
      if (!item) return null;
      item.phase = job.undo ? "undo" : "forward";
      item.attempt = crypto.randomUUID();
      delete item.error;
      this.transition(tx, job, item, job.undo ? "undo_running" : "running");
      return item;
    });
  }
  attachIntent(
    id: string,
    position: number,
    attempt: string,
    lease: IntentLease,
  ) {
    bounds(position);
    selectionToken(attempt);
    bounds(lease.revision);
    return this.transaction("readwrite", async (tx) => {
      const job = await this.job(tx, id),
        item = await request<BulkItem | undefined>(
          tx.objectStore("items").get([id, position]),
        );
      if (
        !item ||
        item.attempt !== attempt ||
        !["running", "undo_running"].includes(item.status) ||
        lease.id !== item.id ||
        lease.account !== item.account ||
        lease.revision !==
          (item.phase === "forward" ? job.forwardIntent : job.undoIntent)
      )
        throw changed();
      const fields = intentValues(lease.fields);
      if (!Object.keys(fields).length) throw changed();
      const expected: Fields =
        item.phase === "forward"
          ? job.action.kind === "flags"
            ? job.action
            : { folder: job.action.folder }
          : Object.fromEntries(
              Object.keys(item.intent?.fields ?? {}).map((key) => [
                key,
                item.receipt?.before[key as keyof BulkIdentity],
              ]),
            );
      if (
        Object.entries(fields).some(
          ([key, value]) =>
            intentValues(expected)[key as keyof Fields] !== value,
        )
      )
        throw changed();
      const saved: IntentLease = {
        id: lease.id,
        account: lease.account,
        revision: lease.revision,
        fields,
      };
      if (item.phase === "forward") item.intent = saved;
      else item.undoIntent = saved;
      tx.objectStore("items").put(item);
      return this.save(tx, job);
    });
  }
  settle(id: string, position: number, attempt: string, result: BulkOutcome) {
    bounds(position);
    selectionToken(attempt);
    return this.transaction("readwrite", async (tx) => {
      const job = await this.job(tx, id);
      const item: BulkItem | undefined = await request(
        tx.objectStore("items").get([id, position]),
      );
      if (
        !item ||
        item.attempt !== attempt ||
        !["running", "undo_running"].includes(item.status)
      )
        throw changed();
      if (result.kind === "committed") {
        const receipt = {
          before: identity(result.receipt.before),
          after: identity(result.receipt.after),
          ...(result.receipt.recovery
            ? { recovery: proof(result.receipt.recovery) }
            : {}),
        };
        const source =
          item.phase === "forward" ? item.original : item.receipt?.after;
        if (
          !source ||
          receipt.before.id !== source.id ||
          receipt.before.account !== source.account ||
          receipt.before.folder !== source.folder ||
          receipt.before.remoteId !== source.remoteId
        )
          throw Error(
            "The receipt does not match this group's physical source message.",
          );
        if (item.phase === "forward") item.receipt = receipt;
        else item.inverse = receipt;
        const lease = item.phase === "forward" ? item.intent : item.undoIntent;
        if (lease) {
          if (!result.applied)
            throw Error(
              "Record the fields acknowledged by this provider step.",
            );
          const applied = intentValues(result.applied);
          if (
            !Object.keys(applied).length ||
            Object.entries(applied).some(
              ([key, value]) =>
                lease.fields[key as keyof Fields] !== value ||
                receipt.after[key as keyof BulkIdentity] !== value,
            )
          )
            throw changed();
          lease.fields = applied;
        }
        item.cache =
          result.cacheApplied === false ||
          (receipt.recovery && receipt.after.remoteId === "")
            ? 1
            : 0;
        job.pendingCache = (job.pendingCache ?? 0) + item.cache;
        this.transition(
          tx,
          job,
          item,
          item.phase === "forward" ? "done" : "restored",
        );
      } else {
        item.error = text(result.error);
        if (result.kind === "uncertain") job.paused = true;
        else if (result.kind !== "rejected" && result.kind !== "skipped")
          throw Error("Unsupported group outcome.");
        this.transition(
          tx,
          job,
          item,
          result.kind === "uncertain"
            ? "uncertain"
            : result.kind === "skipped"
              ? "skipped"
              : "failed",
        );
      }
      return job;
    });
  }
  retry(id: string, expected: number, position: number) {
    bounds(position);
    return this.transaction("readwrite", async (tx) => {
      const job = await this.job(tx, id, expected);
      const item: BulkItem | undefined = await request(
        tx.objectStore("items").get([id, position]),
      );
      if (
        !item ||
        item.status !== "failed" ||
        (job.undo && item.phase === "forward")
      )
        throw changed();
      delete item.error;
      delete item.attempt;
      this.transition(
        tx,
        job,
        item,
        item.phase === "undo" ? "done" : "pending",
      );
      return job;
    });
  }
  cacheSaved(
    id: string,
    position: number,
    attempt: string,
    resolved?: BulkIdentity,
  ) {
    bounds(position);
    selectionToken(attempt);
    return this.transaction("readwrite", async (tx) => {
      const job = await this.job(tx, id);
      const item: BulkItem | undefined = await request(
        tx.objectStore("items").get([id, position]),
      );
      if (
        !item ||
        item.attempt !== attempt ||
        !["done", "restored"].includes(item.status)
      )
        throw changed();
      const receipt = item.phase === "undo" ? item.inverse : item.receipt;
      if (!receipt) throw changed();
      if (resolved) {
        const current = identity(resolved),
          before = receipt.after;
        if (
          current.id !== before.id ||
          current.account !== before.account ||
          current.folder !== before.folder ||
          (before.remoteId !== "" && current.remoteId !== before.remoteId)
        )
          throw changed();
        // Only enrich a formerly absent destination identity. Flags describe the
        // acknowledged operation and must not inherit a later cache/UI refresh.
        receipt.after.remoteId = current.remoteId;
      }
      if (receipt.recovery && receipt.after.remoteId === "")
        throw Error(
          "Refresh the destination to recover this move's identity before continuing.",
        );
      if (item.cache) {
        item.cache = 0;
        job.pendingCache = (job.pendingCache ?? 0) - 1;
      }
      tx.objectStore("items").put(item);
      return this.save(tx, job);
    });
  }
}
function runnable(job: BulkJob) {
  return +(
    job.state === "ready" &&
    !job.paused &&
    (job.undo ? job.counts.done : job.counts.pending) > 0
  );
}
