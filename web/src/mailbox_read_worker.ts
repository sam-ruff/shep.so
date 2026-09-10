import { MailboxReads } from "./mailbox_cache";
let store: MailboxReads | undefined, ready: Promise<void> | undefined;
let closed = false,
  pending = 0,
  sequence = Promise.resolve();
self.onmessage = ({ data }) => {
  const id = data.id;
  if (!Number.isSafeInteger(id) || id < 0) return;
  if (data.initialize !== undefined) {
    if (ready) {
      self.postMessage({ id, error: "Mailbox reader is already connected." });
      return;
    }
    ready = MailboxReads.open(data.initialize).then((value) => {
      store = value;
    });
    void ready.then(
      () => self.postMessage({ id, result: null }),
      () => {
        closed = true;
        self.postMessage({
          id,
          error:
            "Could not open cached mail. Allow browser storage and reopen Shep.",
        });
      },
    );
    return;
  }
  if (!ready || closed) {
    self.postMessage({ id, error: "Cached mail is closed. Reopen Shep." });
    return;
  }
  if (pending >= 32 && !data.close) {
    self.postMessage({
      id,
      error: "Cached mail is catching up. Retry shortly.",
    });
    return;
  }
  if (data.close) closed = true;
  pending++;
  sequence = sequence.then(async () => {
    try {
      await ready;
      const result = data.close
        ? null
        : data.scan
          ? await store!.scan(data.scan)
          : data.metadata !== undefined
            ? await store!.metadata(data.metadata)
            : await store!.detail(data.detail);
      self.postMessage({ id, result });
    } catch (error) {
      self.postMessage({
        id,
        error:
          error instanceof Error && error.name === "Error"
            ? error.message
            : "Could not read cached mail. Retry Refresh.",
      });
    } finally {
      pending--;
      if (data.close) store?.close();
    }
  });
};
