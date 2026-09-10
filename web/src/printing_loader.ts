import type { PrintOptions, PreparedPrint } from "./printing_content";
export class PrintLoader {
  private worker?: Worker;
  private reject?: (error: Error) => void;
  constructor(private user: string) {}
  load(message: string, options: PrintOptions): Promise<PreparedPrint> {
    this.cancel();
    return new Promise((resolve, reject) => {
      const worker = new Worker(
        new URL("./printing_worker.ts", import.meta.url),
        { type: "module" },
      );
      this.worker = worker;
      this.reject = reject;
      const finish = () => {
        worker.terminate();
        if (this.worker === worker) {
          this.worker = undefined;
          this.reject = undefined;
        }
      };
      worker.onmessage = ({ data }) => {
        if (this.worker !== worker) return;
        finish();
        if (data.error) reject(new Error(data.error));
        else resolve(data.prepared);
      };
      worker.onerror = () => {
        if (this.worker !== worker) return;
        finish();
        reject(
          new Error("Print preparation stopped. Retry or choose plain text."),
        );
      };
      try {
        worker.postMessage({ user: this.user, message, options });
      } catch {
        finish();
        reject(new Error("Could not prepare this print. Retry."));
      }
    });
  }
  cancel() {
    this.worker?.terminate();
    this.worker = undefined;
    this.reject?.(new Error("Print preview closed."));
    this.reject = undefined;
  }
}
