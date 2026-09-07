import { SelectionStore } from "./selection_store";
import type { SelectionCommand } from "./selection_types";
let store: SelectionStore | undefined, ready: Promise<void> | undefined;
let closed = false,
  pending = 0,
  sequence = Promise.resolve();
self.onmessage = ({ data }) => {
  const id = data.id;
  if (!Number.isSafeInteger(id) || id < 0) return;
  if (data.initialize !== undefined) {
    if (ready) {
      self.postMessage({ id, error: "Selection worker is already connected." });
      return;
    }
    ready = SelectionStore.open(data.initialize).then((value) => {
      store = value;
    });
    void ready.then(
      () => self.postMessage({ id, result: null }),
      () => {
        closed = true;
        store?.close();
        self.postMessage({
          id,
          error:
            "Could not open selection storage. Close other Shep tabs if an update is waiting, then retry.",
        });
      },
    );
    return;
  }
  if (!ready || closed) {
    self.postMessage({
      id,
      error: "Selection storage is closed. Select the messages again.",
    });
    return;
  }
  if (pending >= 32 && !data.close) {
    self.postMessage({
      id,
      error: "Selection is catching up. Retry that action shortly.",
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
        : await store!.run(
            data.command as SelectionCommand,
            data.observed ?? [],
            () => self.postMessage({ id, phase: "snapshot" }),
          );
      self.postMessage({ id, result });
    } catch (error) {
      const message =
        error instanceof Error && error.name === "Error"
          ? error.message
          : "Could not update this selection. Retry or select the messages again.";
      self.postMessage({ id, error: message });
    } finally {
      pending--;
      if (data.close) store?.close();
    }
  });
};
