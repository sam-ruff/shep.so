// Module worker owning the WASM profile histories. It never touches
// IndexedDB, the network or credentials; the page persists accepted records.
import init, * as wasm from "./wasm/shep_profile_core";
import { HistoryHost, type HostRequest } from "./profile_history";

const ready = init({
  module_or_path: new URL("./wasm/shep_profile_core_bg.wasm", import.meta.url),
}).then(() => new HistoryHost(wasm));
self.onmessage = async (event: MessageEvent) => {
  const { id, request } = event.data as { id: number; request: HostRequest };
  try {
    const host = await ready;
    self.postMessage({ id, reply: host.handle(request) });
  } catch (error) {
    self.postMessage({
      id,
      reply: {
        status: "error",
        kind: "stopped",
        message:
          error instanceof Error
            ? error.message
            : "The profile worker could not start.",
      },
    });
  }
};
