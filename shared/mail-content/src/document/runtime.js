(() => {
  "use strict";
  const data = JSON.parse(document.getElementById("shep-data").textContent);
  const template = document.getElementById("shep-content");
  const bridge = window.ShepReader;
  const generation = data.generation;
  const blobs = new Map();
  let disposed = false;
  let booted = false,
    layout = 0,
    blocks = [],
    chunks = [],
    marks = [];
  let quotes = [],
    expanded = data.quotes,
    dark = data.dark,
    shortcuts = [];
  let highlighting = false,
    resizeTimer,
    selectionRevision = 0;
  function send(type, fields = {}) {
    if (disposed) return;
    const message = { type, generation, layout, ...fields };
    if (bridge?.postMessage) bridge.postMessage(JSON.stringify(message));
    else if (window.parent !== window) window.parent.postMessage(message, "*");
  }
  function clear() {
    if (window.CSS?.highlights) {
      CSS.highlights.delete("shep-matches");
      CSS.highlights.delete("shep-active");
    }
    for (const mark of marks) {
      const parent = mark.parentNode;
      mark.replaceWith(...mark.childNodes);
      parent?.normalize();
    }
    marks = [];
  }
  function collect(notify = true) {
    clear();
    blocks = [];
    chunks = [];
    let current,
      block = -1;
    const visibility = new WeakMap();
    function visible(element) {
      if (!element) return true;
      if (visibility.has(element)) return visibility.get(element);
      const chain = [];
      let current = element;
      while (current && !visibility.has(current)) {
        chain.push(current);
        current = current.parentElement;
      }
      let value = current ? visibility.get(current) : true;
      for (let i = chain.length - 1; i >= 0; i--) {
        const style = getComputedStyle(chain[i]);
        value =
          value &&
          style.display !== "none" &&
          style.visibility !== "hidden" &&
          style.visibility !== "collapse" &&
          style.opacity !== "0";
        visibility.set(chain[i], value);
      }
      return visibility.get(element);
    }
    function owner(element) {
      while (element && element !== document.body) {
        const display = getComputedStyle(element).display;
        if (
          [
            "block",
            "table-cell",
            "list-item",
            "flex",
            "grid",
            "table-caption",
          ].includes(display)
        )
          return element;
        element = element.parentElement;
      }
      return document.body;
    }
    const walker = document.createTreeWalker(
      document.body,
      NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT,
      {
        acceptNode(node) {
          const element =
            node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement;
          if (
            element?.closest("style,script,template,noscript") ||
            !visible(element)
          )
            return NodeFilter.FILTER_REJECT;
          return node.nodeType === Node.TEXT_NODE || node.nodeName === "BR"
            ? NodeFilter.FILTER_ACCEPT
            : NodeFilter.FILTER_SKIP;
        },
      },
    );
    while (walker.nextNode()) {
      const node = walker.currentNode;
      const text = node.nodeName === "BR" ? "\n" : node.data;
      if (!text) continue;
      const container = owner(node.parentElement);
      if (container !== current) {
        current = container;
        blocks.push("");
        chunks.push([]);
        block++;
      }
      const start = blocks[block].length;
      blocks[block] += text;
      if (node.nodeType === Node.TEXT_NODE)
        chunks[block].push({ node, start, end: start + text.length });
    }
    if (notify) {
      layout++;
      selectionRevision = 0;
      send("content", {
        blocks,
        hasQuotes: quotes.length > 0,
        quotes: expanded,
      });
    }
  }
  function theme() {
    document.documentElement.style.setProperty(
      "--shep-text",
      dark ? "#f4f4f5" : "#18181b",
    );
    document.documentElement.style.setProperty(
      "--shep-background",
      dark ? "#19191d" : "#ffffff",
    );
    document.documentElement.style.colorScheme = dark ? "dark" : "light";
  }
  function setQuotes() {
    for (const { node, placeholder } of quotes) {
      if (expanded && !node.isConnected) placeholder.replaceWith(node);
      else if (!expanded && node.isConnected) node.replaceWith(placeholder);
    }
  }
  function ranges(hit) {
    const result = [];
    if (
      !Number.isSafeInteger(hit.block) ||
      !Number.isSafeInteger(hit.start) ||
      !Number.isSafeInteger(hit.end) ||
      hit.start < 0 ||
      hit.end <= hit.start
    )
      return result;
    for (const chunk of chunks[hit.block] ?? []) {
      const start = Math.max(hit.start, chunk.start),
        end = Math.min(hit.end, chunk.end);
      if (end > start && chunk.node.isConnected) {
        const range = document.createRange();
        range.setStart(chunk.node, start - chunk.start);
        range.setEnd(chunk.node, end - chunk.start);
        result.push(range);
      }
    }
    return result;
  }
  function highlight(command) {
    if (
      command.layout !== layout ||
      !Number.isSafeInteger(command.revision) ||
      command.revision < selectionRevision
    )
      return;
    selectionRevision = command.revision;
    highlighting = true;
    // A fallback mark changes text nodes, so rebuild the same logical index
    // after unwrapping it. Its contents and layout revision remain unchanged.
    if (marks.length) {
      collect(false);
    } else clear();
    let active = [];
    if (window.CSS?.highlights && typeof Highlight !== "undefined") {
      const all = new Highlight(),
        selected = new Highlight();
      for (let i = 0; i < command.hits.length; i++) {
        const found = ranges(command.hits[i]);
        for (const range of found) {
          all.add(range);
          if (i === command.active) {
            selected.add(range);
            active.push(range);
          }
        }
      }
      CSS.highlights.set("shep-matches", all);
      CSS.highlights.set("shep-active", selected);
    } else {
      const entries = new Map();
      for (let i = 0; i < command.hits.length; i++)
        for (const range of ranges(command.hits[i])) {
          const list = entries.get(range.startContainer) ?? [];
          list.push({
            start: range.startOffset,
            end: range.endOffset,
            active: i === command.active,
          });
          entries.set(range.startContainer, list);
        }
      for (const [node, list] of entries) {
        list.sort((a, b) => a.start - b.start);
        const fragment = document.createDocumentFragment();
        let offset = 0;
        for (const found of list) {
          if (found.start < offset) continue;
          fragment.append(
            document.createTextNode(node.data.slice(offset, found.start)),
          );
          const mark = document.createElement("shep-match");
          mark.textContent = node.data.slice(found.start, found.end);
          if (found.active) {
            mark.dataset.active = "";
            active.push(mark);
          }
          fragment.append(mark);
          marks.push(mark);
          offset = found.end;
        }
        fragment.append(document.createTextNode(node.data.slice(offset)));
        node.replaceWith(fragment);
      }
    }
    if (command.jump && active.length) {
      const rect = active[0].getBoundingClientRect();
      window.scrollTo({
        top: Math.max(0, scrollY + rect.top - 64),
        left: Math.max(0, scrollX + rect.left - 24),
        behavior: "auto",
      });
    }
    highlighting = false;
  }
  function command(value) {
    if (!value || value.generation !== generation) return;
    if (value.type === "configure") {
      const changed = expanded !== !!value.quotes || dark !== !!value.dark;
      expanded = !!value.quotes;
      dark = !!value.dark;
      shortcuts = Array.isArray(value.shortcuts)
        ? value.shortcuts.filter((v) => typeof v === "string")
        : [];
      if (booted) {
        theme();
        setQuotes();
        if (changed) collect();
      }
    } else if (
      booted &&
      value.type === "highlight" &&
      Array.isArray(value.hits)
    )
      highlight(value);
    else if (booted && value.type === "index") collect();
  }
  Object.defineProperty(window, "shepReaderCommand", { value: command });
  function dispose() {
    disposed = true;
    booted = false;
    clearTimeout(resizeTimer);
    clear();
    for (const url of blobs.values()) URL.revokeObjectURL(url);
    blobs.clear();
    data.images = {};
    document.body.replaceChildren();
  }
  Object.defineProperty(window, "shepReaderDispose", { value: dispose });
  window.addEventListener("message", (event) => {
    if (event.source === window.parent) command(event.data);
  });
  function link(event, menu = false) {
    const node =
      event.target instanceof Element
        ? event.target.closest("[data-shep-link]")
        : null;
    if (!node) return;
    event.preventDefault();
    event.stopPropagation();
    if (!event.isTrusted) return;
    const url = data.links[node.dataset.shepLink];
    if (typeof url !== "string") return;
    if (url.startsWith("#")) {
      let id = url.slice(1);
      try {
        id = decodeURIComponent(id);
      } catch {}
      document.getElementById(id)?.scrollIntoView();
    } else send("link", { url, menu });
  }
  document.addEventListener("click", (event) => link(event), true);
  document.addEventListener("contextmenu", (event) => link(event, true), true);
  document.addEventListener(
    "keydown",
    (event) => {
      if (!event.isTrusted) return;
      if (
        event.key === "Enter" &&
        event.target instanceof Element &&
        event.target.closest("[data-shep-link]")
      ) {
        link(event);
        return;
      }
      const key = [
        ...(event.ctrlKey ? ["Control"] : []),
        ...(event.metaKey ? ["Meta"] : []),
        ...(event.altKey ? ["Alt"] : []),
        ...(event.shiftKey ? ["Shift"] : []),
        event.key.length === 1 ? event.key.toLowerCase() : event.key,
      ].join("+");
      if (shortcuts.includes(key)) {
        event.preventDefault();
        send("shortcut", { key });
      }
    },
    true,
  );
  document.addEventListener(
    "toggle",
    () => {
      if (booted && !highlighting) collect();
    },
    true,
  );
  window.addEventListener("resize", () => {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(() => {
      if (booted) collect();
    }, 100);
  });
  window.addEventListener("pagehide", (event) => {
    if (!event.persisted) dispose();
  });
  (async () => {
    const decoded = [];
    for (const [key, image] of Object.entries(data.images)) {
      const bytes = Uint8Array.from(atob(image.bytes), (value) =>
        value.charCodeAt(0),
      );
      const url = URL.createObjectURL(
        new Blob([bytes], { type: "image/webp" }),
      );
      blobs.set(key, url);
      const probe = new Image();
      probe.src = url;
      decoded.push(probe.decode().catch(() => {}));
    }
    const replace = (value) =>
      value.replace(
        /urn:shep-image:([0-9a-f]{64})/g,
        (_, key) => blobs.get(key) ?? "",
      );
    for (const node of template.content.querySelectorAll("*")) {
      for (const attr of ["src", "srcset", "background", "style"])
        if (node.hasAttribute(attr))
          node.setAttribute(attr, replace(node.getAttribute(attr)));
      if (node.localName === "style")
        node.textContent = replace(node.textContent);
      // Sender CSS must not disable the reader's ordinary selection/Copy.
      node.style.setProperty("user-select", "text", "important");
      node.style.setProperty("-webkit-user-select", "text", "important");
    }
    await Promise.all(decoded);
    if (disposed) return;
    data.images = {};
    document.getElementById("shep-data")?.remove();
    const first = template.content.querySelector("[data-shep-body]");
    for (const attr of [...first.attributes])
      if (!attr.name.startsWith("data-"))
        document.body.setAttribute(attr.name, attr.value);
    while (first.firstChild) first.before(first.firstChild);
    first.remove();
    document.body.append(template.content);
    template.remove();
    quotes = [
      ...document.body.querySelectorAll(
        "blockquote,.gmail_quote,.yahoo_quoted",
      ),
    ]
      .filter(
        (node) =>
          !node.parentElement?.closest("blockquote,.gmail_quote,.yahoo_quoted"),
      )
      .map((node) => ({
        node,
        placeholder: document.createComment("quoted history"),
      }));
    theme();
    document.documentElement.style.setProperty(
      "scroll-behavior",
      "auto",
      "important",
    );
    setQuotes();
    booted = true;
    collect();
    send("ready");
  })().catch(() =>
    send("error", {
      message: "Could not display formatted mail. Use plain text or retry.",
    }),
  );
})();
