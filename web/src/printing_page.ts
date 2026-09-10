import "./style.css";
import "./printing.css";
import type { PreparedPrint } from "./printing_content";
const id = location.hash.slice(1),
  owner = window.opener;
window.opener = null;
const status = document.querySelector<HTMLParagraphElement>("#print-status")!;
const print = document.querySelector<HTMLButtonElement>("#print-action")!;
const retry = document.querySelector<HTMLButtonElement>("#print-retry")!;
const container = document.querySelector<HTMLDivElement>("#print-document")!;
let frame: HTMLIFrameElement | undefined,
  generation = "",
  timer: ReturnType<typeof setTimeout> | undefined,
  prepared: PreparedPrint | undefined;
function error(message: string) {
  clearTimeout(timer);
  status.textContent = message;
  print.disabled = true;
  retry.hidden = !owner || owner.closed;
}
function display(value: PreparedPrint, token: string) {
  document.title = value.title;
  prepared = value;
  generation = token;
  print.disabled = true;
  retry.hidden = true;
  clearTimeout(timer);
  frame?.remove();
  frame = document.createElement("iframe");
  frame.title = "Message to print";
  frame.sandbox.add("allow-scripts", "allow-modals");
  frame.referrerPolicy = "no-referrer";
  frame.srcdoc = value.document;
  container.replaceChildren(frame);
  status.textContent = "Preparing the print document…";
  timer = setTimeout(
    () =>
      error(
        "The print renderer did not finish. Retry preparation or return to Shep and choose plain text.",
      ),
    15000,
  );
}
window.addEventListener("message", (event) => {
  const data = event.data;
  if (
    event.source === frame?.contentWindow &&
    data?.generation === generation
  ) {
    if (data.type === "ready" && print.disabled) {
      clearTimeout(timer);
      print.disabled = false;
      status.textContent = prepared?.issues.length
        ? prepared.issues.join(" ")
        : "Choose a printer or save as PDF. The full message and quoted history are included.";
      print.click();
    } else if (data.type === "error")
      error(
        "Could not display the print document. Retry preparation or choose plain text in Shep.",
      );
    else if (data.type === "dialog-closed")
      status.textContent =
        "Print dialog closed. You can print again or close this preview.";
    return;
  }
  if (
    event.source !== owner ||
    event.origin !== location.origin ||
    data?.id !== id
  )
    return;
  if (data.type === "shep-print-document") {
    document.documentElement.dataset.theme = data.appearance;
    display(data.prepared, data.generation);
  } else if (data.type === "shep-print-loading") {
    document.documentElement.dataset.theme = data.appearance;
    status.textContent = "Preparing your message…";
    retry.hidden = true;
    print.disabled = true;
  } else if (data.type === "shep-print-error") error(data.message);
});
print.onclick = () => {
  if (!print.disabled)
    frame?.contentWindow?.postMessage({ type: "print", generation }, "*");
};
window.addEventListener("keydown", (event) => {
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "p") {
    event.preventDefault();
    print.click();
  }
});
retry.onclick = () => {
  if (owner && !owner.closed)
    owner.postMessage({ type: "shep-print-retry", id }, location.origin);
  else error("Return to Shep and choose Print again.");
};
document.querySelector<HTMLButtonElement>("#print-close")!.onclick = () =>
  window.close();
window.addEventListener("pagehide", () => {
  clearTimeout(timer);
  frame?.contentWindow?.postMessage({ type: "dispose", generation }, "*");
});
if (owner && /^[0-9a-f-]{36}$/.test(id))
  owner.postMessage({ type: "shep-print-ready", id }, location.origin);
else error("This preview has no message. Return to Shep and choose Print.");
