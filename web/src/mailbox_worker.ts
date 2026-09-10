import { MailboxStore } from "./mailbox_store";
let store: MailboxStore | undefined, ready: Promise<void> | undefined;
let closed = false,
  pending = 0,
  sequence = Promise.resolve();
self.onmessage = ({ data }) => {
  const id = data.id;
  if (!Number.isSafeInteger(id) || id < 0) return;
  if (data.initialize !== undefined) {
    if (ready) {
      self.postMessage({ id, error: "Mailbox worker is already connected." });
      return;
    }
    ready = MailboxStore.open(data.initialize).then((value) => {
      store = value;
    });
    void ready.then(
      () => self.postMessage({ id, result: null }),
      () => {
        closed = true;
        self.postMessage({
          id,
          error:
            "Could not open the cached mailbox. Allow browser storage, close other Shep tabs if an update is waiting, then retry.",
        });
      },
    );
    return;
  }
  if (!ready || closed) {
    self.postMessage({ id, error: "Mailbox storage is closed. Reopen Shep." });
    return;
  }
  if (pending >= 32 && !data.close) {
    self.postMessage({
      id,
      error: "The mailbox is catching up. Retry shortly.",
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
        : data.query
          ? await store!.page(data.query, () =>
              self.postMessage({ id, phase: "snapshot" }),
            )
          : (() => {
              throw Error("Invalid mailbox request. Retry Refresh.");
            })();
      self.postMessage({ id, result });
    } catch (error) {
      self.postMessage({
        id,
        error:
          error instanceof Error && error.name === "Error"
            ? error.message
            : "Could not read the cached mailbox. Allow browser storage and retry Refresh.",
      });
    } finally {
      pending--;
      if (data.close) store?.close();
    }
  });
};
