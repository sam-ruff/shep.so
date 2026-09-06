import { BrowserStore } from "./storage";
import { resolveMail, localId } from "./sent_cache";
import {
  initializeAttachments,
  catalogAttachments,
  readAttachment,
} from "./attachment_content";
// One worker processes at most one bounded MIME payload at a time. Main-thread
// admission is bounded separately; only file metadata or one chosen buffer leaves.
let queue = Promise.resolve();
self.onmessage = (event: MessageEvent) => {
  const { job, user, message, file } = event.data;
  queue = queue.then(async () => {
    let store: BrowserStore | undefined;
    try {
      store = await BrowserStore.open(user);
      const mail = await resolveMail(store, message);
      const raw = mail && (await store.get<string>("raw", localId(mail)));
      if (!raw)
        throw new Error(
          "This message is no longer cached. Reopen it and retry.",
        );
      await initializeAttachments();
      const files = catalogAttachments(raw);
      if (file === undefined) self.postMessage({ job, files });
      else {
        const found = files.find((f) => f.id === file);
        if (!found)
          throw new Error(
            "This attachment changed. Reopen the message and retry.",
          );
        const bytes = readAttachment(raw, file);
        self.postMessage(
          { job, file: found, bytes: bytes.buffer },
          { transfer: [bytes.buffer] },
        );
      }
    } catch {
      self.postMessage({
        job,
        error:
          "Could not read this cached attachment. Reopen the message or refresh it, then retry.",
      });
    } finally {
      store?.close();
    }
  });
};
