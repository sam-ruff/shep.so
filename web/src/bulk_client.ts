import { openMailDatabase } from "./storage";
import { read, snapshot } from "./mailbox_cache";
import type { CacheMail, CacheState } from "./cache_changes";
import type { MailAlias } from "./sent_cache";
import { BulkExecutor } from "./bulk_executor";
import {
  BulkJournal,
  tabLock,
  type BulkAction,
  type BulkJob,
  type BulkItem,
  type BulkAttention,
} from "./bulk_journal";
import type { GatewayRepository } from "./provider";
import type { SelectionWorkerClient } from "./selection_worker_client";
import type { SelectionSnapshot } from "./selection_types";
export interface GroupReview {
  job: BulkJob;
  snapshot: SelectionSnapshot;
  selected: string[];
}
export interface GroupView {
  job: BulkJob;
  items: BulkItem[];
  descriptions?: Record<number, string>;
}
export interface GroupRecovery {
  entries: BulkAttention[];
  error?: string;
}
export type GroupDecision = "approve" | "pause" | "resume" | "undo";

/** UI-facing ownership. Preparing another exact review waits for the current
 * receipt, then releases the owner to the selection worker's bounded exporter. */
export class BrowserGroups extends EventTarget {
  private executor: BulkExecutor;
  private preparing = false;
  private closed = false;
  private decision?: Promise<unknown>;
  private active?: Promise<void>;
  private started = false;
  private attentionReading?: Promise<void>;
  private attentionAgain = false;
  private lastAttention: BulkAttention[] = [];
  private executionRevision = 0;
  /** This tab's review ownership token; its lock is held until stop(). */
  readonly owner = crypto.randomUUID();
  private tab?: Promise<void>;
  private releaseTab?: () => void;
  private sweeping?: Promise<void>;
  private sweepTimer?: ReturnType<typeof setTimeout>;
  constructor(
    private repository: GatewayRepository,
    private selection: () => SelectionWorkerClient | undefined,
    private sweepInterval = 30000,
  ) {
    super();
    this.executor = new BulkExecutor(
      repository.profileId,
      repository,
      (job) => this.progress(job),
      true,
      () => this.refreshAttention(),
    );
  }
  private progress(job: BulkJob) {
    if (!this.closed) {
      this.dispatchEvent(new CustomEvent("progress", { detail: job }));
      this.refreshAttention();
    }
  }
  refreshAttention() {
    if (this.closed) return;
    this.attentionAgain = true;
    if (this.attentionReading) return;
    this.attentionReading = Promise.resolve()
      .then(async () => {
        do {
          this.attentionAgain = false;
          const execution = this.executionRevision;
          let detail: GroupRecovery;
          try {
            const entries = await BulkJournal.inspect(
              this.repository.profileId,
              (j) => j.attention(),
            );
            // Ordinary receipt/cache handshakes are still being completed by the
            // active executor. Report a gap if execution stops before repairing it.
            detail = {
              entries: entries.filter(
                (entry) => entry.kind !== "cache" || !this.active,
              ),
            };
          } catch {
            detail = {
              entries: this.lastAttention,
              error:
                "Could not check saved group actions. Refresh their status to retry.",
            };
          }
          if (execution !== this.executionRevision) {
            this.attentionAgain = true;
            continue;
          }
          if (!detail.error) this.lastAttention = detail.entries;
          if (!this.closed)
            this.dispatchEvent(new CustomEvent("attention", { detail }));
        } while (this.attentionAgain && !this.closed);
      })
      .finally(() => {
        this.attentionReading = undefined;
        if (this.attentionAgain && !this.closed) this.refreshAttention();
      });
  }
  private check() {
    if (this.closed) throw Error("Group work is closed. Reopen Shep.");
  }
  async settledDecision() {
    await this.decision?.catch(() => {});
  }
  start() {
    if (this.started) return;
    this.started = true;
    void this.holdTab().catch(() => {});
    this.refreshAttention();
    this.wake();
    this.scheduleSweep();
  }
  stop() {
    this.closed = true;
    clearTimeout(this.sweepTimer);
    this.executor.stop();
    this.releaseTab?.();
  }
  private holdTab() {
    return (this.tab ??= new Promise<void>((resolve, reject) => {
      navigator.locks
        .request(tabLock(this.repository.profileId, this.owner), () => {
          resolve();
          // A client stopped before the grant must not hold the lock.
          if (this.closed) return;
          return new Promise<void>((release) => {
            this.releaseTab = release;
          });
        })
        .catch(reject);
    }));
  }
  private scheduleSweep() {
    if (this.closed) return;
    this.sweepTimer = setTimeout(() => {
      void this.sweep().finally(() => this.scheduleSweep());
    }, this.sweepInterval);
  }
  /** Bounded retirement of abandoned reviews between runs. It never contends
   * with review preparation or a live run in this tab, and another tab's
   * owner is left to sweep for itself. */
  sweep(): Promise<void> {
    if (this.sweeping) return this.sweeping;
    if (this.closed || this.preparing || this.active) return Promise.resolve();
    const work = this.executor
      .sweep()
      .then(
        (result) => {
          if (result?.retired && !this.closed) this.refreshAttention();
        },
        () => {},
      )
      .finally(() => {
        if (this.sweeping === work) this.sweeping = undefined;
      });
    this.sweeping = work;
    return work;
  }
  wake() {
    if (this.closed || this.preparing) return;
    this.executionRevision++;
    // An in-flight periodic sweep owns the journal lock; run after it.
    const run = (this.sweeping ?? Promise.resolve())
      .then(() => this.executor.run())
      .then(
        () => {},
        (error) => {
          if (!this.closed)
            this.dispatchEvent(
              new CustomEvent("failure", {
                detail:
                  error instanceof Error
                    ? error.message
                    : "Could not finish group work. Open History to retry.",
              }),
            );
        },
      );
    this.active = run;
    void run.finally(() => {
      if (this.active === run) {
        this.active = undefined;
        this.executionRevision++;
        this.refreshAttention();
      }
    });
  }
  async prepare(
    snapshot: SelectionSnapshot,
    action: BulkAction,
    observed: string[],
  ): Promise<GroupReview> {
    this.check();
    if (this.preparing)
      throw Error("A group review is already being prepared.");
    this.preparing = true;
    const frozen = crypto.randomUUID(),
      worker = this.selection();
    try {
      if (!worker || worker.stopped)
        throw Error("Selection storage expired. Select the messages again.");
      this.executor.stop();
      await this.active;
      await this.sweeping;
      await this.holdTab();
      this.check();
      const copied = (await worker.selection(
        {
          kind: "freeze",
          id: snapshot.id,
          expected: snapshot.revision,
          target: frozen,
        },
        observed.slice(0, 50),
      )) as SelectionSnapshot;
      const selected = new Set(copied.visible);
      for (let i = 50; i < observed.length; i += 50) {
        const part = (await worker.selection(
          { kind: "observe", id: frozen },
          observed.slice(i, i + 50),
        )) as SelectionSnapshot;
        part.visible.forEach((id) => selected.add(id));
      }
      const job = await worker.prepareBulk(
        frozen,
        copied.revision,
        crypto.randomUUID(),
        action,
        this.owner,
      );
      return { job, snapshot: copied, selected: [...selected] };
    } finally {
      await worker?.selection({ kind: "release", id: frozen }).catch(() => {});
      this.preparing = false;
      this.wake();
    }
  }
  async decide(job: BulkJob, decision: GroupDecision, start = true) {
    this.check();
    if (this.decision)
      throw Error(
        "The previous group decision is still saving. Retry shortly.",
      );
    const work = this.executor.decideCurrent(job, decision);
    this.decision = work;
    try {
      const saved = await work;
      this.progress(saved);
      if (start) this.wake();
      return saved;
    } finally {
      if (this.decision === work) this.decision = undefined;
    }
  }
  /** Declining or closing a frozen review fences it, then the next wake
   * removes its staged rows. Nothing was applied, so no rollback is needed. */
  async cancel(job: BulkJob) {
    this.check();
    const saved = await this.executor.cancel(job);
    this.wake();
    return saved;
  }
  history(before?: [number, string]) {
    this.check();
    return BulkJournal.inspect(this.repository.profileId, (j) =>
      j.history(before),
    );
  }
  async view(id: string, after = -1, describe = true): Promise<GroupView> {
    this.check();
    const result = await BulkJournal.inspect(this.repository.profileId, (j) =>
      j.view(id, after),
    );
    if (!describe) return result;
    const db = await openMailDatabase(this.repository.profileId);
    try {
      const descriptions = await snapshot(
        db,
        ["mailMetadata", "mailAliases", "cacheState"],
        async (tx) => {
          const state = await read<CacheState | undefined>(
            tx.objectStore("cacheState").get("mail"),
          );
          const descriptions: Record<number, string> = {};
          if (state?.epoch !== result.job.cacheEpoch) return descriptions;
          for (const item of result.items) {
            const alias = await read<MailAlias | undefined>(
              tx.objectStore("mailAliases").get(item.id),
            );
            const mail = await read<CacheMail | undefined>(
              tx.objectStore("mailMetadata").get(alias?.target ?? item.id),
            );
            if (mail?.core.account_id === item.account)
              descriptions[item.position] =
                `${mail.core.subject || "(No subject)"} · ${mail.core.sender}`;
          }
          return descriptions;
        },
      );
      return { ...result, descriptions };
    } finally {
      db.close();
    }
  }
  async retry(job: BulkJob, item: BulkItem) {
    this.check();
    const saved = await this.executor.retry(
      job.id,
      job.revision,
      item.position,
    );
    this.progress(saved);
    this.wake();
    return saved;
  }
  async resolve(job: BulkJob, item: BulkItem) {
    this.check();
    const saved = await this.executor.resolve(
      job.id,
      job.revision,
      item.position,
    );
    this.progress(saved);
    return saved;
  }
}
