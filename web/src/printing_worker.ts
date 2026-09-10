import { BrowserStore } from "./storage";
import { resolveMail, localId } from "./sent_cache";
import { preparePrint } from "./printing_content";
self.onmessage = async ({ data }) => {
  let store: BrowserStore | undefined;
  try {
    store = await BrowserStore.open(data.user);
    const mail = await resolveMail(store, data.message);
    const raw = mail && (await store.get<string>("raw", localId(mail)));
    store.close();
    store = undefined;
    if (!raw)
      throw new Error(
        "This message is no longer cached. Refresh it before printing.",
      );
    self.postMessage({ prepared: await preparePrint(raw, data.options) });
  } catch (error) {
    self.postMessage({
      error:
        error instanceof Error
          ? error.message
          : "Could not prepare this print. Try plain text or retry.",
    });
  } finally {
    store?.close();
  }
};
