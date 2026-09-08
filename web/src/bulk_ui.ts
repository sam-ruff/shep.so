import { button, el, modal } from "./ui";
import type { Workspace } from "./model";
import type { GatewayRepository } from "./provider";
import type { BulkAction, BulkItem, BulkJob } from "./bulk_journal";
import type {
  GroupDecision,
  GroupReview,
  GroupView,
  GroupRecovery,
} from "./bulk_client";
export const groupActionName = (action: BulkAction) =>
  action.kind === "move"
    ? action.folder.toLowerCase() === "archive"
      ? "Archive"
      : action.folder.toLowerCase() === "trash"
        ? "Delete"
        : `Move to ${action.folder}`
    : action.unread !== undefined
      ? action.unread
        ? "Mark unread"
        : "Mark read"
      : action.starred
        ? "Flag"
        : "Unflag";
const message = (error: unknown) =>
  error instanceof Error
    ? error.message
    : "Could not finish this group action. Open History to retry.";
const count = (n: number) => `${n} ${n === 1 ? "message" : "messages"}`;
function progress(job: BulkJob) {
  const c = job.counts;
  return [
    job.state === "review"
      ? "Not applied"
      : job.state === "interrupted"
        ? "Review interrupted"
        : job.paused
          ? "Paused"
          : job.undo
            ? "Undo"
            : "Group change",
    `${c.done} changed`,
    `${c.restored} restored`,
    `${c.running + c.undo_running} in progress`,
    `${c.pending} ${job.undo ? "cancelled before sending" : "waiting"}`,
    `${c.failed} failed`,
    `${c.uncertain} unconfirmed`,
    `${c.missing + c.skipped} unavailable or skipped`,
  ].join(" · ");
}

/** Presentation retains its History controls through background progress.
 * It holds at most 20 jobs and one 50-item page; execution stays in the adapter. */
