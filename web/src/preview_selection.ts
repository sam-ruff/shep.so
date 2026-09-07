// Synthetic preview/test transport only. Production uses SelectionWorkerClient;
// this module and its small in-memory fixture sets are not in the app bundle.
import type { Mail } from "./model";
import { mailMatches } from "./mail_query";
import {
  selectionScope,
  type SelectionCommand,
  type SelectionScope,
  type SelectionSnapshot,
  type SelectionGroup,
  type SelectionRepository,
  type SelectionResult,
} from "./selection_types";
interface Capture {
  scope: string;
  revision: number;
  frozen: boolean;
  positions: Map<string, number>;
  selected: Set<string>;
}
export class PreviewSelection implements SelectionRepository {
  captures = new Map<string, Capture>();
  aliases = new Map<string, string>();
  constructor(
    private mail: () => Mail[],
    private folders: () => Map<string, Set<string>> = () => new Map(),
  ) {}
  private canonical(id: string) {
    return this.aliases.get(id) ?? id;
  }
  private matching(scope: SelectionScope) {
    return this.mail()
      .map((m) => ({ ...m, ...scope.projection?.[m.id] }))
      .filter(
        (m) =>
          (!scope.account || scope.account === (m.accountId || m.account)) &&
          mailMatches(m, scope, this.folders().get(m.accountId || m.account)),
      )
      .sort(
        (a, b) =>
          (scope.oldest
            ? a.date.localeCompare(b.date)
            : b.date.localeCompare(a.date)) ||
          (a.id < b.id ? -1 : a.id > b.id ? 1 : 0),
      );
  }
  async selection(
    command: SelectionCommand,
    observed: string[] = [],
  ): Promise<SelectionResult> {
    if (observed.length > 50) throw Error("One observed page");
    const { id } = command;
    if (command.kind === "release") {
      this.captures.delete(id);
      return null;
    }
    if (command.kind === "capture") {
      const old = this.captures.get(id);
      if (old && (old.frozen || old.revision >= command.revision))
        throw Error("Stale capture");
      const rows = this.matching(command.scope);
      this.captures.set(id, {
        scope: JSON.stringify(selectionScope(command.scope)),
        revision: command.revision,
        frozen: false,
        positions: new Map(rows.map((m, i) => [m.id, i])),
        selected: new Set(command.all ? rows.map((m) => m.id) : []),
      });
    }
    const capture = this.captures.get(id);
    if (!capture) throw Error("Missing capture");
    for (const [original, rank] of capture.positions) {
      const target = this.canonical(original);
      if (target === original) continue;
      capture.positions.delete(original);
      capture.positions.set(
        target,
        Math.min(rank, capture.positions.get(target) ?? rank),
      );
      if (capture.selected.delete(original)) capture.selected.add(target);
    }
    if ("expected" in command && capture.revision !== command.expected)
      throw Error("Stale selection");
    if (command.kind === "change") {
      if (
        capture.frozen ||
        capture.scope !== JSON.stringify(selectionScope(command.scope))
      )
        throw Error("Stale selection");
      const c = command.change;
      if (c.kind === "clear") capture.selected.clear();
      else if (c.kind === "set") {
        const target = this.canonical(c.id);
        if (!capture.positions.has(target)) {
          if (!this.matching(command.scope).some((m) => m.id === target))
            throw Error("Outside scope");
          capture.positions.set(
            target,
            Math.max(-1, ...capture.positions.values()) + 1,
          );
        }
        if (c.clear_others) capture.selected.clear();
        if (c.selected) capture.selected.add(target);
        else capture.selected.delete(target);
      } else {
        const from = capture.positions.get(this.canonical(c.anchor)),
          to = capture.positions.get(this.canonical(c.target));
        if (from === undefined || to === undefined)
          throw Error("Outside capture");
        if (!c.additive) capture.selected.clear();
        for (const [id, rank] of capture.positions)
          if (rank >= Math.min(from, to) && rank <= Math.max(from, to))
            capture.selected.add(id);
      }
      capture.revision++;
    }
    if (command.kind === "freeze") {
      if (this.captures.has(command.target)) throw Error("Existing review");
      this.captures.set(command.target, {
        ...capture,
        revision: 0,
        frozen: true,
        positions: new Map(
          [...capture.positions].filter(([id]) => capture.selected.has(id)),
        ),
        selected: new Set(capture.selected),
      });
      return this.snapshot(command.target, observed);
    }
    if (command.kind === "page") {
      const mail = new Map(this.mail().map((m) => [m.id, m]));
      const rows = [...capture.positions]
        .filter(
          ([id, position]) =>
            capture.selected.has(id) &&
            mail.has(id) &&
            position > (command.after ?? -1),
        )
        .sort((a, b) => a[1] - b[1])
        .slice(0, 50)
        .map(([id, position]) => {
          const m = mail.get(id)!;
          return {
            id,
            position,
            account: m.accountId || m.account,
            folder: m.folder,
            unread: m.unread,
            starred: m.starred,
          };
        });
      return {
        revision: capture.revision,
        rows,
        next_after: rows.length === 50 ? rows.at(-1)!.position : null,
      };
    }
    return this.snapshot(id, observed);
  }
  private snapshot(id: string, observed: string[]): SelectionSnapshot {
    const c = this.captures.get(id)!,
      available = this.mail().filter((m) => c.selected.has(m.id));
    const groups = new Map<string, SelectionGroup>();
    for (const m of available) {
      const account = m.accountId || m.account,
        key = JSON.stringify([account, m.folder]);
      const g = groups.get(key) ?? {
        account,
        folder: m.folder,
        total: 0,
        unread: 0,
        starred: 0,
      };
      g.total++;
      g.unread += +m.unread;
      g.starred += +m.starred;
      groups.set(key, g);
    }
    const ids = new Set(available.map((m) => m.id));
    return {
      id,
      revision: c.revision,
      frozen: c.frozen,
      total: c.positions.size,
      selected: c.selected.size,
      available: available.length,
      unread: available.filter((m) => m.unread).length,
      starred: available.filter((m) => m.starred).length,
      groups: [...groups.values()],
      visible: [
        ...new Set(
          observed.map((id) => this.canonical(id)).filter((id) => ids.has(id)),
        ),
      ],
      positions: Object.fromEntries(
        observed
          .map((id) => this.canonical(id))
          .filter((id) => c.positions.has(id))
          .map((id) => [id, c.positions.get(id)!]),
      ),
      aliases: Object.fromEntries(
        observed
          .filter((id) => this.canonical(id) !== id)
          .map((id) => [id, this.canonical(id)]),
      ),
    };
  }
}
