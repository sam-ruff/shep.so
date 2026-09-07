import type { MessageFind } from "./message_find";
// The app currently rebuilds its control tree on change. Keep the frame outside
// that tree so typing Find, sync and toolbar updates cannot reload the document
// or lose its selection/scroll position. A clipped portal follows the placeholder.
export class FormattedFrame {
  private frame = document.createElement("iframe");
  private placeholder?: HTMLElement;
  private observer = new ResizeObserver(() => this.place());
  private configuration = "";
  private layout = 0;
  private jump = -1;
  private highlighted = "";
  private booted = false;
  private startup?: ReturnType<typeof setTimeout>;
  constructor(
    private generation: string,
    documentSource: string,
    private events: {
      content: (blocks: string[], hasQuotes: boolean) => void;
      link: (url: string) => void;
      shortcut: (key: string) => void;
      error: () => void;
    },
  ) {
    this.frame.title = "Formatted message";
    this.frame.className = "formatted-frame";
    this.frame.setAttribute("sandbox", "allow-scripts");
    this.frame.referrerPolicy = "no-referrer";
    this.frame.setAttribute(
      "allow",
      "camera 'none'; microphone 'none'; geolocation 'none'; clipboard-read 'none'; clipboard-write 'none'",
    );
    this.frame.srcdoc = documentSource;
    window.addEventListener("message", this.receive);
    window.addEventListener("scroll", this.place, true);
    window.addEventListener("resize", this.place);
    document.body.append(this.frame);
    this.place();
    this.startup = setTimeout(() => this.events.error(), 10000);
  }
  private receive = (event: MessageEvent) => {
    const value = event.data;
    if (
      event.source !== this.frame.contentWindow ||
      !value ||
      value.generation !== this.generation
    )
      return;
    if (value.type === "ready") {
      clearTimeout(this.startup);
      this.booted = true;
      if (this.configuration)
        this.send({ type: "configure", ...JSON.parse(this.configuration) });
    } else if (
      value.type === "content" &&
      Number.isSafeInteger(value.layout) &&
      value.layout > this.layout &&
      Array.isArray(value.blocks) &&
      value.blocks.every((b: unknown) => typeof b === "string")
    ) {
      this.layout = value.layout;
      this.highlighted = "";
      this.events.content(value.blocks, !!value.hasQuotes);
    } else if (value.type === "error") this.events.error();
    else if (
      this.placeholder?.isConnected &&
      !this.frame.hidden &&
      !document.querySelector("dialog[open]")
    ) {
      if (value.type === "link" && typeof value.url === "string")
        this.events.link(value.url);
      if (value.type === "shortcut" && typeof value.key === "string")
        this.events.shortcut(value.key);
    }
  };
  private send(value: object) {
    this.frame.contentWindow?.postMessage(
      { ...value, generation: this.generation },
      "*",
    );
  }
  configure(dark: boolean, quotes: boolean, shortcuts: string[]) {
    const value = JSON.stringify({ dark, quotes, shortcuts });
    if (this.configuration === value) return;
    this.configuration = value;
    if (this.booted) this.send({ type: "configure", dark, quotes, shortcuts });
  }
  highlight(find: MessageFind) {
    if (!this.layout || !this.placeholder?.isConnected) return;
    const key = [
      this.layout,
      find.revision,
      find.active,
      find.open,
      find.pending,
    ].join(":");
    if (key === this.highlighted) return;
    this.highlighted = key;
    const jump =
      find.open &&
      !find.pending &&
      find.hits.length > 0 &&
      find.jump !== this.jump;
    if (jump) {
      this.jump = find.jump;
      const viewport = this.placeholder.closest(".reader-content");
      if (viewport)
        viewport.scrollTop +=
          this.placeholder.getBoundingClientRect().top -
          viewport.getBoundingClientRect().top;
      this.place();
    }
    this.send({
      type: "highlight",
      layout: this.layout,
      revision: find.revision,
      hits: find.open ? find.hits : [],
      active: find.active,
      jump,
    });
  }
  attach(placeholder?: HTMLElement) {
    this.observer.disconnect();
    this.placeholder = placeholder;
    if (placeholder) {
      this.observer.observe(placeholder);
      if (placeholder.parentElement)
        this.observer.observe(placeholder.parentElement);
    }
    this.place();
  }
  private place = () => {
    const node = this.placeholder,
      parent = node?.closest(".reader-content");
    if (!node?.isConnected || !parent) {
      this.frame.hidden = true;
      return;
    }
    const rect = node.getBoundingClientRect(),
      clip = parent.getBoundingClientRect();
    const top = Math.max(rect.top, clip.top, 0),
      left = Math.max(rect.left, clip.left, 0);
    const bottom = Math.min(rect.bottom, clip.bottom, innerHeight),
      right = Math.min(rect.right, clip.right, innerWidth);
    this.frame.hidden =
      bottom <= top || right <= left || !rect.width || !rect.height;
    Object.assign(this.frame.style, {
      top: `${rect.top}px`,
      left: `${rect.left}px`,
      width: `${rect.width}px`,
      height: `${rect.height}px`,
      clipPath: `inset(${Math.max(0, top - rect.top)}px ${Math.max(0, rect.right - right)}px ${Math.max(0, rect.bottom - bottom)}px ${Math.max(0, left - rect.left)}px)`,
    });
  };
  dispose() {
    clearTimeout(this.startup);
    this.observer.disconnect();
    window.removeEventListener("message", this.receive);
    window.removeEventListener("scroll", this.place, true);
    window.removeEventListener("resize", this.place);
    this.frame.remove();
  }
}
