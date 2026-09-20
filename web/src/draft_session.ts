import type { Draft, DraftAttachment } from "./model";

export class DraftSession {
  readonly draft: Draft;
  private saved: number;
  private failure?: { revision: number; message: string };
  private files?: {
    run: () => Promise<DraftAttachment[]>;
    error?: string;
    retry: boolean;
  };
  private timer?: ReturnType<typeof setTimeout>;
  private writing?: Promise<boolean>;
  private retryText = false;
  private retired = false;
  private listeners = new Set<() => void>();

  constructor(
    draft: Draft,
    persisted: boolean,
    private readonly save: (draft: Draft) => Promise<void>,
    private readonly changed: () => void = () => {},
  ) {
    this.draft = structuredClone(draft);
    this.saved = persisted ? (draft.revision ?? 0) : -1;
  }

  get error() {
    return this.failure?.message ?? this.files?.error;
  }
  get pending() {
    return this.saved < (this.draft.revision ?? 0) || !!this.files;
  }
  get saving() {
    return !!this.writing;
  }
  get filesPending() {
    return !!this.files;
  }
  get fileError() {
    return this.files?.error;
  }
  get status() {
    if (this.error) return this.saving ? "Not saved. Retrying…" : "Not saved";
    return this.pending || this.saving ? "Saving…" : "Saved on this browser";
  }

  subscribe(listener: () => void) {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
  private notify() {
    if (this.retired) return;
    this.changed();
    for (const listener of this.listeners) listener();
  }
  edited() {
    if (this.retired) return;
    this.draft.revision = (this.draft.revision ?? 0) + 1;
    clearTimeout(this.timer);
    this.timer = setTimeout(() => void this.flush(), 500);
    this.notify();
  }

  changeFiles(run: () => Promise<DraftAttachment[]>) {
    if (this.retired) return Promise.resolve(false);
    if (this.files) return this.flush();
    this.files = { run, retry: true };
    return this.flush();
  }

  useSavedFiles(read: () => Promise<DraftAttachment[]>) {
    if (!this.files?.error || this.writing || this.retired)
      return Promise.resolve(false);
    this.files.run = read;
    return this.flush(true);
  }

  flush(retry = false): Promise<boolean> {
    if (this.retired) return Promise.resolve(false);
    clearTimeout(this.timer);
    this.retryText ||= retry;
    if (retry && this.files) this.files.retry = true;
    if (this.writing) return this.writing;
    this.writing = this.write().finally(() => {
      clearTimeout(this.timer);
      this.writing = undefined;
      this.notify();
    });
    this.notify();
    return this.writing;
  }

  private async write() {
    while (this.pending && !this.retired) {
      const revision = this.draft.revision ?? 0;
      if (this.saved < revision) {
        if (this.failure?.revision === revision && !this.retryText)
          return false;
        this.retryText = false;
        const snapshot = structuredClone(this.draft);
        try {
          await this.save(snapshot);
          this.saved = Math.max(this.saved, revision);
          if (this.failure && this.failure.revision <= revision)
            this.failure = undefined;
        } catch (error) {
          this.failure = {
            revision,
            message:
              error instanceof Error
                ? error.message
                : "Could not save this draft.",
          };
          this.notify();
          if ((this.draft.revision ?? 0) === revision) return false;
        }
        continue;
      }
      const files = this.files;
      if (!files) break;
      if (files.error && !files.retry) return false;
      files.retry = false;
      try {
        this.draft.attachments = await files.run();
        this.files = undefined;
      } catch (error) {
        files.error =
          error instanceof Error
            ? error.message
            : "Could not save attachments.";
        if (this.saved < (this.draft.revision ?? 0)) continue;
        return false;
      }
    }
    return !this.retired && !this.error;
  }

  retire() {
    this.retired = true;
    clearTimeout(this.timer);
    this.listeners.clear();
    this.files = undefined;
  }
}
