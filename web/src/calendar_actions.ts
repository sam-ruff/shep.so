export interface CalendarSource { id: string; name: string; read_only: boolean }
export interface ProviderEvent {
  id: string; source_id: string; title: string; start: string; end: string;
  location: string; description: string; all_day: boolean;
  etag: string | null; remote_url: string | null;
}
export type CalendarMutation = { save: { before: ProviderEvent | null; after: ProviderEvent } } | { delete: { before: ProviderEvent } };
export interface CalendarReceipt { request_id: string; before: ProviderEvent | null; after: ProviderEvent | null }
export type CalendarStatus = "Queued" | "Waiting" | "Running" | "Uncertain" | "Rejected" | "Repair" | "Succeeded" | "Dismissed" | "Cancelled";
export interface CalendarRecord { key: string; event: ProviderEvent; revision: number }
export interface CalendarAction {
  id: string; key: string; source: string; sequence: number; revision: number;
  owner: string; status: CalendarStatus; active: 0 | 1;
  requested: CalendarMutation; dispatch?: CalendarMutation; dependency?: string;
  receipt?: CalendarReceipt; error?: string;
  observation?: { current: ProviderEvent | null; revision: number };
}
export interface CalendarAdmission { id: string; key: string; owner: string; mutation: CalendarMutation }
export function calendarStatus(job: CalendarAction): string {
  return ({ Queued: "Waiting to start", Waiting: "Waiting for Calendar access", Running: "Syncing event", Uncertain: "Needs checking", Rejected: "Needs review", Repair: "Saving on this browser", Succeeded: "Complete", Dismissed: "Checked state adopted", Cancelled: "Cancelled before syncing" })[job.status];
}
export type CalendarReply = { state: "acknowledged"; receipt: CalendarReceipt } | { state: "waiting" | "rejected" | "uncertain"; error: string };
export interface CalendarProvider { mutate(id: string, mutation: CalendarMutation): Promise<CalendarReply> }
export const calendarBefore = (mutation: CalendarMutation) => "save" in mutation ? mutation.save.before : mutation.delete.before;
export const calendarAfter = (mutation: CalendarMutation) => "save" in mutation ? mutation.save.after : null;
export const calendarSource = (mutation: CalendarMutation) => (calendarAfter(mutation) ?? calendarBefore(mutation))!.source_id;
const equal = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const request = <T>(value: IDBRequest<T>) => new Promise<T>((resolve, reject) => {
  value.onsuccess = () => resolve(value.result); value.onerror = () => reject(value.error);
});
export function calendarIdentity(before: ProviderEvent, after: ProviderEvent) {
  return before.id === after.id && before.source_id === after.source_id && before.etag === after.etag && before.remote_url === after.remote_url;
}
export function validCalendarEvent(event: ProviderEvent): boolean {
  const bytes = (value: unknown, max: number) => typeof value === "string" && new TextEncoder().encode(value).length <= max;
  return !!event && !!event.id && !!event.source_id && bytes(event.id, 1024) && bytes(event.source_id, 1024) &&
    bytes(event.title, 1024) && bytes(event.location, 4096) && bytes(event.description, 65536) &&
    typeof event.all_day === "boolean" && (event.etag === null || bytes(event.etag, 4096)) &&
    (event.remote_url === null || bytes(event.remote_url, 8192)) && Number.isFinite(Date.parse(event.start)) && Date.parse(event.end) > Date.parse(event.start);
}
function validate(mutation: CalendarMutation) {
  const before = calendarBefore(mutation), after = calendarAfter(mutation);
  if (before && !validCalendarEvent(before) || after && !validCalendarEvent(after) || !before && !after ||
    before && after && !calendarIdentity(before, after) || !before && after && (after.etag !== null || after.remote_url !== null)) {
    throw Error("The event identity or content changed. Keep your edits and refresh the saved event.");
  }
}

