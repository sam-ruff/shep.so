export interface SearchHit {
  block: number;
  start: number;
  end: number;
}
export type SearchText = (
  blocks: string[],
  query: string,
  matchCase: boolean,
) => Promise<SearchHit[]>;
export class MessageFind extends EventTarget {
  query = "";
  identity = "";
  blocks: string[] = [];
  hits: SearchHit[] = [];
  open = false;
  matchCase = false;
  pending = false;
  active = 0;
  revision = 0;
  jump = 0;
  error: string | null = null;
  private running = false;
  private ready = false;
  private timer?: ReturnType<typeof setTimeout>;
  private disposed = false;
  constructor(
    private search: SearchText,
    private debounce = 100,
  ) {
    super();
  }
  get status() {
    return this.pending
      ? "Searching…"
      : !this.query
        ? "Search this message"
        : !this.hits.length
          ? "No matches"
          : `${this.active + 1} of ${this.hits.length}`;
  }
  private changed() {
    this.dispatchEvent(new Event("change"));
  }
  setSource(id: string, blocks: string[]) {
    if (
      this.identity === id &&
      blocks.length === this.blocks.length &&
      blocks.every((s, i) => s === this.blocks[i])
    )
      return;
    this.identity = id;
    this.blocks = [...blocks];
    this.invalidate();
  }
  show() {
    this.open = true;
    this.invalidate();
  }
  close() {
    this.open = false;
    this.invalidate();
  }
  setQuery(value: string) {
    this.query = value;
    this.invalidate();
  }
  toggleCase() {
    this.matchCase = !this.matchCase;
    this.invalidate();
  }
  retry() {
    this.invalidate();
  }
  next(previous = false) {
    if (!this.hits.length || this.pending) return;
    this.active =
      (this.active + (previous ? -1 : 1) + this.hits.length) % this.hits.length;
    this.jump++;
    this.changed();
  }
  private invalidate() {
    this.revision++;
    this.hits = [];
    this.active = 0;
    this.error = null;
    clearTimeout(this.timer);
    this.ready = false;
    this.pending = this.open && !!this.query;
    if (this.pending)
      this.timer = setTimeout(() => {
        this.ready = true;
        void this.pump();
      }, this.debounce);
    this.changed();
  }
  private async pump() {
    if (this.disposed || this.running || !this.ready || !this.open) return;
    this.ready = false;
    this.running = true;
    const revision = this.revision;
    try {
      const hits = await this.search(
        [...this.blocks],
        this.query,
        this.matchCase,
      );
      if (!this.disposed && this.open && revision === this.revision) {
        this.hits = hits;
        this.active = 0;
        this.pending = false;
        this.error = null;
        this.jump++;
        this.changed();
      }
    } catch {
      if (!this.disposed && this.open && revision === this.revision) {
        this.pending = false;
        this.error = "Could not search this message. Retry Find.";
        this.changed();
      }
    } finally {
      this.running = false;
      if (!this.disposed && this.ready) void this.pump();
    }
  }
  dispose() {
    this.disposed = true;
    clearTimeout(this.timer);
  }
}
export class SearchWorker {
  private worker?: Worker;
  private loading?: Promise<void>;
  private cancelReady?: () => void;
  private disposed = false;
  private resolve?: (hits: SearchHit[]) => void;
  private reject?: (error: Error) => void;
  constructor() {
    void this.ready().catch(() => {});
  }
  private ready(): Promise<void> {
    if (this.disposed) return Promise.reject(new Error("Find closed."));
    if (this.loading) return this.loading;
    const worker = new Worker(new URL("./search_worker.ts", import.meta.url), {
      type: "module",
    });
    this.worker = worker;
    this.loading = new Promise((resolveReady, rejectReady) => {
      this.cancelReady = () => rejectReady(new Error("Find closed."));
      const failed = () => {
        if (this.worker !== worker) return;
        const error = new Error("Find stopped. Retry the search.");
        rejectReady(error);
        this.reject?.(error);
        this.resolve = this.reject = undefined;
        worker.terminate();
        this.worker = undefined;
        this.loading = undefined;
      };
      worker.onmessage = ({ data }) => {
        if (this.worker !== worker) return;
        if (data.ready) {
          resolveReady();
          return;
        }
        if (data.startup) {
          failed();
          return;
        }
        const resolve = this.resolve,
          reject = this.reject;
        this.resolve = this.reject = undefined;
        if (data.error) reject?.(new Error(data.error));
        else resolve?.(data.hits);
      };
      worker.onerror = failed;
    });
    return this.loading;
  }
  search: SearchText = async (blocks, query, matchCase) => {
    await this.ready();
    if (this.disposed) throw new Error("Find closed.");
    if (this.resolve) throw new Error("Find is busy. Retry shortly.");
    return new Promise((resolve, reject) => {
      this.resolve = resolve;
      this.reject = reject;
      this.worker!.postMessage({ blocks, query, matchCase });
    });
  };
  dispose() {
    this.disposed = true;
    this.cancelReady?.();
    this.worker?.terminate();
    this.reject?.(new Error("Find closed."));
    this.resolve = this.reject = undefined;
  }
}
