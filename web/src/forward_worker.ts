import { BrowserStore } from "./storage";
import { resolveMail, localId } from "./sent_cache";
import { prepareForwardContent } from "./forward_content";

self.onmessage = async ({ data }) => {
  let store: BrowserStore | undefined;
  try {
    store = await BrowserStore.open(data.user);
    const mail = await resolveMail(store, data.message);
    const raw = mail && (await store.get<string>("raw", localId(mail)));
    store.close();
    store = undefined;
    if (!raw || !mail)
      throw new Error(
        "The original message is no longer cached. Refresh it before forwarding.",
      );
    const prepared = {
      ...(await prepareForwardContent(raw)),
      accountId: mail.core.account_id,
    };
    self.postMessage(
      { prepared },
      { transfer: prepared.files.map((f) => f.bytes.buffer) },
    );
  } catch (error) {
    self.postMessage({
      error:
        error instanceof Error
          ? error.message
          : "Could not prepare this forward. Reopen the message and retry.",
    });
  } finally {
    store?.close();
  }
};