/** Calendar receipts and cached events share the profile database transaction. */
export class BrowserCalendar {
  constructor(private readonly db: IDBDatabase) {}
  private transaction<T>(mode: IDBTransactionMode, work: (tx: IDBTransaction) => Promise<T>) {
    return new Promise<T>((resolve, reject) => {
      const tx = this.db.transaction(["calendarSources", "calendarEvents", "calendarActions", "calendarState"], mode, { durability: "strict" });
      let result: T, failure: unknown;
      tx.oncomplete = () => resolve(result);
      tx.onabort = () => reject(failure ?? Error("The Calendar change could not be saved on this browser. Keep your edits and retry."));
      void work(tx).then(value => { result = value; }, error => {
        failure = error;
        try { tx.abort(); } catch { reject(error); }
      });
    });
  }
  private async clock(tx: IDBTransaction, advance = false): Promise<number> {
    const store = tx.objectStore("calendarState");
    const revision = (await request<number | undefined>(store.get("revision")) ?? 0) + Number(advance);
    if (advance) store.put(revision, "revision");
    return revision;
  }
  private async current(tx: IDBTransaction, expected: CalendarAction) {
    const current = await request<CalendarAction | undefined>(tx.objectStore("calendarActions").get(expected.id));
    if (!current || current.revision !== expected.revision) throw Error("This Calendar change advanced. Review its current state.");
    return current;
  }
  private async latest(tx: IDBTransaction, key: string): Promise<CalendarAction | undefined> {
    const cursor = await request(tx.objectStore("calendarActions").index("eventSequence").openCursor(IDBKeyRange.bound([key, 0], [key, Number.MAX_SAFE_INTEGER]), "prev"));
    return cursor?.value;
  }
  get(id: string) { return this.transaction("readonly", tx => request<CalendarAction | undefined>(tx.objectStore("calendarActions").get(id))); }
  currentEvent(key: string) { return this.transaction("readonly", tx => request<CalendarRecord | undefined>(tx.objectStore("calendarEvents").get(key))); }
  summary() {
    return this.transaction("readonly", async tx => {
      const store = tx.objectStore("calendarActions");
      const pending = await request(store.index("activeId").count(IDBKeyRange.bound([1, ""], [1, "\uffff"])));
      const attention = await request(store.index("status").count("Uncertain")) + await request(store.index("status").count("Rejected")) + await request(store.index("status").count("Repair"));
      return { pending, attention };
    });
  }
  page(after?: string, completed = false) {
    return this.transaction("readonly", async tx => {
      const active = completed ? 0 : 1;
      const rows = await request<CalendarAction[]>(tx.objectStore("calendarActions").index("activeId").getAll(IDBKeyRange.bound([active, after ?? ""], [active, "\uffff"], !!after), 21));
      const more = rows.length > 20; rows.length = Math.min(20, rows.length);
      return { rows, next: more ? rows.at(-1)!.id : undefined };
    });
  }
  admit(input: CalendarAdmission) {
    validate(input.mutation);
    if (!/^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/.test(input.id) || !input.key || input.key.length > 2048) throw Error("The saved Calendar request identity is invalid.");
    return this.transaction("readwrite", async tx => {
      const actions = tx.objectStore("calendarActions");
      const previous = await request<CalendarAction | undefined>(actions.get(input.id));
      if (previous) {
        if (previous.key !== input.key || !equal(previous.requested, input.mutation)) throw Error("This Calendar request identity is already in use.");
        return previous;
      }
      const source = await request<CalendarSource | undefined>(tx.objectStore("calendarSources").get(calendarSource(input.mutation)));
      if (!source || source.read_only) throw Error("Refresh a writable calendar before saving this event.");
      const count = await request(actions.index("activeId").count(IDBKeyRange.bound([1, ""], [1, "\uffff"])));
      if (count >= 100) throw Error("Review pending Calendar changes before adding more.");
      const latest = await this.latest(tx, input.key);
      const pending = latest && (latest.active === 1 && latest.status !== "Rejected" || latest.status === "Succeeded" && equal(calendarAfter(latest.requested), calendarBefore(input.mutation))) ? latest : undefined;
      const cached = await request<CalendarRecord | undefined>(tx.objectStore("calendarEvents").get(input.key));
      const observed = pending ? calendarAfter(pending.requested) : cached?.event ?? null;
      if (!equal(observed, calendarBefore(input.mutation))) throw Error("The event changed after you opened it. Keep your edits and review the current event.");
      const revision = await this.clock(tx, true);
      const next: CalendarAction = { id: input.id, key: input.key, owner: input.owner, source: source.id,
        sequence: revision, revision, status: "Queued", active: 1, requested: structuredClone(input.mutation), dependency: pending?.id };
      if (latest?.status === "Rejected") actions.put({ ...latest, status: "Dismissed", active: 0, revision }, latest.id);
      actions.put(next, next.id);
      return next;
    });
  }
  claim(expected: CalendarAction, owner: string) {
    return this.transaction("readwrite", async tx => {
      const job = await this.current(tx, expected);
      if (!["Queued", "Waiting"].includes(job.status)) return undefined;
      const source = await request<CalendarSource | undefined>(tx.objectStore("calendarSources").get(job.source));
      if (!source || source.read_only) throw Error("Refresh this calendar's permissions before continuing.");
      let mutation = structuredClone(job.requested);
      if (job.dependency) {
        const parent = await request<CalendarAction | undefined>(tx.objectStore("calendarActions").get(job.dependency));
        if (!parent || ["Rejected", "Dismissed", "Cancelled"].includes(parent.status)) throw Error("The preceding change needs review. Reopen the current event and keep these edits for comparison.");
        if (!parent || parent.status !== "Succeeded") return undefined;
        const actual = parent.receipt?.after;
        if (!actual) throw Error("The preceding event change has no current event. Review this saved edit.");
        mutation = "save" in mutation ? { save: { before: actual, after: { ...mutation.save.after,
          id: actual.id, source_id: actual.source_id, etag: actual.etag, remote_url: actual.remote_url } } } : { delete: { before: actual } };
      }
      const cached = await request<CalendarRecord | undefined>(tx.objectStore("calendarEvents").get(job.key));
      if (!equal(cached?.event ?? null, calendarBefore(mutation))) throw Error("The saved event changed before dispatch. Review this Calendar change.");
      const before = calendarBefore(mutation);
      if (before && !before.etag) throw Error("Refresh this event's version before editing or deleting it.");
      const next: CalendarAction = { ...job, owner, dispatch: mutation, status: "Running", error: undefined, revision: await this.clock(tx, true) };
      tx.objectStore("calendarActions").put(next, next.id); return next;
    });
  }
  outcome(expected: CalendarAction, status: "Waiting" | "Rejected" | "Uncertain", error: string) {
    return this.transaction("readwrite", async tx => {
      const job = await this.current(tx, expected);
      const next: CalendarAction = { ...job, status, error, revision: await this.clock(tx, true) };
      tx.objectStore("calendarActions").put(next, next.id); return next;
    });
  }
  retainError(expected: CalendarAction, error: string) {
    return this.transaction("readwrite", async tx => {
      const job = await this.current(tx, expected);
      const next = { ...job, error, revision: await this.clock(tx, true) };
      tx.objectStore("calendarActions").put(next, next.id); return next;
    });
  }
  acknowledge(expected: CalendarAction, receipt: CalendarReceipt) {
    return this.transaction("readwrite", async tx => {
      const job = await this.current(tx, expected);
      if (job.status !== "Running" || !job.dispatch || receipt.request_id !== job.id || !equal(receipt.before, calendarBefore(job.dispatch))) throw Error("The Calendar receipt does not match the dispatched request.");
      const after = calendarAfter(job.dispatch);
      const expectedId = calendarBefore(job.dispatch)?.id ?? `shep${job.id.replaceAll("-", "")}`;
      if (!!after !== !!receipt.after || receipt.after && (!validCalendarEvent(receipt.after) || receipt.after.source_id !== job.source || receipt.after.id !== expectedId || !receipt.after.etag)) throw Error("The Calendar receipt has no confirmed event version.");
      const next: CalendarAction = { ...job, receipt, status: "Repair", error: undefined, revision: await this.clock(tx, true) };
      tx.objectStore("calendarActions").put(next, next.id); return next;
    });
  }
  repair(expected: CalendarAction) {
    return this.transaction("readwrite", async tx => {
      const job = await this.current(tx, expected);
      if (job.status !== "Repair" || !job.receipt) throw Error("This Calendar change has no acknowledged receipt.");
      const events = tx.objectStore("calendarEvents");
      const cached = await request<CalendarRecord | undefined>(events.get(job.key));
      if (!equal(cached?.event ?? null, job.receipt.before)) throw Error("The cached event changed. Check the server before adopting its current state.");
      const revision = await this.clock(tx, true);
      if (job.receipt.after) events.put({ key: job.key, event: job.receipt.after, revision } satisfies CalendarRecord, job.key);
      else events.delete(job.key);
      const next: CalendarAction = { ...job, status: "Succeeded", active: 0, revision };
      tx.objectStore("calendarActions").put(next, next.id); return next;
    });
  }
  recoverAbandoned(expected: CalendarAction) {
    if (expected.status !== "Running") return Promise.resolve(expected);
    return this.outcome(expected, "Uncertain", "The provider request was interrupted. Check the saved event before making another change.");
  }
  cancel(expected: CalendarAction) {
    return this.transaction("readwrite", async tx => {
      const job = await this.current(tx, expected);
      if (!["Queued", "Waiting"].includes(job.status)) throw Error("This Calendar change has already started. Check its result before making another decision.");
      const next: CalendarAction = { ...job, status: "Cancelled", active: 0, error: undefined, revision: await this.clock(tx, true) };
      tx.objectStore("calendarActions").put(next, next.id); return next;
    });
  }
  retry(expected: CalendarAction) {
    return this.transaction("readwrite", async tx => {
      const job = await this.current(tx, expected);
      if (!["Waiting", "Rejected"].includes(job.status) || (await this.latest(tx, job.key))?.id !== job.id) throw Error("Review the latest saved change before retrying.");
      const next: CalendarAction = { ...job, status: "Queued", error: undefined, observation: undefined, revision: await this.clock(tx, true) };
      tx.objectStore("calendarActions").put(next, next.id); return next;
    });
  }
  observe(expected: CalendarAction, current: ProviderEvent | null, observedRevision: number) {
    return this.transaction("readwrite", async tx => {
      const job = await this.current(tx, expected);
      if (observedRevision !== await this.clock(tx)) throw Error("Calendar changed while the server was being checked. Check it again before adopting its state.");
      if (!["Uncertain", "Repair", "Rejected"].includes(job.status)) throw Error("This Calendar change is still running.");
      const mutation = job.dispatch ?? job.requested;
      const before = calendarBefore(mutation), after = calendarAfter(mutation);
      const id = before?.id ?? `shep${job.id.replaceAll("-", "")}`;
      if (current && (!validCalendarEvent(current) || current.source_id !== job.source || current.id !== id || !current.etag)) throw Error("The server did not confirm this exact event and its version.");
      if (!before && !after) throw Error("This Calendar change has no event identity.");
      const revision = await this.clock(tx, true);
      const next: CalendarAction = { ...job, revision, observation: { current, revision } };
      tx.objectStore("calendarActions").put(next, next.id); return next;
    });
  }
  adoptObserved(expected: CalendarAction) {
    return this.transaction("readwrite", async tx => {
      const job = await this.current(tx, expected);
      if (!job.observation || job.observation.revision !== await this.clock(tx)) throw Error("Calendar changed after this review. Check the server again before adopting its state.");
      const revision = await this.clock(tx, true);
      const events = tx.objectStore("calendarEvents"), current = job.observation.current;
      if (current) events.put({ key: job.key, event: current, revision } satisfies CalendarRecord, job.key);
      else events.delete(job.key);
      const error = job.receipt ? "The acknowledged receipt was retained and the checked current server state was adopted." : job.status === "Rejected" ? "The refused change was retained and the checked current server state was adopted." : "The checked server state was adopted. The original provider outcome remains unconfirmed.";
      const next: CalendarAction = { ...job, revision, status: "Dismissed", active: 0, error };
      tx.objectStore("calendarActions").put(next, next.id); return next;
    });
  }
  async observationRevision() { return this.transaction("readonly", tx => this.clock(tx)); }
  sources() { return this.transaction("readonly", tx => request<CalendarSource[]>(tx.objectStore("calendarSources").getAll(undefined, 51))); }
  saveSources(sources: CalendarSource[], observedRevision?: number) {
    if (sources.length > 50 || new Set(sources.map(source => source.id)).size !== sources.length || sources.some(source => !source.id || source.id.length > 1024 || source.name.length > 1024 || typeof source.read_only !== "boolean")) throw Error("The calendar list exceeded its bound or contained invalid identities.");
    return this.transaction("readwrite", async tx => {
      if (observedRevision !== undefined && observedRevision !== await this.clock(tx)) throw Error("Calendar changed while sources were loading. Refresh again to keep the current permissions.");
      const store = tx.objectStore("calendarSources");
      store.clear(); for (const source of sources) store.put(source, source.id);
      await this.clock(tx, true);
    });
  }
  sync(source: string, start: string, end: string, events: ProviderEvent[], observedRevision: number) {
    if (!Number.isFinite(Date.parse(start)) || Date.parse(end) <= Date.parse(start) || Date.parse(end) - Date.parse(start) > 366 * 86400000) throw Error("Choose a calendar window of at most one year.");
    if (events.length > 5000 || events.some(event => !validCalendarEvent(event) || event.source_id !== source) || new Set(events.map(event => event.id)).size !== events.length) throw Error("The calendar response contained invalid or repeated events.");
    return this.transaction("readwrite", async tx => {
      if (await this.clock(tx) !== observedRevision) throw Error("Calendar changed while refresh was running. Refresh again to keep the newer changes.");
      const store = tx.objectStore("calendarEvents");
      const existing = await request<CalendarRecord[]>(store.index("source").getAll(source, 5001));
      if (existing.length > 5000) throw Error("This calendar cache has reached its event bound. Choose a smaller calendar set.");
      const pending = await request<CalendarAction[]>(tx.objectStore("calendarActions").index("activeId").getAll(IDBKeyRange.bound([1, ""], [1, "\uffff"]), 101));
      const protectedKeys = new Set(pending.filter(job => !["Rejected"].includes(job.status)).map(job => job.key));
      const byId = new Map(existing.map(record => [record.event.id, record]));
      const reserved = new Map(pending.filter(job => !calendarBefore(job.requested)).map(job => [`shep${job.id.replaceAll("-", "")}`, job.key]));
      const arriving = new Set(events.map(event => event.id));
      const revision = await this.clock(tx, true);
      let count = existing.length;
      for (const record of existing) {
        if (!protectedKeys.has(record.key) && !arriving.has(record.event.id) && Date.parse(record.event.start) < Date.parse(end) && Date.parse(record.event.end) > Date.parse(start)) { store.delete(record.key); count--; }
      }
      for (const event of events) {
        const key = byId.get(event.id)?.key ?? reserved.get(event.id) ?? `${source.length}:${source}${event.id}`;
        if (!protectedKeys.has(key)) {
          if (!byId.has(event.id)) count++;
          store.put({ key, event, revision } satisfies CalendarRecord, key);
        }
      }
      if (count > 5000) throw Error("This calendar cache has reached its event bound. Choose a smaller calendar set.");
    });
  }
  view(source: string, start: string, end: string) {
    return this.transaction("readonly", async tx => {
      const cached = await request<CalendarRecord[]>(tx.objectStore("calendarEvents").index("source").getAll(source, 5001));
      if (cached.length > 5000) throw Error("This calendar cache is larger than the supported view.");
      const rows = new Map(cached.map(record => [record.key, record]));
      const pending = await request<CalendarAction[]>(tx.objectStore("calendarActions").index("activeId").getAll(IDBKeyRange.bound([1, ""], [1, "\uffff"]), 101));
      if (pending.length > 100) throw Error("Review pending Calendar changes before loading more.");
      for (const job of pending.sort((a, b) => a.sequence - b.sequence)) {
        if (job.source !== source || job.status === "Rejected") continue;
        const event = calendarAfter(job.requested);
        if (event) rows.set(job.key, { key: job.key, event, revision: job.revision });
        else rows.delete(job.key);
      }
      return [...rows.values()].filter(row => Date.parse(row.event.start) < Date.parse(end) && Date.parse(row.event.end) > Date.parse(start));
    });
  }
}

