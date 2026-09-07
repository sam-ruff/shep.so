import { initializeAttachments } from "./attachment_content";
import { find_text } from "./wasm/shep_mail_content";
const ready = initializeAttachments();
void ready
  .then(() => self.postMessage({ ready: true }))
  .catch(() => self.postMessage({ startup: true }));
self.onmessage = async ({ data }: MessageEvent) => {
  try {
    await ready;
    self.postMessage({
      hits: JSON.parse(
        find_text(JSON.stringify(data.blocks), data.query, data.matchCase),
      ),
    });
  } catch {
    self.postMessage({ error: "Could not search this message. Retry Find." });
  }
};
