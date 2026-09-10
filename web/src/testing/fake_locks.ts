/** Minimal exclusive Web Lock manager for Node unit tests: request with
 * ifAvailable, queued waiters and query() of held names. */
type Callback = (lock: { name: string } | null) => unknown;
export class FakeLockManager {
  private held = new Map<string, (() => void)[]>();
  async request(
    name: string,
    options: { ifAvailable?: boolean } | Callback,
    callback?: Callback,
  ) {
    const work = (typeof options === "function" ? options : callback)!;
    const settings = typeof options === "function" ? {} : options;
    for (;;) {
      const waiters = this.held.get(name);
      if (!waiters) break;
      if (settings.ifAvailable) return work(null);
      await new Promise<void>((resolve) => waiters.push(resolve));
    }
    this.held.set(name, []);
    try {
      return await work({ name });
    } finally {
      const next = this.held.get(name) ?? [];
      this.held.delete(name);
      // Waiters recheck the map on their turn; wake them in arrival order.
      for (const resume of next) resume();
    }
  }
  async query() {
    return {
      held: [...this.held.keys()].map((name) => ({
        name,
        mode: "exclusive" as const,
      })),
      pending: [],
    };
  }
}
export function installFakeLocks() {
  const locks = new FakeLockManager();
  Object.defineProperty(globalThis, "navigator", {
    value: { locks },
    configurable: true,
    writable: true,
  });
  return locks;
}