export class GroupUI {
  private groups;
  private current?: BulkJob;
  private visible = false;
  private timer?: ReturnType<typeof setTimeout>;
  private preparing = false;
  private applying = false;
  private undoQueued = false;
  private historyDialog?: HTMLDialogElement;
  private historyRefresh?: () => void;
  private disposed = false;
  private recovery: GroupRecovery = { entries: [] };
  private recoveryExpanded = false;
  error?: string;
  constructor(
    private w: Workspace,
    private gateway: GatewayRepository,
  ) {
    this.groups = gateway.groups;
    this.groups.addEventListener("attention", (event) => {
      if (this.disposed) return;
      const next = (event as CustomEvent<GroupRecovery>).detail;
      if (JSON.stringify(next) === JSON.stringify(this.recovery)) return;
      this.recovery = next;
      this.w.changed();
    });
    this.groups.addEventListener("progress", (event) => {
      const job = (event as CustomEvent<BulkJob>).detail;
      if (this.disposed) return;
      if (this.current?.id === job.id && job.revision >= this.current.revision)
        this.current = job;
      this.w.groupChanged();
      this.historyRefresh?.();
    });
    this.groups.addEventListener("failure", (event) => {
      if (this.disposed) return;
      this.error = (event as CustomEvent<string>).detail;
      this.w.groupChanged();
      this.historyRefresh?.();
    });
    this.groups.start();
  }
  dispose() {
    this.disposed = true;
    clearTimeout(this.timer);
    this.groups.stop();
    this.historyDialog?.close();
  }
  private notify(job: BulkJob) {
    this.current = job;
    this.w.watchGroupUndo(job.undo ? undefined : job.id);
    this.visible = true;
    clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.visible = false;
      this.w.changed();
    }, 6000);
  }
  toolbar() {
    const controls = el("div", "group-actions");
    const actions: [string, BulkAction][] = [
      ["Archive", { kind: "move", folder: "Archive", account: null }],
      ["Delete", { kind: "move", folder: "Trash", account: null }],
      ["Mark read", { kind: "flags", unread: false }],
      ["Mark unread", { kind: "flags", unread: true }],
      ["Flag", { kind: "flags", starred: true }],
      ["Unflag", { kind: "flags", starred: false }],
    ];
    for (const [name, action] of actions) {
      const control = button(
        `${name} selected messages`,
        () => void this.prepare(action),
      );
      control.querySelector("span")!.textContent = name;
      control.disabled = !this.w.selection.ready || this.preparing;
      controls.append(control);
    }
    const move = button("Move selected messages", () => this.chooseFolder());
    move.querySelector("span")!.textContent = "Move…";
    move.disabled = !this.w.selection.ready || this.preparing;
    controls.append(move);
    return controls;
  }
  private chooseFolder() {
    const d = modal("Move selected messages"),
      label = el("label", "field", "Folder in each original account"),
      input = el("input");
    input.setAttribute("aria-label", "Destination folder");
    input.placeholder = "Projects";
    label.append(input);
    const status = el("p", "form-status"),
      apply = button("Review move", () => {
        if (!input.value.trim()) return;
        d.close();
        void this.prepare({
          kind: "move",
          folder: input.value.trim(),
          account: null,
        });
      });
    apply.disabled = true;
    input.oninput = () => {
      apply.disabled = !input.value.trim();
    };
    status.textContent =
      "Each message stays in its original account. Enter an existing destination folder.";
    d.append(label, status, apply);
    input.focus();
  }
  async prepare(action: BulkAction) {
    if (!this.w.selection.ready || !this.w.selection.snapshot || this.preparing)
      return;
    this.preparing = true;
    this.error = undefined;
    const snapshot = this.w.selection.snapshot,
      observed = this.w.groupObserved;
    const d = modal("Review group action"),
      status = el("p", "muted", "Preparing the selected messages…");
    status.setAttribute("role", "status");
    d.append(status);
    this.w.changed();
    try {
      const review = await this.groups.prepare(snapshot, action, observed);
      if (!d.isConnected || this.disposed) return;
      status.textContent = `${groupActionName(action)} ${count(review.job.total - review.job.counts.missing)}?`;
      const explanation = el(
        "p",
        "muted",
        `${count(review.job.total)} selected across all pages. ${review.job.counts.missing} unavailable on this device. New arrivals are excluded.`,
      );
      const accounts = el("ul", "selection-groups");
      for (const group of review.snapshot.groups.slice(0, 20))
        accounts.append(
          el(
            "li",
            "",
            `${count(group.total)} · ${this.gateway.accounts.find((a) => a.id === group.account)?.email ?? group.account} · ${group.folder === "INBOX" ? "Inbox" : group.folder}`,
          ),
        );
      if (review.snapshot.groups.length > 20)
        accounts.append(
          el(
            "li",
            "",
            `${review.snapshot.groups.length - 20} more account/folder groups`,
          ),
        );
      const actions = el("div", "dialog-actions"),
        cancel = button("Cancel group action", () => d.close());
      const apply = button(
        `${groupActionName(action)} ${count(review.job.total - review.job.counts.missing)}`,
        () => {
          if (this.applying) return;
          this.applying = true;
          this.undoQueued = false;
          const operation = this.groups.decide(review.job, "approve", false);
          const rollback = this.w.optimisticGroup(review);
          this.notify({ ...review.job, state: "ready" });
          d.close();
          this.w.changed();
          void operation
            .then(async (job) => {
              this.current = job;
              this.error = undefined;
              if (this.undoQueued) await this.undo(job);
              else this.groups.wake();
            })
            .catch((error) => {
              this.w.finishGroupUndo(review.job.id, false);
              rollback();
              this.visible = false;
              this.error = message(error);
            })
            .finally(() => {
              this.applying = false;
              this.w.groupChanged();
            });
        },
      );
      apply.disabled = review.job.total === review.job.counts.missing;
      apply.classList.add(
        action.kind === "move" && action.folder.toLowerCase() === "trash"
          ? "danger"
          : "primary",
      );
      actions.append(cancel, apply);
      d.append(explanation, accounts, actions);
    } catch (error) {
      status.textContent = message(error);
      status.className = "form-status";
      status.setAttribute("role", "alert");
      d.append(
        button("Select messages again", () => {
          d.close();
          this.w.selection.done();
          this.w.changed();
        }),
      );
    } finally {
      this.preparing = false;
      this.w.changed();
    }
  }
  private async undo(job: BulkJob) {
    if (this.applying && !job.forwardIntent) {
      this.undoQueued = true;
      this.w.beginGroupUndo(job.id);
      this.w.changed();
      return;
    }
    try {
      this.w.beginGroupUndo(job.id);
      const operation = this.groups.decide(job, "undo");
      this.notify({ ...job, undo: true });
      this.w.groupChanged();
      this.current = await operation;
      this.w.finishGroupUndo(job.id, true);
      this.error = undefined;
    } catch (error) {
      this.w.finishGroupUndo(job.id, false);
      this.current = job;
      this.error = message(error);
    }
    this.w.groupChanged();
  }
  notification() {
    if (!this.visible || !this.current) return;
    const job = this.current,
      n =
        job.total - job.counts.missing - job.counts.failed - job.counts.skipped;
    const status = el("div", "status");
    status.setAttribute("role", "status");
    status.setAttribute("aria-label", "Group notification");
    const label =
      job.undo || this.undoQueued
        ? `Undo requested for ${count(n)}`
        : `${groupActionName(job.action)} · ${count(n)}`;
    status.append(el("span", "", label));
    if (!job.undo && !this.undoQueued) {
      const undo = button("Undo group", () => void this.undo(this.current!));
      undo.dataset.stable = `group-undo:${job.id}`;
      status.append(undo);
    }
    const history = button("Group details", () => this.history(job.id));
    history.dataset.stable = `group-details:${job.id}`;
    status.append(
      history,
      button(
        "Dismiss group notification",
        () => {
          this.visible = false;
          this.w.changed();
        },
        "close",
        true,
      ),
    );
    return status;
  }
  errorBanner() {
    if (!this.error) return;
    const error = el("div", "error-banner");
    error.setAttribute("role", "alert");
    error.setAttribute("aria-label", "Group error");
    error.append(
      el("span", "", this.error),
      button("Review group history", () => this.history()),
      button(
        "Dismiss group error",
        () => {
          this.error = undefined;
          this.w.changed();
        },
        "close",
        true,
      ),
    );
    return error;
  }
  recoveryBanner() {
    if (!this.recovery.entries.length && (this.error || !this.recovery.error))
      return;
    const panel = el("section", "error-banner group-recovery");
    panel.setAttribute("role", "alert");
    panel.setAttribute("aria-label", "Saved group actions");
    const heading = el("div", "group-recovery-heading");
    heading.append(el("strong", "", "Saved group actions need review"));
    if (this.recovery.entries.length) {
      heading.append(
        el(
          "span",
          "",
          this.recovery.entries
            .map((entry) => {
              const label = {
                failed: "failed",
                uncertain: "unconfirmed",
                cache: "awaiting local repair",
                interrupted:
                  entry.count === 1
                    ? "interrupted review"
                    : "interrupted reviews",
              }[entry.kind];
              return `${entry.count} ${label}`;
            })
            .join(" · "),
        ),
      );
      const toggle = button("Review saved group actions", () => {
        this.recoveryExpanded = !this.recoveryExpanded;
        this.w.changed();
      });
      toggle.querySelector("span")!.textContent = this.recoveryExpanded
        ? "Hide details"
        : "Review";
      toggle.setAttribute("aria-expanded", String(this.recoveryExpanded));
      toggle.dataset.stable = "group-recovery:toggle";
      heading.append(toggle);
    }
    panel.append(heading);
    for (const entry of this.recoveryExpanded ? this.recovery.entries : []) {
      const n = entry.count;
      const description =
        entry.kind === "uncertain"
          ? `${n} ${n === 1 ? "change has an unconfirmed result" : "changes have unconfirmed results"}. Check the server folders before resolving ${n === 1 ? "it" : "them"}.`
          : entry.kind === "failed"
            ? `${n} ${n === 1 ? "change failed" : "changes failed"}. Review the errors before retrying.`
            : entry.kind === "cache"
              ? `${n} confirmed ${n === 1 ? "change needs" : "changes need"} local repair.`
              : `${n} ${n === 1 ? "review ended" : "reviews ended"} before approval. No changes were applied from these reviews.`;
      const label = {
        uncertain: "Review unconfirmed group changes",
        failed: "Review failed group changes",
        cache: "Review saved group results",
        interrupted: "Review interrupted group reviews",
      }[entry.kind];
      const row = el("div", "group-recovery-row"),
        open = button(label, () => this.history(entry.job));
      open.dataset.stable = `group-recovery:${entry.kind}`;
      row.append(el("span", "", description), open);
      panel.append(row);
    }
    if (this.recovery.error) panel.append(el("span", "", this.recovery.error));
    const retry = button("Refresh saved group status", () =>
      this.groups.refreshAttention(),
    );
    retry.dataset.stable = "group-recovery:refresh";
    if (this.recoveryExpanded || this.recovery.error) panel.append(retry);
    return panel;
  }
  history(selected?: string) {
    this.historyDialog?.close();
    const d = (this.historyDialog = modal("Group history")),
      status = el("p", "form-status", "Loading group history…"),
      list = el("div", "group-history-list"),
      detail = el("section", "group-detail");
    status.setAttribute("role", "status");
    let before: [number, string] | undefined,
      previous: ([number, string] | undefined)[] = [],
      jobs: BulkJob[] = [],
      view: GroupView | undefined,
      after = -1,
      positions: number[] = [],
      generation = 0,
      busy = false,
      refreshAgain = false,
      undoPreparing: { id: string } | undefined;
    const title = el("h3"),
      summary = el("p", "group-progress"),
      items = el("div", "group-items"),
      decisionError = el("p", "form-status"),
      previewError = el("p", "form-status");
    decisionError.setAttribute("role", "alert");
    previewError.setAttribute("role", "alert");
    const act = async (decision: GroupDecision) => {
      if (!view || busy) return;
      const target = view.job;
      busy = true;
      updateButtons();
      try {
        if (decision === "undo") this.w.beginGroupUndo(target.id);
        const current = await this.groups.decide(target, decision);
        if (decision === "undo") this.w.finishGroupUndo(target.id, true);
        if (decision === "undo") this.notify(current);
        if (view?.job.id === target.id) {
          view.job = current;
          decisionError.textContent = "";
        }
        this.error = undefined;
      } catch (error) {
        if (decision === "undo") this.w.finishGroupUndo(target.id, false);
        this.error = message(error);
        if (selected === target.id) decisionError.textContent = message(error);
      } finally {
        busy = false;
        updateButtons();
        refresh();
        this.w.groupChanged();
      }
    };
    const pause = button("Pause group", () => void act("pause")),
      resume = button("Resume group", () => void act("resume")),
      undo = button("Undo group", () => void act("undo"));
    const repair = button("Retry cached results", () => {
      this.error = undefined;
      this.groups.wake();
      status.textContent = "Retrying saved results…";
    });
    const details = el("div", "dialog-actions");
    details.append(pause, resume, undo, repair);
    const earlierItems = button("Previous results", () => {
        after = positions.pop() ?? -1;
        void loadView(true);
      }),
      laterItems = button("Next results", () => {
        if (!view?.items.length) return;
        positions.push(after);
        after = view.items.at(-1)!.position;
        void loadView(true);
      });
    detail.append(
      title,
      summary,
      decisionError,
      previewError,
      details,
      items,
      earlierItems,
      laterItems,
    );
    const updateButtons = () => {
      const job = view?.job;
      detail.hidden = !job;
      pause.hidden = !job || job.state !== "ready" || job.paused;
      resume.hidden = !job || job.state !== "ready" || !job.paused;
      undo.hidden = !job || job.state !== "ready" || job.undo;
      repair.hidden = !job?.pendingCache;
      for (const b of [pause, resume, undo, repair]) b.disabled = busy;
      undo.disabled = busy || undoPreparing?.id === job?.id;
      earlierItems.disabled = !positions.length;
      laterItems.disabled = (view?.items.length ?? 0) < 50;
    };
    const rowControls = new Map<
      number,
      {
        root: HTMLElement;
        text: HTMLElement;
        retry: HTMLButtonElement;
        resolve: HTMLButtonElement;
        checked: HTMLInputElement;
      }
    >();
    const renderView = (replace = false) => {
      if (!view) return;
      title.textContent = `${groupActionName(view.job.action)} · ${count(view.job.total)}`;
      summary.textContent = progress(view.job);
      if (replace) {
        rowControls.clear();
        items.replaceChildren();
      }
      const positions = new Set(view.items.map((item) => item.position));
      for (const [position, row] of rowControls) {
        if (!positions.has(position)) {
          row.root.remove();
          rowControls.delete(position);
        }
      }
      for (const item of view.items) {
        let row = rowControls.get(item.position);
        if (!row) {
          const root = el("article", "group-item"),
            text = el("p"),
            checked = el("input"),
            label = el("label", "selection-checkbox");
          checked.type = "checkbox";
          checked.setAttribute(
            "aria-label",
            `I checked server folders for message ${item.position + 1}`,
          );
          label.append(
            checked,
            el("span", "", "I checked the source and destination folders"),
          );
          const retry = button(
            `Retry message ${item.position + 1}`,
            async () => {
              if (!view || busy) return;
              busy = true;
              updateButtons();
              try {
                view.job = await this.groups.retry(
                  view.job,
                  view.items.find((v) => v.position === item.position)!,
                );
                decisionError.textContent = "";
              } catch (error) {
                decisionError.textContent = message(error);
              } finally {
                busy = false;
                refresh();
              }
            },
          );
          const resolve = button(
            `Accept current state for message ${item.position + 1}`,
            async () => {
              if (!view || busy || !checked.checked) return;
              busy = true;
              updateButtons();
              try {
                view.job = await this.groups.resolve(
                  view.job,
                  view.items.find((v) => v.position === item.position)!,
                );
                decisionError.textContent = "";
              } catch (error) {
                decisionError.textContent = message(error);
              } finally {
                busy = false;
                refresh();
                this.w.groupChanged();
              }
            },
          );
          checked.onchange = () => {
            resolve.disabled = !checked.checked || busy;
          };
          root.append(text, retry, label, resolve);
          items.append(root);
          row = { root, text, retry, resolve, checked };
          rowControls.set(item.position, row);
        }
        const state = {
          pending: "Waiting",
          running: "In progress",
          done: "Changed",
          undo_running: "Restoring",
          restored: "Restored",
          failed: "Failed",
          uncertain: "Unconfirmed",
          missing: "Unavailable",
          skipped: "Skipped",
        }[item.status];
        const source =
          item.original?.folder === "INBOX" ? "Inbox" : item.original?.folder;
        row.text.textContent = `Message ${item.position + 1} · ${view.descriptions?.[item.position] ?? "Message unavailable in this device cache"} · ${this.gateway.accounts.find((a) => a.id === item.account)?.email ?? item.account}${source ? ` · ${source}` : ""} · ${state}${item.error ? ` · ${item.error}` : ""}`;
        row.retry.hidden =
          item.status !== "failed" ||
          (view.job.undo && item.phase === "forward");
        row.retry.disabled = busy;
        row.resolve.hidden = item.status !== "uncertain";
        row.checked.parentElement!.hidden = item.status !== "uncertain";
        row.resolve.disabled = busy || !row.checked.checked;
      }
      updateButtons();
    };
    const loadView = async (replace = false) => {
      if (!selected) return;
      if (replace) {
        undoPreparing = undefined;
        previewError.textContent = "";
      }
      const request = ++generation;
      try {
        const describe = replace || !view || view.job.id !== selected;
        const result = await this.groups.view(selected, after, describe);
        if (!d.isConnected || request !== generation) return;
        const preparing =
          describe && !result.job.undo ? { id: result.job.id } : undefined;
        if (preparing) undoPreparing = preparing;
        // Subjects/senders are one displayed metadata page, refreshed explicitly
        // or when changing pages, never reread for every provider receipt.
        if (!describe)
          result.descriptions = Object.fromEntries(
            result.items.flatMap((item) => {
              const description = view?.descriptions?.[item.position];
              return description ? [[item.position, description]] : [];
            }),
          );
        view = result;
        renderView(replace);
        status.textContent = "";
        // Do not keep the History refresh loop waiting for a changing preview.
        // Its independent token also rejects late results after switching groups.
        if (preparing) {
          const current = () =>
            d.isConnected &&
            selected === result.job.id &&
            undoPreparing === preparing;
          void this.w.prepareGroupUndo(result.job.id).then(
            () => {
              if (!current()) return;
              undoPreparing = undefined;
              updateButtons();
            },
            (error) => {
              if (current())
                previewError.textContent = `${message(error)} Refresh history to retry Undo.`;
            },
          );
        }
      } catch (error) {
        if (d.isConnected && request === generation)
          status.textContent = message(error);
      }
    };
    let refreshing = false;
    const refresh = () => {
      if (!d.isConnected) return;
      if (refreshing) {
        refreshAgain = true;
        return;
      }
      refreshing = true;
      void loadView().finally(() => {
        refreshing = false;
        if (refreshAgain) {
          refreshAgain = false;
          refresh();
        }
      });
    };
    this.historyRefresh = refresh;
    const load = async () => {
      status.textContent = "Loading group history…";
      try {
        jobs = await this.groups.history(before);
        if (!d.isConnected) return;
        list.replaceChildren();
        for (const job of jobs) {
          const open = button(
            `${groupActionName(job.action)} ${count(job.total)} · ${new Date(job.created).toLocaleString()}`,
            () => {
              selected = job.id;
              undoPreparing = undefined;
              view = undefined;
              title.textContent = "";
              summary.textContent = "";
              decisionError.textContent = "";
              rowControls.clear();
              items.replaceChildren();
              updateButtons();
              status.textContent = "Loading group details…";
              after = -1;
              positions = [];
              void loadView(true);
            },
          );
          list.append(open);
        }
        status.textContent = jobs.length
          ? ""
          : "No group changes on this device.";
        older.disabled = jobs.length < 20;
        newer.disabled = !previous.length;
        if (selected) await loadView(true);
        else detail.hidden = true;
      } catch (error) {
        status.textContent = message(error);
        status.setAttribute("role", "alert");
      }
    };
    const older = button("Older groups", () => {
        if (!jobs.length) return;
        previous.push(before);
        before = [jobs.at(-1)!.created, jobs.at(-1)!.id];
        selected = undefined;
        void load();
      }),
      newer = button("Newer groups", () => {
        before = previous.pop();
        selected = undefined;
        void load();
      });
    const refreshControl = button("Refresh history", () => void load());
    d.append(refreshControl, status, list, newer, older, detail);
    d.addEventListener("close", () => {
      if (this.historyDialog === d) {
        this.historyDialog = undefined;
        this.historyRefresh = undefined;
        this.w.watchGroupUndo(this.current?.id);
      }
    });
    void load();
  }
}
