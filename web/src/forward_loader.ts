import type { ForwardPrepared } from "./forward_content";
export class ForwardLoader {
  private workers = new Set<Worker>();
  constructor(private user: string) {}
  load(message: string): Promise<ForwardPrepared> {
    if (this.workers.size >= 2)
      return Promise.reject(
        new Error("Forward preparation is busy. Retry shortly."),
      );
    return new Promise((resolve, reject) => {
      const worker = new Worker(
        new URL("./forward_worker.ts", import.meta.url),
        { type: "module" },
      );
      this.workers.add(worker);
      const finish = () => {
        worker.terminate();
        this.workers.delete(worker);
      };
      worker.onmessage = ({ data }) => {
        finish();
        if (data.error) reject(new Error(data.error));
        else resolve(data.prepared);
      };
      worker.onerror = () => {
        finish();
        reject(
          new Error(
            "Forward preparation stopped. Reopen the message and retry.",
          ),
        );
      };
      try {
        worker.postMessage({ user: this.user, message });
      } catch {
        finish();
        reject(new Error("Could not prepare this forward. Retry."));
      }
    });
  }
}
