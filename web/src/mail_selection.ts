import {
  selectionScope,
  type SelectionChange,
  type SelectionCommand,
  type SelectionRepository,
  type SelectionScope,
  type SelectionSnapshot,
} from "./selection_types";

type Gesture =
  | { all: true; failed?: boolean }
  | {
      all?: false;
      change: SelectionChange;
      range?: [number, number];
      failed?: boolean;
    };

/** One ordered request, at most 32 gestures and one rendered page. The complete
 * captured membership stays in the repository, never in the UI controller. */
export class MailSelection {
  mode = false;
  snapshot?: SelectionSnapshot;
  error?: string;
  warning?: string;
  anchor?: string;
  private anchorPosition?: number;
  private identity?: string;
  private backend?: string;
  private capturedScope?: string;
  private running = false;
  private disposed = false;
  private gestures: Gesture[] = [];
  private watched = new Set<string>();
  private dirty = new Map<string, number>();
  private observationGeneration = 0;
  private refreshVersion = 0;
  private acknowledgedRefresh = 0;
  private positions = new Map<string, number>();
  private chosen = new Set<string>();
  private aliases = new Map<string, string>();
  constructor(
    private repository: SelectionRepository,
    private scope: () => SelectionScope,
    private currentCount: () => number,
    private changed: () => void,
  ) {}
  private canonical(id: string) {
    return this.aliases.get(id) ?? id;
  }
  get pending() {
    return this.mode && (!this.snapshot || this.gestures.length > 0);
  }
  get observing() {
    return (
      this.mode &&
      (this.dirty.size > 0 || this.refreshVersion !== this.acknowledgedRefresh)
    );
  }
  get ready() {
    return (
      this.mode &&
      !this.pending &&
      !this.observing &&
      !this.error &&
      this.count > 0
    );
  }
  get count() {
    let count = this.snapshot?.selected ?? 0;
    const chosen = new Set(this.chosen);
    for (const gesture of this.gestures) {
      if (gesture.failed) continue;
      if (gesture.all) {
        count = this.currentCount();
        for (const id of [...this.watched, ...this.gestureIds])
          chosen.add(this.canonical(id));
        continue;
      }
      const change = gesture.change;
      if (change.kind === "clear") {
        count = 0;
        chosen.clear();
      } else if (change.kind === "set") {
        if (change.clear_others) {
          count = 0;
          chosen.clear();
        }
        const id = this.canonical(change.id);
        if (change.selected && !chosen.has(id)) {
          count++;
          chosen.add(id);
        } else if (!change.selected && chosen.delete(id)) count--;
      } else if (gesture.range) {
        const [start, end] = gesture.range;
        if (!change.additive) {
          count = end - start + 1;
          chosen.clear();
        }
        for (const [id, position] of this.positions)
          if (position >= start && position <= end) chosen.add(id);
      }
    }
    return Math.max(0, Math.min(Number.MAX_SAFE_INTEGER, count));
  }
  selected(original: string) {
    const id = this.canonical(original);
    let selected = this.chosen.has(id);
    for (const gesture of this.gestures) {
      if (gesture.failed) continue;
      if (gesture.all) {
        selected = true;
        continue;
      }
      const change = gesture.change;
      if (change.kind === "clear") selected = false;
      else if (change.kind === "set") {
        if (change.clear_others) selected = false;
        if (this.canonical(change.id) === id) selected = change.selected;
      } else if (gesture.range) {
        if (!change.additive) selected = false;
        const position = this.positions.get(id),
          [start, end] = gesture.range;
        if (position !== undefined && position >= start && position <= end)
          selected = true;
      } else if (change.kind === "range") {
        if (!change.additive) selected = false;
        if (
          id === this.canonical(change.target) ||
          id === this.canonical(change.anchor)
        )
          selected = true;
      }
    }
    return this.mode && selected;
  }
  setObserved(ids: string[]) {
    if (ids.length > 50) throw Error("Observe one mail page at a time.");
    const next = new Set(ids);
    this.markDirty(ids.filter((id) => !this.watched.has(id)));
    for (const id of this.watched) if (!next.has(id)) this.dirty.delete(id);
    this.watched = next;
    this.prune();
    queueMicrotask(() => void this.pump());
  }
  private markDirty(ids: Iterable<string>) {
    const generation = ++this.observationGeneration;
    for (const id of ids) this.dirty.set(id, generation);
  }
  private get gestureIds() {
    const ids = new Set<string>();
    for (const gesture of this.gestures) {
      if (gesture.all) continue;
      if (gesture.change.kind === "set") ids.add(gesture.change.id);
      else if (gesture.change.kind === "range") {
        ids.add(gesture.change.anchor);
        ids.add(gesture.change.target);
      }
    }
    return ids;
  }
  private prune() {
    const needed = new Set(
      [
        ...this.watched,
        ...this.gestureIds,
        ...(this.anchor ? [this.anchor] : []),
      ].map((id) => this.canonical(id)),
    );
    for (const id of this.positions.keys())
      if (!needed.has(id)) this.positions.delete(id);
    for (const id of this.chosen) if (!needed.has(id)) this.chosen.delete(id);
    for (const [id, target] of this.aliases)
      if (!needed.has(target)) this.aliases.delete(id);
  }
  start() {
    if (this.mode || this.disposed) return;
    this.mode = true;
    this.identity = crypto.randomUUID();
    this.capturedScope = JSON.stringify(selectionScope(this.scope()));
    this.snapshot = undefined;
    this.error = undefined;
    this.markDirty(this.watched);
    this.changed();
    void this.pump();
  }
  /** Called before the workspace emits a state change, including direct filter
   * and account edits. Scope changes discard immediately; pages do not. */
  reconcileScope() {
    if (
      this.mode &&
      this.capturedScope !== JSON.stringify(selectionScope(this.scope()))
    )
      this.done(false);
  }
  done(notify = true) {
    this.mode = false;
    this.identity = this.anchor = this.capturedScope = undefined;
    this.anchorPosition = undefined;
    this.snapshot = undefined;
    this.error = this.warning = undefined;
    this.gestures = [];
    this.chosen.clear();
    this.positions.clear();
    this.aliases.clear();
    if (notify && !this.disposed) this.changed();
    void this.pump();
  }
  all() {
    this.start();
    this.enqueue({ all: true });
  }
  clear() {
    if (this.enqueue({ change: { kind: "clear" } }))
      this.anchor = this.anchorPosition = undefined;
  }
  toggle(id: string, clearOthers = false) {
    const selected = clearOthers || !this.selected(id);
    this.start();
    if (
      this.enqueue({
        change: { kind: "set", id, selected, clear_others: clearOthers },
      })
    ) {
      this.anchor = id;
      this.anchorPosition = this.positions.get(this.canonical(id));
    }
  }
  range(id: string, additive = false) {
    if (!this.mode || !this.anchor) {
      this.toggle(id, !additive);
      return;
    }
    const from =
        this.positions.get(this.canonical(this.anchor)) ?? this.anchorPosition,
      to = this.positions.get(this.canonical(id));
    this.enqueue({
      change: { kind: "range", anchor: this.anchor, target: id, additive },
      range:
        from === undefined || to === undefined
          ? undefined
          : [Math.min(from, to), Math.max(from, to)],
    });
  }
  refresh() {
    if (!this.mode) return;
    this.refreshVersion++;
    this.markDirty(this.watched);
    void this.pump();
  }
  retry() {
    this.error = undefined;
    if (this.gestures.length) this.gestures[0].failed = false;
    this.changed();
    void this.pump();
  }
  private enqueue(gesture: Gesture) {
    if (!this.mode) return false;
    if (this.gestures.length >= 32) {
      this.warning =
        "Selection is catching up. Retry that click after it finishes.";
      this.changed();
      return false;
    }
    this.warning = undefined;
    this.gestures.push(gesture);
    this.changed();
    void this.pump();
    return true;
  }
  private accept(
    result: SelectionSnapshot,
    observed: Map<string, number | undefined>,
  ) {
    this.snapshot = result;
    for (const [old, target] of Object.entries(result.aliases))
      this.aliases.set(old, target);
    const visible = new Set(result.visible);
    for (const [original, generation] of observed) {
      const id = this.canonical(original);
      this.chosen.delete(id);
      this.positions.delete(id);
      if (visible.has(id)) this.chosen.add(id);
      if (result.positions[id] !== undefined)
        this.positions.set(id, result.positions[id]);
      if (this.dirty.get(original) === generation) this.dirty.delete(original);
    }
    if (this.anchor)
      this.anchorPosition =
        this.positions.get(this.canonical(this.anchor)) ?? this.anchorPosition;
    this.prune();
  }
  private async pump() {
    if (this.running) return;
    this.running = true;
    try {
      while (true) {
        if (this.backend && this.backend !== this.identity) {
          try {
            await this.repository.selection({
              kind: "release",
              id: this.backend,
            });
            this.backend = undefined;
          } catch {
            if (!this.disposed) {
              this.error =
                "Could not close the previous selection. Retry to select messages.";
              this.changed();
            }
            break;
          }
          continue;
        }
        if (this.disposed || !this.mode || this.error) break;
        const id = this.identity!,
          gesture = this.snapshot ? this.gestures[0] : undefined;
        if (
          this.snapshot &&
          !gesture &&
          !this.dirty.size &&
          this.acknowledgedRefresh === this.refreshVersion
        )
          break;
        const observed = [
          ...(gesture || !this.snapshot
            ? new Set([...this.gestureIds, ...this.watched])
            : this.dirty.keys()),
        ].slice(0, 50);
        const versions = new Map(
          observed.map((id) => [id, this.dirty.get(id)]),
        );
        const refresh = this.refreshVersion;
        const command: SelectionCommand =
          !this.snapshot || gesture?.all
            ? {
                kind: "capture",
                id,
                revision: this.snapshot ? this.snapshot.revision + 1 : 0,
                scope: this.scope(),
                all: !!gesture?.all,
              }
            : !gesture
              ? { kind: "observe", id }
              : {
                  kind: "change",
                  id,
                  expected: this.snapshot.revision,
                  scope: this.scope(),
                  change: gesture.change,
                };
        // This unpredictable token may have committed even if its reply was lost.
        this.backend = id;
        try {
          let result: SelectionSnapshot;
          try {
            result = (await this.repository.selection(
              command,
              observed,
            )) as SelectionSnapshot;
          } catch (cause) {
            const recovered = (await this.repository.selection(
              { kind: "observe", id },
              observed,
            )) as SelectionSnapshot;
            const committed =
              command.kind === "capture"
                ? command.revision
                : command.kind === "change"
                  ? command.expected + 1
                  : this.snapshot!.revision;
            if (recovered.revision !== committed) throw cause;
            result = recovered;
          }
          if (this.identity !== id || this.disposed) continue;
          if (gesture) {
            this.gestures.shift();
            this.markDirty(
              [...this.watched].filter((id) => !observed.includes(id)),
            );
          }
          this.acknowledgedRefresh = refresh;
          this.accept(result, versions);
          this.changed();
        } catch {
          if (this.identity !== id || this.disposed) continue;
          if (gesture) gesture.failed = true;
          this.error =
            "Could not update the selection. Retry, or choose Done and select again.";
          this.changed();
          break;
        }
      }
    } finally {
      this.running = false;
    }
  }
  dispose() {
    this.disposed = true;
    this.done(false);
  }
}