export async function executeCalendar(journal: BrowserCalendar, initial: CalendarAction, owner: string, provider: CalendarProvider, stopping: () => boolean) {
  let job = initial;
  const message = (error: unknown) => error instanceof Error ? error.message : "The Calendar change needs review.";
  if (stopping()) return job;
  if (job.status === "Repair") {
    try { return await journal.repair(job); }
    catch (error) { return journal.retainError(job, message(error)); }
  }
  if (!["Queued", "Waiting"].includes(job.status)) return job;
  try {
    const claimed = await journal.claim(job, owner);
    if (!claimed) return job;
    job = claimed;
  } catch (error) { return journal.outcome(job, "Rejected", message(error)); }
  // A durable Running claim owns the attempt even if close arrives now.
  let reply: CalendarReply;
  try { reply = await provider.mutate(job.id, job.dispatch!); }
  catch { return journal.outcome(job, "Uncertain", "The Calendar request may have reached the server. Check this saved change before retrying."); }
  if (reply.state !== "acknowledged") {
    const status = reply.state === "waiting" ? "Waiting" : reply.state === "rejected" ? "Rejected" : "Uncertain";
    return journal.outcome(job, status, reply.error);
  }
  try { job = await journal.acknowledge(job, reply.receipt); }
  catch { return journal.outcome(job, "Uncertain", "The provider replied, but its exact receipt could not be saved. Check the saved event before continuing."); }
  try { return await journal.repair(job); }
  catch (error) { return journal.retainError(job, message(error)); }
}
