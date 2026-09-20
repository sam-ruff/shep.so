import { BrowserCalendar, executeCalendar, validCalendarEvent, type CalendarAction, type CalendarMutation, type CalendarReply, type CalendarSource, type ProviderEvent } from "./calendar_actions";
import type { CalendarEntry } from "./model";
import type { ActionSource } from "./action_scheduler";

type Exclusive = <T>(scope: string, work: () => Promise<T>, wait?: boolean) => Promise<T>;
type Reply = { state: string; value?: { sources?: CalendarSource[]; events?: ProviderEvent[]; current?: ProviderEvent | null; receipt?: import("./calendar_actions").CalendarReceipt }; error?: string };

export class CalendarRepository {
  sources: CalendarSource[] = [];
  events: CalendarEntry[] = [];
  source = "";
  error = "";
  pending = 0;
  attention = 0;
  private active = new Set<string>();
  private window?: [string, string];
  private refreshGeneration = 0;
  private reloadGeneration = 0;
  constructor(
    readonly journal: BrowserCalendar,
    private readonly owner: string,
    private readonly request: (operation: object) => Promise<unknown>,
    private readonly exclusive: Exclusive,
    private readonly stopping: () => boolean,
    private readonly changed: () => void,
  ) {}
  private async read(operation: object) {
    const reply = await this.request(operation) as Reply;
    if (reply?.state !== "observed" || !reply.value) throw Error(reply?.error ?? "Calendar could not be loaded. Retry or reconnect Google in Preferences.");
    return reply.value;
  }
  async load() {
    this.sources = await this.journal.sources();
    if (!this.sources.some(source => source.id === this.source)) this.source = this.sources[0]?.id ?? "";
    await this.reload();
  }
  async reload() {
    const generation = ++this.reloadGeneration, window = this.window, source = this.source;
    const current = () => generation === this.reloadGeneration && window === this.window && source === this.source;
    for (let attempt = 0; attempt < 2; attempt++) {
      const revision = await this.journal.observationRevision();
      if (!current()) return;
      const summary = await this.journal.summary();
      if (!current()) return;
      const rows = window && source ? await this.journal.view(source, ...window) : [];
      if (!current()) return;
      const latest = await this.journal.observationRevision();
      if (!current()) return;
      if (revision !== latest) continue;
      const name = this.sources.find(item => item.id === source);
      this.events = rows.map(row => ({ id: row.event.id, localKey: row.key, provider: row.event,
        title: row.event.title, start: row.event.start, end: row.event.end, calendar: name?.name ?? "Calendar",
        location: row.event.location, readOnly: name?.read_only ?? true }));
      this.pending = summary.pending; this.attention = summary.attention;
      return;
    }
  }
  async show(start: string, end: string, source = this.source) {
    this.refreshGeneration++;
    this.window = [start, end]; this.source = source;
    await this.reload();
  }
  async refresh(start: string, end: string, source = this.source) {
    const generation = ++this.refreshGeneration;
    this.window = [start, end];
    try {
      const sourceRevision = await this.journal.observationRevision();
      if (generation !== this.refreshGeneration || this.stopping()) return;
      const listed = await this.read({ kind: "sources" });
      if (generation !== this.refreshGeneration || this.stopping()) return;
      if (!Array.isArray(listed.sources)) throw Error("Google returned no confirmed calendar list.");
      await this.journal.saveSources(listed.sources, sourceRevision);
      if (generation !== this.refreshGeneration || this.stopping()) return;
      this.sources = listed.sources;
      this.source = this.sources.some(item => item.id === source) ? source : this.sources[0]?.id ?? "";
      if (this.source) {
        const revision = await this.journal.observationRevision();
        if (generation !== this.refreshGeneration || this.stopping()) return;
        const result = await this.read({ kind: "events", source_id: this.source, start, end });
        if (generation !== this.refreshGeneration || this.stopping()) return;
        if (!Array.isArray(result.events)) throw Error("Google returned no confirmed event list.");
        await this.journal.sync(this.source, start, end, result.events, revision);
        if (generation !== this.refreshGeneration || this.stopping()) return;
      }
      await this.reload();
      if (generation !== this.refreshGeneration || this.stopping()) return;
      this.error = "";
    } catch (error) {
      if (generation !== this.refreshGeneration) return;
      this.error = error instanceof Error ? error.message : "Calendar could not be refreshed.";
      throw error;
    } finally { if (generation === this.refreshGeneration) this.changed(); }
  }
  async save(entry: CalendarEntry, before: CalendarEntry | null, id: string) {
    if (!entry.provider || before && (!before.provider || before.localKey !== entry.localKey)) throw Error("Refresh this event before editing it. Your entered text is still available.");
    const after = { ...entry.provider, title: entry.title, location: entry.location, start: entry.start, end: entry.end };
    const mutation: CalendarMutation = { save: { before: before?.provider ?? null, after } };
    const action = await this.journal.admit({ id, key: entry.localKey ?? crypto.randomUUID(), owner: this.owner, mutation });
    await this.reload(); this.changed(); return action;
  }
  async remove(entry: CalendarEntry, id: string) {
    if (!entry.provider || !entry.localKey) throw Error("Refresh this event before deleting it.");
    const action = await this.journal.admit({ id, key: entry.localKey, owner: this.owner, mutation: { delete: { before: entry.provider } } });
    await this.reload(); this.changed(); return action;
  }
  private runnable(action: CalendarAction) {
    return !this.active.has(action.id) && ["Queued", "Waiting", "Running", "Repair"].includes(action.status) && !action.observation;
  }
  sourceWork(): ActionSource {
    return { page: async after => {
      const page = await this.journal.page(after);
      const rows = await Promise.all(page.rows.map(async action => {
        const parent = action.dependency ? await this.journal.get(action.dependency) : undefined;
        const ready = !action.dependency || !parent || ["Succeeded", "Rejected", "Dismissed", "Cancelled"].includes(parent.status);
        return { id: action.id, account: `calendar:${action.source}`, eligible: this.runnable(action) && ready, run: () => this.run(action) };
      }));
      return { next: page.next, rows };
    } };
  }
  private async run(expected: CalendarAction) {
    const run = () => this.exclusive(`calendar.${expected.source}`, async () => {
      if (this.stopping() || this.active.has(expected.id)) return;
      const current = await this.journal.get(expected.id);
      if (!current || current.revision !== expected.revision) return;
      this.active.add(expected.id);
      try {
        if (current.status === "Running") { await this.journal.recoverAbandoned(current); return; }
        await executeCalendar(this.journal, current, this.owner, { mutate: async (id, mutation): Promise<CalendarReply> => {
          const reply = await this.request({ kind: "mutate", request_id: id, mutation }) as Reply;
          if (reply?.state === "acknowledged" && reply.value?.receipt) return { state: "acknowledged", receipt: reply.value.receipt };
          if (reply && ["waiting", "rejected", "uncertain"].includes(reply.state)) return { state: reply.state as "waiting" | "rejected" | "uncertain", error: reply.error ?? "Review this Calendar change." };
          throw Error("The Calendar response could not be confirmed.");
        } }, this.stopping);
      } finally {
        this.active.delete(expected.id); await this.reload(); this.changed();
      }
    }, false);
    try {
      if (expected.owner !== this.owner) await this.exclusive(`actions.tab.${expected.owner}`, run, false);
      else await run();
    } catch { /* Another tab or a failed local write retains ownership. */ }
  }
  async decide(expected: CalendarAction, decision: "retry" | "repair" | "check" | "adopt" | "cancel") {
    if (decision === "cancel") {
      const result = await this.journal.cancel(expected);
      await this.reload(); this.changed(); return result;
    }
    const work = () => this.exclusive(`calendar.${expected.source}`, async () => {
      if (this.active.has(expected.id)) throw Error("Wait for the current Calendar request to finish.");
      if (decision === "retry") return this.journal.retry(expected);
      if (decision === "repair") return this.journal.repair(expected);
      if (decision === "adopt") return this.journal.adoptObserved(expected);
      const mutation = expected.dispatch ?? expected.requested;
      const revision = await this.journal.observationRevision();
      const value = await this.read({ kind: "inspect", request_id: expected.id, mutation });
      if (!("current" in value) || value.current !== null && !validCalendarEvent(value.current!)) throw Error("The exact Calendar event could not be inspected.");
      return this.journal.observe(expected, value.current ?? null, revision);
    }, false);
    const result = expected.owner === this.owner ? await work() : await this.exclusive(`actions.tab.${expected.owner}`, work, false);
    await this.reload(); this.changed(); return result;
  }
}
