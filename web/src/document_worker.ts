import { BrowserStore } from "./storage";
import { resolveMail, localId } from "./sent_cache";
import { prepareDocument } from "./formatted_content";
// Owned by one displayed message. Navigation terminates this worker, including
// synchronous MIME/CSS/image work. Attachment saves have an independent worker.
self.onmessage = async ({ data }) => {
  let store: BrowserStore | undefined;
  try {
    store = await BrowserStore.open(data.user);
    const mail = await resolveMail(store, data.message);
    const raw = mail && (await store.get<string>("raw", localId(mail)));
    store.close();
    store = undefined;
    if (!raw) throw new Error("Missing cached MIME");
    self.postMessage({ prepared: await prepareDocument(raw, data.options) });
  } catch {
    self.postMessage({
      error:
        "Could not format this cached message. Use plain text or retry. If it is still unavailable, refresh the message.",
    });
  } finally {
    store?.close();
  }
};
