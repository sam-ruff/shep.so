/** Delivers only the latest value, one round at a time. A round's consumers
 * read the same browser stores the producer writes, so the next round waits at
 * least as long as the previous one took (and never less than `minimumGap`).
 * This keeps those reads to at most half of the wall time however slow the
 * host is. The first value after an idle gap is delivered immediately. */
export class ProgressPacer<T> {
  private latest?: { value: T };
  private round = false;
  private timer?: ReturnType<typeof setTimeout>;
  private closed = false;
  constructor(
    private deliver: (value: T) => Promise<unknown> | void,
    private minimumGap = 100,
    private now: () => number = () => performance.now(),
  ) {}
  push(value: T) {
    if (this.closed) return;
    this.latest = { value };
    if (!this.round && this.timer === undefined) this.next();
  }
  close() {
    this.closed = true;
    this.latest = undefined;
    clearTimeout(this.timer);
    this.timer = undefined;
  }
  private next() {
    const pending = this.latest;
    this.latest = undefined;
    if (!pending || this.closed) return;
    const started = this.now();
    this.round = true;
    let delivered: Promise<unknown>;
    try {
      delivered = Promise.resolve(this.deliver(pending.value));
    } catch (error) {
      delivered = Promise.reject(error);
    }
    // A failed consumer read still ends its round; it reports its own error.
    void delivered
      .catch(() => {})
      .then(() => {
        this.round = false;
        if (this.closed) return;
        const gap = Math.max(this.minimumGap, this.now() - started);
        this.timer = setTimeout(() => {
          this.timer = undefined;
          this.next();
        }, gap);
      });
  }
}
