export interface AccountWork {
  id: string;
  account: string;
  eligible?: boolean;
  run(): Promise<void>;
}
export interface ActionSource {
  page(after?: string): Promise<{ rows: AccountWork[]; next?: string }>;
}

/** Admission wakes the existing owner even while every active request is held. */
export class ActionWake {
  revision = 0;
  private listeners = new Set<() => void>();
  notify() {
    this.revision++;
    for (const listener of this.listeners) listener();
    this.listeners.clear();
  }
  async wait(work: Iterable<Promise<void>>, observed: number) {
    if (observed !== this.revision) return;
    let release!: () => void;
    const changed = new Promise<void>(resolve => { release = resolve; this.listeners.add(resolve); });
    try { await Promise.race([...work, changed]); }
    finally { this.listeners.delete(release); }
  }
}

/** One owner, four provider slots, one active request per account. */
export async function runAccountActions(sources: ActionSource[], wake: ActionWake, stopping: () => boolean) {
  const running = new Set<Promise<void>>(), accounts = new Set<string>();
  const attempted = new Set<string>();
  let observed = wake.revision;
  try {
    for (;;) {
      const cursors: (string | undefined)[] = sources.map(() => undefined);
      const finished = sources.map(() => false);
      const buffered: AccountWork[][] = sources.map(() => []);
      const present = new Set<string>();
      let blocked = false, restart = false;
      do {
        for (let source = 0; source < sources.length && !stopping(); source++) {
          if (!buffered[source].length && !finished[source]) {
            const page = await sources[source].page(cursors[source]);
            cursors[source] = page.next;
            finished[source] = !page.next;
            buffered[source] = page.rows;
          }
          const work = buffered[source].shift();
          if (work) {
            const key = `${source}:${work.id}`;
            present.add(key);
            if (work.eligible === false || attempted.has(key) || stopping()) continue;
            if (accounts.has(work.account)) { blocked = true; continue; }
            while (running.size >= 4 && !stopping()) {
              const before = wake.revision;
              await wake.wait(running, before);
              if (observed !== wake.revision) { restart = true; break; }
            }
            if (stopping() || restart) break;
            attempted.add(key);
            accounts.add(work.account);
            const task = work.run().catch(() => {}).finally(() => { running.delete(task); accounts.delete(work.account); });
            running.add(task);
          }
        }
      } while (!stopping() && !restart && finished.some((done, source) => !done || buffered[source].length));
      if (restart) { observed = wake.revision; continue; }
      // Both domain journals cap unresolved admissions; retire completed identities.
      for (const key of attempted) if (!present.has(key)) attempted.delete(key);
      if (stopping()) break;
      if (observed !== wake.revision) { observed = wake.revision; continue; }
      if (!running.size) {
        if (blocked) continue;
        break;
      }
      await wake.wait(running, observed);
      observed = wake.revision;
    }
  } finally { await Promise.all(running); }
}
