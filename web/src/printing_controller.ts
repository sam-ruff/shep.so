import type { PrintLoader } from "./printing_loader";
interface Preview {
  id: string;
  source: string;
  popup: Window;
  plain: boolean;
  appearance: string;
  loader: PrintLoader;
  busy: boolean;
  attempt: number;
  timeout?: ReturnType<typeof setTimeout>;
}
/** A separate preview captures its source at the user's click. It remains usable
 * when the mail reader navigates; closing it cancels only its owned worker. */
export class PrintController {
  private previews = new Map<string, Preview>();
  private timer?: ReturnType<typeof setInterval>;
  constructor(
    private loader: () => PrintLoader,
    private changed: () => void,
    private error: (message: string) => void,
  ) {
    window.addEventListener("message", this.receive);
  }
  preparing(source: string) {
    return [...this.previews.values()].some(
      (p) => p.source === source && p.busy,
    );
  }
  open(source: string, plain: boolean, appearance: string) {
    this.prune();
    if (this.previews.size >= 2) {
      this.error("Close a print preview before opening another.");
      return;
    }
    const id = crypto.randomUUID(),
      url = new URL("print.html", location.href);
    url.hash = id;
    const popup = window.open(
      url.href,
      "_blank",
      "popup,width=1000,height=800",
    );
    if (!popup) {
      this.error(
        "Allow Shep to open a print preview, then choose Print again.",
      );
      return;
    }
    const entry: Preview = {
      id,
      source,
      popup,
      plain,
      appearance,
      loader: this.loader(),
      busy: true,
      attempt: 0,
    };
    this.previews.set(id, entry);
    entry.timeout = setTimeout(() => {
      if (entry.attempt === 0 && !popup.closed) {
        entry.busy = false;
        this.error(
          "The print preview did not open. Check the connection or sign in again, then retry Print.",
        );
        this.changed();
      }
    }, 15000);
    this.timer ??= setInterval(() => this.prune(), 250);
    this.changed();
  }
  private receive = (event: MessageEvent) => {
    if (event.origin !== location.origin) return;
    const entry = this.previews.get(event.data?.id);
    if (!entry || event.source !== entry.popup) return;
    if (event.data.type === "shep-print-ready" && entry.attempt === 0) {
      clearTimeout(entry.timeout);
      void this.prepare(entry);
    } else if (event.data.type === "shep-print-retry" && !entry.busy)
      void this.prepare(entry);
  };
  private async prepare(entry: Preview) {
    entry.busy = true;
    const generation = `${entry.id}.${++entry.attempt}`;
    entry.popup.postMessage(
      {
        type: "shep-print-loading",
        id: entry.id,
        appearance: entry.appearance,
      },
      location.origin,
    );
    this.changed();
    try {
      const prepared = await entry.loader.load(entry.source, {
        generation,
        plain: entry.plain,
      });
      if (this.previews.get(entry.id) === entry && !entry.popup.closed)
        entry.popup.postMessage(
          {
            type: "shep-print-document",
            id: entry.id,
            generation,
            prepared,
            appearance: entry.appearance,
          },
          location.origin,
        );
    } catch (error) {
      if (this.previews.get(entry.id) === entry && !entry.popup.closed)
        entry.popup.postMessage(
          {
            type: "shep-print-error",
            id: entry.id,
            message:
              error instanceof Error
                ? error.message
                : "Could not prepare this print. Retry.",
          },
          location.origin,
        );
    } finally {
      entry.busy = false;
      this.changed();
    }
  }
  private prune() {
    for (const [id, p] of this.previews)
      if (p.popup.closed) {
        clearTimeout(p.timeout);
        p.loader.cancel();
        this.previews.delete(id);
        this.changed();
      }
    if (!this.previews.size) {
      clearInterval(this.timer);
      this.timer = undefined;
    }
  }
  dispose() {
    window.removeEventListener("message", this.receive);
    clearInterval(this.timer);
    for (const p of this.previews.values()) {
      clearTimeout(p.timeout);
      p.loader.cancel();
      p.popup.close();
    }
    this.previews.clear();
  }
}
