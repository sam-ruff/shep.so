import {
  BulkJournal,
  type BulkJob,
  type BulkItem,
  type BulkSweep,
} from "./bulk_journal";
import { intentValues, type IntentLease } from "./mail_intents";
import { MutationFailure, type Fields } from "./model";
import {
  MutationSuperseded,
  type GatewayRepository,
  type MutationReceipt,
} from "./provider";

type Operations = Pick<
  GatewayRepository,
  | "profileId"
  | "bulkIntents"
  | "mutateWithReceipt"
  | "repairMutation"
  | "bulkCacheEpoch"
  | "bulkUnavailable"
>;
type Decision = Parameters<BulkJournal["decide"]>[2];
export interface BulkRun {
  steps: number;
  repairs: number;
}
const failure = (error: unknown) =>
  (error instanceof Error && error.message
    ? error.message
    : "The group change could not finish. Reopen its history."
  )
    .replace(/[\x00-\x1f\x7f]/g, " ")
    .slice(0, 4096);

/** One owner, one provider step and bounded journal observations. UI decisions
 * may use this owner while its provider call is pending. No captured ID list is
 * brought into the main thread, and an abandoned running step never replays. */
export class BulkExecutor {
  private journal?: BulkJournal;
  private running?: Promise<BulkRun>;
  private stopped = false;
  private requested = 0;
  private completed = 0;
  private lastNotice = 0;
  constructor(
    private user: string,
    private operations: Operations,
    private changed?: (job: BulkJob) => void,
    private requireEpoch = false,
    private recovered?: () => void,
  ) {
    if (operations.profileId !== user)
      throw Error(
        "This group belongs to another browser profile. Reopen Shep.",
      );
  }
  stop() {
    this.stopped = true;
  }
  async decide(id: string, expected: number, decision: Decision) {
    // Clock allocation begins with the decision, before ownership/older work.
    const revision =
      decision === "approve" || decision === "undo"
        ? await this.operations.bulkIntents.reserve()
        : undefined;
    const apply = (journal: BulkJournal) =>
      journal.decide(id, expected, decision, revision);
    return this.journal
      ? apply(this.journal)
      : BulkJournal.own(this.user, apply);
  }
  async decideCurrent(expected: BulkJob, decision: Decision) {
    const revision =
      decision === "approve" || decision === "undo"
        ? await this.operations.bulkIntents.reserve()
        : undefined;
    const apply = (journal: BulkJournal) =>
      journal.decideCurrent(expected, decision, revision);
    return this.journal
      ? apply(this.journal)
      : BulkJournal.own(this.user, apply);
  }
  resolve(id: string, expected: number, position: number) {
    const apply = async (journal: BulkJournal) => {
      const job = await journal.get(id),
        item = await journal.getItem(id, position);
      if (job.revision !== expected || item.status !== "uncertain")
        throw Error(
          "This group changed. Refresh its review before continuing.",
        );
      const lease = item.phase === "forward" ? item.intent : item.undoIntent;
      // Retire only this unresolved local ownership before accepting the review.
      // This does not classify the provider result as a rejection. A failed
      // cleanup leaves the uncertain item available for another explicit try.
      if (lease) await this.operations.bulkIntents.finish(lease, "failed");
      return journal.resolveUncertain(id, expected, position);
    };
    return this.journal
      ? apply(this.journal)
      : BulkJournal.own(this.user, apply);
  }
  retry(id: string, expected: number, position: number) {
    const apply = (journal: BulkJournal) =>
      journal.retry(id, expected, position);
    return this.journal
      ? apply(this.journal)
      : BulkJournal.own(this.user, apply);
  }
  cancel(job: BulkJob) {
    const apply = (journal: BulkJournal) => journal.cancel(job);
    return this.journal
      ? apply(this.journal)
      : BulkJournal.own(this.user, apply);
  }
  /** Periodic cleanup between runs. A running owner already sweeps at each
   * wake, and another tab's owner sweeps for itself. */
  sweep(limit = 20): Promise<BulkSweep | null> {
    if (this.running) return Promise.resolve(null);
    return BulkJournal.own(this.user, (journal) => journal.sweep(limit));
  }
  run(): Promise<BulkRun> {
    const requested = ++this.requested;
    this.stopped = false;
    if (this.running)
      return this.running.then((result) =>
        requested > this.completed && !this.stopped ? this.run() : result,
      );
    this.running = BulkJournal.own(this.user, async (journal) => {
      this.journal = journal;
      const total: BulkRun = { steps: 0, repairs: 0 };
      let first = true;
      do {
        const wake = this.requested;
        // Abandoned reviews retire before any provider step of this wake, and
        // before the recovery status is read for the new owner.
        await journal.sweep(4);
        if (first) {
          first = false;
          this.recovered?.();
        }
        await this.drain(journal, total);
        this.completed = wake;
      } while (this.requested > this.completed && !this.stopped);
      return total;
    }).finally(() => {
      this.journal = undefined;
      this.running = undefined;
    });
    return this.running;
  }
  private notice(job: BulkJob, force = false) {
    if (force || Date.now() - this.lastNotice >= 100) {
      this.lastNotice = Date.now();
      this.changed?.(job);
    }
  }
  private lease(item: BulkItem) {
    const lease = item.phase === "forward" ? item.intent : item.undoIntent;
    if (!lease)
      throw Error(
        "This saved group has no action ownership. Reopen its recovery review.",
      );
    return lease;
  }
  private async repair(
    journal: BulkJournal,
    item: BulkItem,
    result?: MutationReceipt,
  ) {
    const receipt =
      result?.receipt ??
      (item.phase === "forward" ? item.receipt : item.inverse);
    if (!receipt || !item.attempt)
      throw Error(
        "This group has no saved receipt. Reopen its recovery review.",
      );
    const after = await this.operations.repairMutation(
      receipt,
      this.lease(item),
    );
    const job = await journal.cacheSaved(
      item.job,
      item.position,
      item.attempt,
      after,
    );
    this.notice(job);
    return job;
  }
  private async drain(journal: BulkJournal, total: BulkRun) {
    let latest: BulkJob | undefined;
    while (!this.stopped) {
      // No provider writes while any acknowledged cache/identity gap remains.
      const repairs = await journal.pendingCache();
      for (const item of repairs) {
        if (this.requireEpoch) {
          const job = await journal.get(item.job);
          if (
            !job.cacheEpoch ||
            job.cacheEpoch !== (await this.operations.bulkCacheEpoch())
          )
            throw Error(
              "This saved result belongs to an earlier device cache. Review the server folders before repeating this change.",
            );
        }
        latest = await this.repair(journal, item);
        total.repairs++;
        if (this.stopped) break;
      }
      if (repairs.length) continue;
      const job = await journal.next();
      if (!job) break;
      if (!job.forwardIntent || (job.undo && !job.undoIntent)) {
        await journal.decide(job.id, job.revision, "pause");
        throw Error(
          "This earlier group has no saved approval revision. Select and review its messages again.",
        );
      }
      if (
        this.requireEpoch &&
        (!job.cacheEpoch ||
          job.cacheEpoch !== (await this.operations.bulkCacheEpoch()))
      ) {
        const saved = await journal.decide(job.id, job.revision, "pause");
        this.notice(saved, true);
        throw Error(
          "This group belongs to an earlier device cache. Select and review its messages again.",
        );
      }
      const item = await journal.claim(job.id);
      if (!item) break;
      if (this.requireEpoch) {
        const unavailable = await this.operations.bulkUnavailable(item.id);
        if (unavailable) {
          const failed = await journal.settle(
            job.id,
            item.position,
            item.attempt!,
            { kind: "rejected", error: unavailable },
          );
          const paused = await journal.decide(job.id, failed.revision, "pause");
          this.notice(paused, true);
          throw Error(unavailable);
        }
      }
      await this.execute(journal, job, item);
      total.steps++;
      latest = await journal.get(job.id);
      this.notice(latest);
    }
    if (latest) this.notice(latest, true);
  }
  private async execute(journal: BulkJournal, job: BulkJob, item: BulkItem) {
    const attempt = item.attempt!;
    let lease: IntentLease | undefined,
      acknowledged = false;
    const settleReceipt = async (result: MutationReceipt) => {
      const current = await journal.getItem(job.id, item.position);
      if (current.attempt !== attempt)
        throw Error("This provider step changed. Reopen its recovery review.");
      if (!["done", "restored"].includes(current.status))
        await journal.settle(job.id, item.position, attempt, {
          kind: "committed",
          receipt: result.receipt,
          applied: result.applied,
          // Require the final cache/identity/action-record handshake even for a
          // local cache acknowledgment; completion cannot be lost on disk error.
          cacheApplied: false,
        });
      acknowledged = true;
    };
    try {
      const current = await journal.get(job.id);
      job = current;
      if (item.phase === "forward" && current.undo) {
        await journal.settle(job.id, item.position, attempt, {
          kind: "skipped",
          error: "Cancelled before this message was sent to the provider.",
        });
        return;
      }
      if (
        job.action.kind === "move" &&
        job.action.account !== null &&
        job.action.account !== item.account
      )
        throw Error(
          "Cross-account group moves are not connected yet. Choose a folder in the original account.",
        );
      const expected =
        item.phase === "forward" ? item.original : item.receipt?.after;
      if (!expected)
        throw Error(
          "This message has no acknowledged source identity. Reopen its review.",
        );
      const fields: Fields =
        item.phase === "forward"
          ? job.action.kind === "move"
            ? { folder: job.action.folder }
            : job.action
          : Object.fromEntries(
              Object.keys(item.intent?.fields ?? {}).map((key) => [
                key,
                item.receipt!.before[key as keyof Fields],
              ]),
            );
      lease = await this.operations.bulkIntents.claim(
        item.id,
        item.phase === "forward" ? job.forwardIntent! : job.undoIntent!,
        intentValues(fields),
        item.phase === "undo" ? item.intent?.revision : undefined,
      );
      if (!Object.keys(lease.fields).length) {
        await journal.settle(job.id, item.position, attempt, {
          kind: "skipped",
          error:
            "A newer choice owns these fields. No provider change was made.",
        });
        return;
      }
      await journal.attachIntent(job.id, item.position, attempt, lease);
      const result = await this.operations.mutateWithReceipt(
        item.id,
        lease.fields,
        settleReceipt,
        expected,
        lease,
      );
      // A lost local callback reply may already have saved the exact receipt.
      if (!acknowledged) await settleReceipt(result);
      await this.repair(
        journal,
        await journal.getItem(job.id, item.position),
        result,
      );
    } catch (error) {
      if (
        error instanceof MutationFailure &&
        error.committed &&
        error.receipt &&
        lease
      ) {
        const result = {
          receipt: error.receipt,
          cacheApplied: error.cacheApplied,
          applied: error.applied ?? lease.fields,
        };
        await settleReceipt(result);
        await this.repair(
          journal,
          await journal.getItem(job.id, item.position),
          result,
        );
        return;
      }
      if (acknowledged) throw error;
      if (error instanceof MutationSuperseded) {
        await journal.settle(job.id, item.position, attempt, {
          kind: "skipped",
          error: "A newer choice replaced this action before dispatch.",
        });
        return;
      }
      if (lease && !(error instanceof MutationFailure))
        await this.operations.bulkIntents.finish(lease, "failed");
      await journal.settle(job.id, item.position, attempt, {
        kind: error instanceof MutationFailure ? "uncertain" : "rejected",
        error: failure(error),
      });
    }
  }
}
