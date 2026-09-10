import type { ReceivedAttachment } from "./attachment_content";
export type { ReceivedAttachment } from "./attachment_content";
export class AttachmentReader {
  private worker?: Worker;
  private serial = 0;
  private pending = new Map<
    number,
    { resolve: (value: any) => void; reject: (error: Error) => void }
  >();
  constructor(private user: string) {}
  private call(message: string, file?: string): Promise<any> {
    if (this.pending.size >= 8)
      return Promise.reject(
        new Error(
          "Attachment reader is busy. Wait for a save to finish and retry.",
        ),
      );
    if (!this.worker) {
      this.worker = new Worker(
        new URL("./attachment_worker.ts", import.meta.url),
        { type: "module" },
      );
      this.worker.onmessage = ({ data }) => {
        const pending = this.pending.get(data.job);
        if (!pending) return;
        this.pending.delete(data.job);
        if (data.error) pending.reject(new Error(data.error));
        else pending.resolve(data);
      };
      this.worker.onerror = () => {
        for (const pending of this.pending.values())
          pending.reject(
            new Error("The attachment reader stopped. Retry Save."),
          );
        this.pending.clear();
        this.worker?.terminate();
        this.worker = undefined;
      };
    }
    return new Promise((resolve, reject) => {
      const job = ++this.serial;
      this.pending.set(job, { resolve, reject });
      this.worker!.postMessage({ job, user: this.user, message, file });
    });
  }
  async files(message: string): Promise<ReceivedAttachment[]> {
    return (await this.call(message)).files;
  }
  async read(message: string, file: ReceivedAttachment): Promise<ArrayBuffer> {
    const result = await this.call(message, file.id);
    if (result.file.id !== file.id || result.bytes.byteLength !== file.size)
      throw new Error("This attachment changed. Reopen the message and retry.");
    return result.bytes;
  }
}
