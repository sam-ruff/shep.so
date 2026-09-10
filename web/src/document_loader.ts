import type { DocumentOptions, PreparedMessage } from "./formatted_content";
export class DocumentLoader {
  private worker?: Worker;
  private reject?: (error: Error) => void;
  constructor(private user: string) {}
  load(message: string, options: DocumentOptions): Promise<PreparedMessage> {
    this.cancel();
    return new Promise((resolve, reject) => {
      const worker = new Worker(
        new URL("./document_worker.ts", import.meta.url),
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
          new Error("The formatted reader stopped. Use plain text or retry."),
        );
      };
      worker.postMessage({ user: this.user, message, options });
    });
  }
  cancel() {
    this.worker?.terminate();
    this.worker = undefined;
    this.reject?.(new Error("Message reader closed."));
    this.reject = undefined;
  }
}
