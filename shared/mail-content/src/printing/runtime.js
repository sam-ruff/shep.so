(() => {
  "use strict";
  const data = JSON.parse(document.getElementById("shep-print-data").textContent);
  const generation = data.generation;
  const template = document.getElementById("shep-print-content");
  const blobs = new Map();
  let ready = false, disposed = false;
  function send(type) {
    const value = { type, generation };
    if (window.ShepPrint?.postMessage) window.ShepPrint.postMessage(JSON.stringify(value));
    else if (window.webkit?.messageHandlers?.ShepPrint) window.webkit.messageHandlers.ShepPrint.postMessage(value);
    else if (window.parent !== window) window.parent.postMessage(value, "*");
  }
  function dispose() {
    disposed = true;
    ready = false;
    for (const url of blobs.values()) URL.revokeObjectURL(url);
    blobs.clear();
    data.images = {};
    document.body.replaceChildren();
  }
  window.addEventListener("message", event => {
    if (event.source !== window.parent || event.data?.generation !== generation) return;
    if (event.data.type === "dispose") dispose();
    else if (event.data.type === "print" && ready && !disposed) {
      try { window.print(); } catch { send("error"); }
    }
  });
  window.addEventListener("afterprint", () => { if (!disposed) send("dialog-closed"); });
  window.addEventListener("pagehide", event => { if (!event.persisted) dispose(); });
  for (const event of ["click", "auxclick", "submit"]) document.addEventListener(event, e => e.preventDefault(), true);
  (async () => {
    const decoded = [];
    for (const [key, image] of Object.entries(data.images)) {
      const bytes = Uint8Array.from(atob(image.bytes), value => value.charCodeAt(0));
      const url = URL.createObjectURL(new Blob([bytes], {type: "image/webp"}));
      blobs.set(key, url);
      const probe = new Image();
      probe.src = url;
      decoded.push(probe.decode());
    }
    const replace = value => value.replace(/urn:shep-image:([0-9a-f]{64})/g, (_, key) => blobs.get(key) ?? "");
    for (const node of template.content.querySelectorAll("*")) {
      for (const attr of ["src", "srcset", "background", "style"]) if (node.hasAttribute(attr)) node.setAttribute(attr, replace(node.getAttribute(attr)));
      if (node.localName === "style") node.textContent = replace(node.textContent);
      if (node.localName === "img" && !node.hasAttribute("src") && !node.getAttribute("srcset")) {
        const alt = document.createElement("span");
        alt.textContent = node.getAttribute("alt") || "";
        node.replaceWith(alt);
      }
    }
    await Promise.all(decoded);
    if (disposed) return;
    data.images = {};
    document.getElementById("shep-print-data").remove();
    const first = template.content.querySelector("[data-shep-body]");
    for (const attr of [...first.attributes]) if (!attr.name.startsWith("data-")) document.body.setAttribute(attr.name, attr.value);
    while (first.firstChild) first.before(first.firstChild);
    first.remove();
    document.body.append(template.content);
    template.remove();
    // Keep the source envelope readable even when the email styles headings.
    const host = document.createElement("div");
    host.setAttribute("style", "display:block!important;position:static!important;visibility:visible!important;opacity:1!important;height:auto!important;margin:0 0 24px!important;float:none!important;transform:none!important");
    const root = host.attachShadow({mode: "closed"});
    const style = document.createElement("style");
    style.textContent = ":host{all:initial}section{display:block;font:13px/1.6 Arial,sans-serif;color:#25232b;background:white;padding:12px 0 18px;border-bottom:1px solid #ddd;overflow-wrap:anywhere}h1{font-size:22px;line-height:1.3;margin:0 0 12px}div{margin:2px 0}b{display:inline-block;min-width:54px}";
    root.append(style);
    const header = document.createElement("section");
    for (const [name, value] of data.headers) {
      const row = document.createElement(name === "Subject" ? "h1" : "div");
      if (name !== "Subject") { const label = document.createElement("b"); label.textContent = name + ": "; row.append(label); }
      row.append(document.createTextNode(value)); header.append(row);
    }
    if (data.files.length) { const row = document.createElement("div"); row.textContent = "Attachments: " + data.files.join(", "); header.append(row); }
    root.append(header); document.body.prepend(host);
    await Promise.all([...document.images].map(image => image.decode()));
    await document.fonts.ready;
    if (disposed) return;
    ready = true;
    send("ready");
  })().catch(() => { if (!disposed) send("error"); });
})();
