import { PrintController } from "./printing_controller";
import { MessageFind, SearchWorker } from "./message_find";
import { FormattedFrame } from "./formatted_frame";
import type { PreparedMessage } from "./formatted_content";
import type { ReceivedAttachment } from "./attachments";
import {
  type Action,
  type Mail,
  type Draft,
  type CalendarEntry,
  type Preferences,
  Workspace,
} from "./model";
import "./style.css";
import { GatewayRepository } from "./provider";
import { accountPanel } from "./accounts";

const paths: Record<string, string> = {
  up: "m6 15 6-6 6 6",
  down: "m6 9 6 6 6-6",
  mail: "M3 5h18v14H3z M3 5l9 7 9-7",
  inbox: "M4 4h16l2 12v4H2v-4z M2 16h6l2 3h4l2-3h6",
  calendar: "M4 5h16v16H4z M4 10h16 M8 2v6 M16 2v6",
  edit: "m15 4 5 5 M4 16 16 4l4 4L8 20H4z",
  archive: "M3 3h18v5H3z M5 8v13h14V8 M10 12h4",
  trash: "M3 6h18 M9 6V3h6v3 M5 6l1 15h12l1-15 M9 10v7 M15 10v7",
  flag: "M5 22V3 M5 3c5-4 9 4 14 0v11c-5 4-9-4-14 0",
  move: "M3 6h7l2 3h9v12H3z M11 13l3 3-3 3 M7 16h7",
  reply: "m10 6-6 6 6 6 M4 12h9c4 0 7 2 7 6",
  print: "M6 9V3h12v6 M6 18H3V9h18v9h-3 M6 14h12v7H6z M17 11h1",
  forward: "m14 6 6 6-6 6 M20 12h-9c-4 0-7 2-7 6",
  search: "M20 20l-5-5 M17 10a7 7 0 1 0-14 0 7 7 0 0 0 14 0",
  refresh: "M20 10a8 8 0 1 0-2 8 M20 3v7h-7",
  settings: "M4 7h16 M4 17h16 M8 4v6 M16 14v6",
  chevron: "m9 5 7 7-7 7",
  back: "m15 5-7 7 7 7",
  close: "m6 6 12 12 M6 18 18 6",
  send: "m3 3 19 9-19 9 3-9z M6 12h16",
  expand: "M8 3H3v5 M16 3h5v5 M3 16v5h5 M21 16v5h-5",
  file: "M14 2H4v20h16V8z M14 2v6h6",
  menu: "M3 6h18 M3 12h18 M3 18h18",
  check: "m4 12 5 5L20 6",
  lock: "M5 10h14v11H5z M8 10V6a4 4 0 0 1 8 0v4",
};
function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className = "",
  text?: string,
) {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}
function icon(name: string) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "1.5");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  svg.setAttribute("aria-hidden", "true");
  const p = document.createElementNS(svg.namespaceURI, "path");
  p.setAttribute("d", paths[name] ?? paths.mail);
  svg.append(p);
  return svg;
}
function button(
  label: string,
  fn: () => void,
  iconName?: string,
  only = false,
) {
  const b = el("button", only ? "icon-button" : "button");
  b.type = "button";
  b.setAttribute("aria-label", label);
  if (only) b.title = label;
  if (iconName) b.append(icon(iconName));
  if (!only) b.append(el("span", "", label));
  b.onclick = fn;
  return b;
}
function field(
  label: string,
  value: string,
  onInput: (value: string) => void,
  multiline = false,
) {
  const wrap = el("label", "field");
  wrap.append(el("span", "", label));
  const input = multiline ? el("textarea") : el("input");
  input.value = value;
  input.setAttribute("aria-label", label);
  input.setAttribute(
    "placeholder",
    label === "Search conversations" ? "Search conversations…" : "",
  );
  input.dataset.focus = label;
  input.oninput = () => onInput(input.value);
  wrap.append(input);
  return wrap;
}
function select(
  label: string,
  value: string,
  choices: string[],
  onChange: (v: string) => void,
) {
  const wrap = el("label", "select-field");
  wrap.append(el("span", "", label));
  const input = el("select");
  input.setAttribute("aria-label", label);
  for (const name of choices) {
    const o = el("option", "", name);
    o.value = name;
    o.selected = value === name;
    input.append(o);
  }
  input.onchange = () => onChange(input.value);
  wrap.append(input);
  return wrap;
}
function modal(title: string) {
  const d = el("dialog");
  d.setAttribute("aria-label", title);
  const top = el("div", "dialog-heading");
  top.append(
    el("h2", "", title),
    button("Close", () => d.close(), "close", true),
  );
  d.append(top);
  d.addEventListener("close", () => d.remove());
  document.body.append(d);
  d.showModal();
  return d;
}

export function mount(
  w: Workspace,
  login?: { email: string; signOut: () => void },
) {
  const root = document.querySelector<HTMLDivElement>("#app")!;
  let tab = "Mail",
    fullReader = false,
    searchTimer: ReturnType<typeof setTimeout> | undefined;
  const searchWorker = new SearchWorker();
  const find = new MessageFind(searchWorker.search);
  let quoteState: { id: string; mode: string; open: boolean } | undefined;
  let findRenderQueued = false,
    findJump = -1;
  find.addEventListener("change", () => {
    if (!findRenderQueued) {
      findRenderQueued = true;
      queueMicrotask(() => {
        findRenderQueued = false;
        render();
      });
    }
  });
  const blurred = () =>
    queueMicrotask(() => {
      if (!document.hasFocus()) void w.finishReading();
    });
  const hidden = () => {
    if (document.hidden) void w.finishReading();
  };
  window.addEventListener("blur", blurred);
  document.addEventListener("visibilitychange", hidden);
  window.addEventListener("pagehide", (event) => {
    // A page retained by browser history resumes with the same controls/model.
    if (!event.persisted) {
      w.dispose();
      window.removeEventListener("blur", blurred);
      document.removeEventListener("visibilitychange", hidden);
      find.dispose();
      searchWorker.dispose();
      printer?.dispose();
      formattedFrame?.dispose();
      gateway?.formattedMessages.cancel();
      systemAppearance.removeEventListener("change", appearanceChanged);
    }
  });
  function openFind() {
    if (!w.readerMessage) return;
    find.show();
    queueMicrotask(() => {
      const input = root.querySelector<HTMLInputElement>(
        'input[aria-label="Find in message"]',
      );
      input?.focus();
      input?.select();
    });
  }
  function closeFind() {
    find.close();
  }
  function findBar() {
    const bar = el("div", "message-find");
    bar.setAttribute("role", "search");
    bar.setAttribute("aria-label", "Search this message");
    const input = field("Find in message", find.query, (value) =>
      find.setQuery(value),
    );
    input.querySelector("input")!.placeholder = "Find in message";
    input.querySelector("input")!.onkeydown = (e) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        closeFind();
      } else if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
        find.next(e.shiftKey);
      } else if (
        (e.ctrlKey || e.metaKey) &&
        e.key.toLowerCase() === "f" &&
        w.preferences.shortcuts.find === "Control+f"
      ) {
        e.preventDefault();
        e.stopPropagation();
        (e.target as HTMLInputElement).select();
      }
    };
    const controls = el("div", "message-find-controls");
    const status = el("span", "find-status", find.error ?? find.status);
    status.setAttribute("role", "status");
    controls.append(status);
    if (find.error) controls.append(button("Retry Find", () => find.retry()));
    const matchCase = button("Match case", () => find.toggleCase());
    matchCase.textContent = "Aa";
    matchCase.title = "Match case";
    matchCase.setAttribute("aria-pressed", String(find.matchCase));
    const previous = button(
        "Previous match",
        () => find.next(true),
        "up",
        true,
      ),
      next = button("Next match", () => find.next(), "down", true);
    previous.disabled = next.disabled = find.pending || !find.hits.length;
    const close = button("Close Find", closeFind, "close", true);
    for (const b of [matchCase, previous, next, close])
      b.dataset.focus = b.getAttribute("aria-label")!;
    controls.append(matchCase, previous, next, close);
    bar.append(input, controls);
    return bar;
  }
  function foundText(text: string, block: number) {
    const content = el("div", "message-body");
    let offset = 0;
    if (find.open)
      for (let i = 0; i < find.hits.length; i++) {
        const hit = find.hits[i];
        if (hit.block !== block) continue;
        content.append(document.createTextNode(text.slice(offset, hit.start)));
        const mark = el(
          "mark",
          i === find.active ? "find-hit active" : "find-hit",
          text.slice(hit.start, hit.end),
        );
        mark.dataset.findHit = String(i);
        content.append(mark);
        offset = hit.end;
      }
    content.append(document.createTextNode(text.slice(offset)));
    return content;
  }
  let month = new Date(w.events[0]?.start ?? Date.now());
  let sidebarOpen = false;
  const gateway =
    w.repository instanceof GatewayRepository ? w.repository : undefined;
  const printer = gateway
    ? new PrintController(
        () => gateway.createPrinter(),
        () => w.changed(),
        (message) => {
          w.error = message;
          w.changed();
        },
      )
    : undefined;
  function printMessage(m: Mail) {
    if (!printer) {
      w.error = "Printing is available for cached account messages.";
      w.changed();
      return;
    }
    printer.open(
      m.id,
      formattedState?.id === m.id && formattedState.plain,
      darkReader() ? "dark" : "light",
    );
  }
  let formattedFrame: FormattedFrame | undefined;
  let formattedState:
    | {
        id: string;
        generation: string;
        plain: boolean;
        loading: boolean;
        prepared?: PreparedMessage;
        error?: string;
        blocks: string[];
        hasQuotes: boolean;
      }
    | undefined;
  const systemAppearance = matchMedia("(prefers-color-scheme: dark)");
  const appearanceChanged = () => {
    if (w.preferences.appearance === "system") w.changed();
  };
  systemAppearance.addEventListener("change", appearanceChanged);
  const darkReader = () =>
    w.preferences.appearance === "dark" ||
    (w.preferences.appearance === "system" && systemAppearance.matches);
  function readerShortcuts() {
    return [
      ...Object.values(w.preferences.shortcuts).filter(Boolean),
      "Escape",
      ...(w.preferences.shortcuts.find === "Control+f" ? ["Meta+f"] : []),
      ...(w.preferences.shortcuts.print === "Control+p" ? ["Meta+p"] : []),
    ];
  }
  function reviewLink(value: string) {
    let url: URL;
    try {
      url = new URL(value);
    } catch {
      return;
    }
    if (
      !["http:", "https:", "mailto:"].includes(url.protocol) ||
      url.username ||
      url.password
    )
      return;
    const d = modal("Message link");
    const address = el("p", "message-link-address", url.href);
    const open = el("a", "button", "Open link");
    open.href = url.href;
    open.target = "_blank";
    open.rel = "noopener noreferrer";
    open.onclick = () => d.close();
    const status = el("p", "form-status");
    status.setAttribute("role", "status");
    const copy = button("Copy address", async () => {
      try {
        await navigator.clipboard.writeText(url.href);
        status.textContent = "Address copied.";
      } catch {
        status.textContent = "Select and copy the address above.";
      }
    });
    d.append(address, open, copy, status);
  }
  function clearFormatted() {
    formattedState = undefined;
    formattedFrame?.dispose();
    formattedFrame = undefined;
    gateway?.formattedMessages.cancel();
  }
  function loadFormatted(m: Mail, plain = false) {
    if (!gateway) return;
    clearFormatted();
    const state = (formattedState = {
      id: m.id,
      generation: crypto.randomUUID(),
      plain,
      loading: true,
      blocks: [] as string[],
      hasQuotes: false,
    } as NonNullable<typeof formattedState>);
    void gateway.formattedMessages
      .load(m.id, {
        generation: state.generation,
        dark: darkReader(),
        quotes: quoteState?.open ?? false,
      })
      .then((prepared) => {
        if (formattedState !== state) return;
        state.prepared = prepared;
        if (prepared.document)
          formattedFrame = new FormattedFrame(
            state.generation,
            prepared.document,
            {
              content: (blocks, hasQuotes) => {
                if (formattedState !== state) return;
                state.blocks = blocks;
                state.hasQuotes = hasQuotes;
                w.changed();
              },
              link: reviewLink,
              shortcut: (key) => {
                if (formattedState === state && !state.plain)
                  handleShortcut(key);
              },
              error: () => {
                if (formattedState !== state) return;
                state.error =
                  "Could not display formatted mail. Use plain text or retry.";
                formattedFrame?.dispose();
                formattedFrame = undefined;
                w.changed();
              },
            },
          );
      })
      .catch((error) => {
        if (formattedState === state) state.error = error.message;
      })
      .finally(() => {
        if (formattedState === state) {
          state.loading = false;
          w.changed();
        }
      });
  }
  function folders() {
    return [
      ...new Set([
        "Inbox",
        "Archive",
        "Sent",
        "Trash",
        "Spam",
        ...[...(gateway?.folders.values() ?? [])].flat(),
      ]),
    ];
  }
  function go(folder: string) {
    tab = "Mail";
    fullReader = false;
    sidebarOpen = false;
    w.navigate(folder);
  }
  function act(action: Action, id = w.selected) {
    if (!id) return;
    if (action === "move") {
      const d = modal("Move message");
      const list = el("div", "folder-choices");
      const render = (query = "") => {
        list.replaceChildren();
        for (const f of folders().filter((f) =>
          f.toLowerCase().includes(query.toLowerCase()),
        ))
          list.append(
            button(
              f,
              () => {
                void w.action(id!, action, f);
                d.close();
              },
              "move",
            ),
          );
      };
      const search = field("Find a folder", "", render);
      const input = search.querySelector("input")!;
      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          list.querySelector("button")?.click();
        }
      });
      d.append(search, list);
      render();
      input.focus();
    } else void w.action(id, action);
  }
  async function outbox() {
    if (!gateway) return;
    const d = modal("Outbox");
    d.classList.add("outbox");
    const content = el("div", "outbox-entries");
    const status = el("p", "form-status");
    status.role = "status";
    d.append(content, status);
    let busy = false;
    let drawGeneration = 0;
    const refresh = button("Refresh Outbox", () => void draw());
    d.append(refresh);
    async function draw() {
      if (!gateway || !d.isConnected) return;
      const generation = ++drawGeneration;
      content.replaceChildren(el("p", "muted", "Loading Outbox…"));
      try {
        const entries = await gateway.outgoing();
        if (!d.isConnected || generation !== drawGeneration) return;
        content.replaceChildren();
        if (!entries.length)
          content.append(
            el("p", "empty", "No outgoing messages need attention."),
          );
        for (const entry of entries) {
          const card = el("section", "settings-card");
          const labels: Record<string, string> = {
            preparing: "Not yet submitted",
            reserved: "Not yet submitted",
            cancelled: "Not sent",
            rejected: "Not sent",
            submitting: "Delivery in progress",
            uncertain: "Delivery not confirmed",
            unknown: "Delivery status unavailable",
            delivered: "Delivery confirmed",
          };
          card.append(
            el("h3", "", entry.draft.subject || "Untitled message"),
            el(
              "p",
              "muted",
              entry.recovery?.action === "marked"
                ? "SMTP delivery unconfirmed; recorded as sent after your review"
                : labels[entry.state] || "Review delivery",
            ),
            el(
              "p",
              "",
              `To: ${entry.draft.to || entry.draft.cc || "Recipients in Bcc"}`,
            ),
          );
          const actions = el("div", "outbox-actions");
          const review = el("label", "checkbox-field");
          const confirmed = el("input");
          confirmed.type = "checkbox";
          review.append(
            confirmed,
            document.createTextNode(
              "I reviewed delivery; another send could create a duplicate",
            ),
          );
          const uncertain =
            ["uncertain", "unknown"].includes(entry.state) &&
            entry.recovery?.action !== "marked";
          if (uncertain)
            card.append(
              el(
                "p",
                "muted",
                "This message may already have been sent. Check your provider's Sent folder or the recipient before making a decision.",
              ),
              review,
            );
          async function recover(
            action:
              | "check"
              | "return"
              | "mark"
              | "local"
              | "sent-check"
              | "sent-copy",
          ) {
            if (busy || !gateway) return;
            busy = true;
            status.textContent = "";
            for (const button of content.querySelectorAll<HTMLButtonElement>(
              "button",
            ))
              button.disabled = true;
            refresh.disabled = true;
            confirmed.disabled = true;
            try {
              const draft =
                action === "sent-check" || action === "sent-copy"
                  ? await gateway.recoverSent(
                      entry.id,
                      action === "sent-copy" ? "copy" : "check",
                      copyConfirmed.checked,
                    )
                  : await gateway.recoverOutgoing(
                      entry.id,
                      action,
                      confirmed.checked,
                    );
              await gateway.load();
              w.drafts = new Map(gateway.drafts.map((d) => [d.id, d]));
              w.addCachedMail(gateway.cached);
              w.notice = draft
                ? "Returned to drafts. Sending requires a new Send action."
                : action === "mark"
                  ? "Recorded as sent after your review."
                  : action === "local"
                    ? "Sent copy kept locally."
                    : action.startsWith("sent-")
                      ? "Sent recovery checked. See Outbox for the saved result."
                      : "Delivery status checked.";
              w.changed();
              await draw();
            } catch (error) {
              if (d.isConnected) {
                await draw();
                status.textContent =
                  error instanceof Error
                    ? error.message
                    : "Could not recover this entry. Retry from Outbox.";
              }
            } finally {
              busy = false;
              refresh.disabled = false;
            }
          }
          const copyConfirmed = el("input");
          copyConfirmed.type = "checkbox";
          const copyReview = el("label", "checkbox-field");
          copyReview.append(
            copyConfirmed,
            document.createTextNode(
              "I checked the server folder; another Sent upload could create a duplicate",
            ),
          );
          if (entry.sentError)
            card.append(el("p", "form-status", entry.sentError));
          const copyLabels: Record<string, string> = {
            pending: "Sent copy pending",
            missing: "No matching provider Sent copy found yet",
            reserved: "Sent copy reserved; upload not confirmed",
            copying: "Sent upload in progress or awaiting its receipt",
            saved: "Provider Sent copy acknowledged",
            failed: "Sent copy needs attention",
            uncertain: "Sent upload not confirmed",
            unknown: "Sent upload receipt unavailable",
            local: "Sent copy kept on this browser",
          };
          if (entry.sent)
            card.append(
              el(
                "p",
                "muted",
                copyLabels[entry.sent.state] ?? "Review Sent copy",
              ),
            );
          if (
            ["delivered", "uncertain", "unknown"].includes(entry.state) &&
            entry.account &&
            entry.wire
          ) {
            actions.append(
              button(
                entry.sent?.state === "saved"
                  ? "Finish saving Sent copy"
                  : "Check server Sent",
                () => void recover("sent-check"),
              ),
            );
            if (
              entry.sent?.state !== "saved" &&
              entry.sent?.state !== "local" &&
              (entry.state === "delivered" ||
                entry.recovery?.action === "marked")
            ) {
              const requiresReview =
                !!entry.sent?.copyId && entry.sent.state !== "reserved";
              const upload = button(
                "Save copy to server Sent",
                () => void recover("sent-copy"),
              );
              upload.disabled = requiresReview;
              if (requiresReview) {
                card.append(copyReview);
                copyConfirmed.onchange = () =>
                  (upload.disabled = !copyConfirmed.checked || busy);
              }
              actions.append(upload);
            }
          }
          actions.append(
            button("Check delivery status", () => void recover("check")),
          );
          if (
            entry.state === "delivered" ||
            entry.sent?.state === "saved" ||
            entry.recovery?.action === "marked"
          )
            actions.append(
              button("Keep local copy", () => void recover("local")),
            );
          else if (entry.state !== "submitting") {
            const back = button(
              "Return to drafts",
              () => void recover("return"),
            );
            back.disabled = uncertain;
            actions.append(back);
            if (uncertain) {
              const mark = button("Record as sent", () => void recover("mark"));
              mark.disabled = true;
              actions.append(mark);
              confirmed.onchange = () => {
                back.disabled = mark.disabled = !confirmed.checked || busy;
              };
            }
          }
          card.append(actions);
          content.append(card);
        }
      } catch (error) {
        if (!d.isConnected || generation !== drawGeneration) return;
        status.textContent =
          error instanceof Error
            ? error.message
            : "Could not load Outbox. Retry.";
      }
    }
    await draw();
  }
  const forwardRequests = new Map<string, string>();
  const forwarding = new Set<string>();
  async function forward(original: Mail) {
    if (forwarding.has(original.id)) return;
    const startingError = w.error;
    forwarding.add(original.id);
    w.changed();
    try {
      let draft: Draft;
      if (gateway) {
        const target = forwardRequests.get(original.id) ?? crypto.randomUUID();
        forwardRequests.set(original.id, target);
        draft = await gateway.forward(original.id, target);
      } else {
        draft = {
          id: crypto.randomUUID(),
          to: "",
          cc: "",
          bcc: "",
          subject: `Fwd: ${original.subject}`,
          body: `\n\n---------- Forwarded message ----------\nFrom: ${original.address}\nSubject: ${original.subject}\n\n${original.body}`,
        };
        await w.repository.saveDraft(draft);
      }
      forwardRequests.delete(original.id);
      if (draft.accountId && gateway?.removedAccounts.has(draft.accountId))
        throw new Error(
          "This account was removed. Open Drafts or choose a connected account.",
        );
      w.rememberDraft(draft);
      w.notice = "Forward saved in Drafts";
      if (w.error === startingError) w.error = null;
      if (
        tab === "Mail" &&
        w.readerMessage?.id === original.id &&
        !document.querySelector("dialog[open]")
      )
        await composer(undefined, draft);
    } catch (error) {
      w.error =
        error instanceof Error
          ? error.message
          : "Could not prepare this forward. Retry.";
    } finally {
      forwarding.delete(original.id);
      w.changed();
    }
  }
  async function composer(original?: Mail, existing?: Draft, all = false) {
    void w.finishReading();
    if (original && gateway && !existing) {
      try {
        existing = await gateway.reply(original.id, all);
      } catch (error) {
        w.error =
          error instanceof Error
            ? error.message
            : "Could not prepare a reply. Refresh this folder.";
        w.changed();
        return;
      }
    }
    const draft: Draft = existing
      ? structuredClone(existing)
      : {
          id: crypto.randomUUID(),
          ...(gateway
            ? { accountId: original?.accountId ?? gateway.accounts[0]?.id }
            : {}),
          to: original?.address ?? "",
          cc: "",
          bcc: "",
          subject: original ? `Re: ${original.subject}` : "",
          revision: 0,
          attachments: [],
          body: original
            ? `\n\n> ${original.body.replaceAll("\n", "\n> ")}`
            : "",
        };
    const d = modal(draft.forward ? "Forward message" : "New message");
    d.classList.add("composer");
    const fields = el("div", "composer-fields");
    const status = el("p", "form-status");
    status.setAttribute("role", "status");
    let autosave: ReturnType<typeof setTimeout> | undefined;
    let writes: Promise<void> = Promise.resolve();
    function edited() {
      draft.revision = (draft.revision ?? 0) + 1;
      clearTimeout(autosave);
      autosave = setTimeout(() => {
        const snapshot = structuredClone(draft);
        writes = writes.then(async () => {
          try {
            await w.repository.saveDraft(snapshot);
            w.rememberDraft(snapshot);
          } catch (error) {
            if (d.isConnected)
              status.textContent =
                error instanceof Error
                  ? error.message
                  : "Could not save. Keep the editor open and retry.";
          }
        });
      }, 500);
    }
    d.addEventListener("close", () => clearTimeout(autosave));
    if (w.repository.preview)
      fields.append(el("p", "muted", "Preview • sending is disabled"));
    if (gateway) {
      const label = el("label", "field");
      label.append(el("span", "", "From account"));
      const input = el("select");
      input.setAttribute("aria-label", "From account");
      for (const account of gateway.accounts) {
        const option = el("option", "", account.email);
        option.value = account.id;
        option.selected = draft.accountId === account.id;
        input.append(option);
      }
      input.onchange = () => {
        draft.accountId = input.value;
        edited();
      };
      label.append(input);
      fields.append(label);
    }
    for (const key of ["to", "cc", "bcc", "subject", "body"] as const) {
      const label =
        key === "body"
          ? "Message"
          : key === "subject"
            ? "Subject"
            : key === "bcc"
              ? "Bcc"
              : key === "cc"
                ? "Cc"
                : "To";
      const view = field(
        label,
        draft[key],
        (v) => {
          if (v !== draft[key]) {
            draft[key] = v;
            edited();
          }
        },
        key === "body",
      );
      fields.append(view);
    }
    const filePanel = el("div", "draft-files");
    const fileInput = el("input");
    fileInput.type = "file";
    fileInput.multiple = true;
    fileInput.hidden = true;
    fileInput.setAttribute("aria-label", "Choose attachments");
    const attach = button("Attach files", () => fileInput.click());
    function renderFiles() {
      filePanel.replaceChildren();
      for (const file of draft.attachments ?? []) {
        const row = el("div", "draft-file");
        row.append(el("span", "", `${file.name} (${file.size} bytes)`));
        const remove = button(
          `Remove ${file.name}`,
          () => void changeFiles([], file.id),
          "close",
          true,
        );
        remove.disabled = busy || deliveryLocked;
        row.append(remove);
        filePanel.append(row);
      }
    }
    async function changeFiles(files: File[], remove?: string) {
      if (!gateway || busy || deliveryLocked) return;
      const previous = draft.attachments;
      clearTimeout(autosave);
      busy = true;
      if (remove)
        draft.attachments = (draft.attachments ?? []).filter(
          (f) => f.id !== remove,
        );
      renderFiles();
      setControls(true);
      try {
        await writes;
        await w.repository.saveDraft(draft);
        draft.attachments = remove
          ? await gateway.removeFile(draft.id, remove)
          : await gateway.addFiles(draft.id, files);
        w.rememberDraft(draft);
        status.textContent = "";
      } catch (error) {
        draft.attachments = previous;
        status.textContent =
          error instanceof Error
            ? error.message
            : "Could not save attachments. Retry.";
      } finally {
        busy = false;
        renderFiles();
        setControls(false);
      }
    }
    fileInput.onchange = () => {
      const files = [...(fileInput.files ?? [])];
      fileInput.value = "";
      if (files.length) void changeFiles(files);
    };
    if (gateway) fields.append(filePanel, attach, fileInput);
    const actions = el("div", "dialog-actions");
    let busy = false,
      deliveryLocked = false;
    function setControls(disabled: boolean) {
      for (const input of d.querySelectorAll<
        HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement
      >("input,textarea,select"))
        input.disabled = disabled || deliveryLocked;
      for (const b of d.querySelectorAll<HTMLButtonElement>("button"))
        b.disabled = disabled;
      attach.disabled = disabled || deliveryLocked;
      for (const b of filePanel.querySelectorAll<HTMLButtonElement>("button"))
        b.disabled = disabled || deliveryLocked;
    }
    async function save(send: boolean) {
      if (busy) return;
      if (
        send &&
        (!(draft.to.trim() || draft.cc.trim() || draft.bcc.trim()) ||
          !draft.subject.trim())
      ) {
        status.textContent = "Add a recipient and subject before sending.";
        return;
      }
      clearTimeout(autosave);
      busy = true;
      setControls(true);
      for (const b of actions.querySelectorAll("button")) b.disabled = true;
      for (const input of d.querySelectorAll<
        HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement
      >("input,textarea,select"))
        input.disabled = true;
      try {
        await writes;
        if (send) await w.repository.send(draft);
        else await w.repository.saveDraft(draft);
        if (send) w.drafts.delete(draft.id);
        else w.rememberDraft(draft);
        if (send) {
          if (gateway) w.addCachedMail(gateway.cached);
          w.notice = "Message accepted by SMTP";
        }
        d.close();
        w.changed();
      } catch (error) {
        status.textContent =
          error instanceof Error
            ? error.message
            : send
              ? "Mail was not sent. Your draft is still open; connect a provider before retrying."
              : "Draft could not be saved. Keep this editor open and retry.";
      } finally {
        busy = false;
        for (const b of actions.querySelectorAll("button")) b.disabled = false;
        for (const input of d.querySelectorAll<
          HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement
        >("input,textarea,select"))
          input.disabled = false;
        setControls(false);
        void updateDelivery();
      }
    }
    d.addEventListener("cancel", (e) => {
      e.preventDefault();
      void save(false);
    });
    d.querySelector<HTMLButtonElement>('[aria-label="Close"]')!.onclick = () =>
      void save(false);
    async function updateDelivery() {
      if (!gateway || !d.isConnected) return;
      setControls(true);
      let delivery;
      try {
        draft.attachments = await gateway.attachments(draft.id);
        renderFiles();
        delivery = await gateway.delivery(draft);
      } catch {
        deliveryLocked = true;
        setControls(false);
        status.textContent =
          "Could not check this saved draft. Reopen it before editing or sending.";
        return;
      }
      deliveryLocked = !!delivery;
      setControls(false);
      if (delivery) {
        for (const input of d.querySelectorAll<
          HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement
        >("input,select,textarea"))
          input.disabled = true;
        const send = actions.querySelector<HTMLButtonElement>(
          '[aria-label="Send"], [aria-label="Check delivery status"]',
        );
        if (send) {
          send.setAttribute("aria-label", "Check delivery status");
          send.textContent = "Check delivery status";
        }
        if (!status.textContent)
          status.textContent =
            "A delivery record is saved for this draft. Check its status before composing another copy.";
      }
    }
    actions.append(
      button("Save draft", () => void save(false)),
      button("Send", () => void save(true), "send"),
    );
    d.append(fields, status, actions);
    renderFiles();
    setControls(!!gateway);
    void updateDelivery();
  }
  function editEvent(original?: CalendarEntry) {
    const day = new Date(
      month.getFullYear(),
      month.getMonth(),
      original ? new Date(original.start).getDate() : 1,
    );
    const entry: CalendarEntry = original
      ? { ...original }
      : {
          id: crypto.randomUUID(),
          title: "",
          start: day.toISOString(),
          end: new Date(+day + 86400000).toISOString(),
          calendar: "Personal",
          location: "",
          readOnly: false,
        };
    const d = modal(
      entry.readOnly ? "View event" : original ? "Edit event" : "New event",
    );
    const status = el("p", "form-status");
    status.setAttribute("role", "status");
    d.append(
      el(
        "p",
        "muted",
        `${day.toLocaleDateString(undefined, { dateStyle: "long" })} · All day`,
      ),
      field("Event title", entry.title, (v) => (entry.title = v)),
      field("Location", entry.location, (v) => (entry.location = v)),
    );
    if (entry.readOnly) {
      for (const input of d.querySelectorAll("input")) input.readOnly = true;
      d.append(el("p", "muted", "This calendar is read-only."));
    } else
      d.append(
        button("Save event", async () => {
          if (!entry.title.trim()) {
            status.textContent = "Enter an event title.";
            return;
          }
          try {
            await w.repository.saveEvent(entry);
            w.events = [...w.events.filter((e) => e.id !== entry.id), entry];
            d.close();
            w.changed();
          } catch {
            status.textContent =
              "Event could not be saved. Keep the form open and retry.";
          }
        }),
      );
    d.append(status);
  }
  function sidebar() {
    const aside = el("aside", sidebarOpen ? "sidebar open" : "sidebar");
    aside.setAttribute("aria-label", "Navigation");
    const brand = el("div", "brand");
    const img = el("img");
    img.src = "./logo-light.webp";
    img.alt = "";
    img.className = "logo light-logo";
    const dark = el("img");
    dark.src = "./logo-dark.webp";
    dark.alt = "";
    dark.className = "logo dark-logo";
    brand.append(img, dark, el("span", "", "shep"));
    aside.append(brand);
    const nav = el("nav");
    nav.setAttribute("aria-label", "Workspace");
    for (const [name, i] of [
      ["Mail", "mail"],
      ["Calendar", "calendar"],
    ]) {
      const b = button(
        name,
        () => {
          void w.finishReading();
          tab = name;
          fullReader = false;
          w.changed();
        },
        i,
      );
      if (tab === name) b.classList.add("active");
      nav.append(b);
    }
    const compose = button("New message", () => composer(), "edit");
    compose.classList.add("primary", "compose-button");
    nav.append(compose);
    for (const [folder, i] of [
      ["Inbox", "inbox"],
      ["Flagged", "flag"],
      ["Sent", "send"],
      ["Archive", "archive"],
      ["Drafts", "edit"],
      ["Trash", "trash"],
    ]) {
      const b = button(
        folder === "Inbox" ? `Inbox (${w.unread})` : folder,
        () => {
          if (folder === "Flagged") {
            w.filter = "Flagged";
            go("Inbox");
          } else {
            w.filter = "All";
            go(folder);
          }
        },
        i,
      );
      if (tab === "Mail" && w.folder === folder && w.filter !== "Flagged")
        b.classList.add("active");
      nav.append(b);
    }
    if (gateway) nav.append(button("Outbox", () => void outbox(), "send"));
    const accounts = el("div", "accounts");
    for (const name of new Set([
      ...w.mail.map((m) => m.account),
      ...(gateway?.accounts.map((a) => a.email) ?? []),
    ])) {
      const details = el("details");
      details.open = true;
      const summary = el("summary");
      summary.append(icon("mail"), el("span", "", name));
      details.append(
        summary,
        button(
          "Inbox",
          () => {
            tab = "Mail";
            w.navigate("Inbox", name);
          },
          "inbox",
        ),
      );
      const account = gateway?.accounts.find((a) => a.email === name);
      for (const folder of (account && gateway?.folders.get(account.id)) ??
        []) {
        if (folder !== "Inbox")
          details.append(
            button(
              folder,
              () => {
                tab = "Mail";
                w.navigate(folder, name);
              },
              "mail",
            ),
          );
      }
      accounts.append(details);
    }
    nav.append(accounts);
    aside.append(nav);
    const prefs = button(
      "Preferences",
      () => {
        void w.finishReading();
        tab = "Preferences";
        fullReader = false;
        w.changed();
      },
      "settings",
    );
    prefs.classList.add("preferences-nav");
    if (tab === "Preferences") prefs.classList.add("active");
    aside.append(prefs);
    if (login) {
      const identity = el("p", "identity", login.email);
      aside.append(
        identity,
        button(
          "Sign out",
          () => {
            void w.finishReading().then(() => {
              find.dispose();
              searchWorker.dispose();
              printer?.dispose();
              login.signOut();
            });
          },
          "lock",
        ),
      );
    }
    return aside;
  }
  function split(
    name: string,
    value: number,
    min: number,
    max: number,
    change: (n: number) => void,
  ) {
    const s = el("div", "splitter");
    s.tabIndex = 0;
    s.role = "separator";
    s.setAttribute("aria-label", name);
    s.setAttribute("aria-orientation", "vertical");
    s.setAttribute("aria-valuemin", `${min}`);
    s.setAttribute("aria-valuemax", `${max}`);
    s.setAttribute("aria-valuenow", `${value}`);
    s.onkeydown = (e) => {
      if (["ArrowLeft", "ArrowRight"].includes(e.key)) {
        e.preventDefault();
        change(
          Math.min(
            max,
            Math.max(min, value + (e.key === "ArrowRight" ? 10 : -10)),
          ),
        );
      }
    };
    s.onpointerdown = (e) => {
      const start = e.clientX;
      s.setPointerCapture(e.pointerId);
      const move = (ev: PointerEvent) => {
        const next = Math.min(max, Math.max(min, value + ev.clientX - start));
        root.style.setProperty(
          name === "Sidebar width" ? "--sidebar" : "--list",
          `${next}px`,
        );
      };
      const up = (ev: PointerEvent) => {
        s.removeEventListener("pointermove", move);
        s.removeEventListener("pointerup", up);
        change(Math.min(max, Math.max(min, value + ev.clientX - start)));
      };
      s.addEventListener("pointermove", move);
      s.addEventListener("pointerup", up);
    };
    return s;
  }
  function open(m: Mail) {
    attachmentState = undefined;
    w.beginReading(m.id);
  }
  function inbox() {
    const box = el(
      "section",
      fullReader ? "mail-shell full-reader" : "mail-shell",
    );
    box.setAttribute("aria-label", "Mail workspace");
    if (w.folder === "Drafts") {
      box.classList.add("draft-list");
      for (const d of w.drafts.values())
        box.append(
          button(
            d.subject || "Untitled draft",
            () => composer(undefined, d),
            "edit",
          ),
        );
      if (!w.drafts.size) box.append(el("p", "empty", "No saved drafts"));
      return box;
    }
    const list = el("section", "mail-list");
    list.setAttribute("aria-label", "Message list");
    const controls = el("div", "list-controls");
    controls.append(
      select("Filter", w.filter, ["All", "Unread", "Flagged"], (v) => {
        void w.finishReading();
        w.filter = v;
        w.page = 0;
        w.changed();
      }),
      select(
        "Sort",
        w.newestFirst ? "Newest first" : "Oldest first",
        ["Newest first", "Oldest first"],
        (v) => {
          void w.finishReading();
          w.newestFirst = v === "Newest first";
          w.page = 0;
          w.changed();
        },
      ),
    );
    const search = field("Search conversations", w.query, (v) => {
      clearTimeout(searchTimer);
      searchTimer = setTimeout(() => w.search(v), 100);
    });
    search
      .querySelector("input")
      ?.addEventListener("focus", () => void w.finishReading());
    search.classList.add("search-field");
    controls.append(search);
    list.append(controls);
    const rows = el("div", "rows");
    rows.tabIndex = 0;
    rows.dataset.scroll = "mail-list";
    rows.setAttribute("aria-label", "Emails");
    for (const m of w.visible) {
      const row = el(
        "article",
        `mail-row${w.selected === m.id ? " selected" : ""}${m.unread ? " unread" : ""}`,
      );
      row.dataset.id = m.id;
      const main = button(m.subject, () => open(m));
      main.className = "row-open";
      main.ondblclick = () => {
        w.beginReading(m.id);
        fullReader = true;
        w.changed();
      };
      const top = el("div", "row-top");
      if (w.preferences.avatars) top.append(el("span", "avatar", m.sender[0]));
      top.append(
        el("span", "sender", m.sender),
        el(
          "time",
          "",
          new Date(m.date).toLocaleTimeString("en-GB", {
            hour: "2-digit",
            minute: "2-digit",
          }),
        ),
      );
      main.replaceChildren(top, el("span", "subject", m.subject));
      if (w.preferences.previewLines) {
        const preview = el("span", "preview", m.preview);
        preview.style.setProperty("--lines", `${w.preferences.previewLines}`);
        main.append(preview);
      }
      const meta = el("span", "row-meta", m.account);
      if (m.attachments.length) meta.append(icon("file"));
      if (m.unread) meta.append(el("span", "unread-dot"));
      main.append(meta);
      const flag = button(
        `${m.starred ? "Unflag" : "Flag"} ${m.subject}`,
        () => void w.action(m.id, "star"),
        "flag",
        true,
      );
      if (m.starred) flag.classList.add("flagged");
      row.append(main, flag);
      rows.append(row);
    }
    if (!w.visible.length) {
      const empty = el("div", "empty");
      empty.append(
        icon("inbox"),
        el(
          "h2",
          "",
          w.query
            ? "No matching mail"
            : w.repository.preview
              ? "All clear"
              : "Welcome to Shep",
        ),
        el(
          "p",
          "",
          w.query
            ? "Try another sender, subject or phrase."
            : w.repository.preview
              ? "No messages in this view."
              : gateway?.accounts.length
                ? "No cached messages in this view. Refresh to check the mail server."
                : "Add a mail account in Preferences to get started.",
        ),
      );
      rows.append(empty);
    }
    list.append(rows);
    const paging = el("div", "paging");
    const prev = button(
      "Previous page",
      () => {
        void w.finishReading();
        w.page--;
        w.changed();
      },
      "back",
      true,
    );
    prev.disabled = w.page === 0;
    const next = button(
      "Next page",
      () => {
        void w.finishReading();
        w.page++;
        w.changed();
      },
      "chevron",
      true,
    );
    next.disabled = (w.page + 1) * 50 >= w.matching.length;
    paging.append(
      prev,
      el(
        "span",
        "",
        `${w.matching.length ? w.page * 50 + 1 : 0}–${Math.min((w.page + 1) * 50, w.matching.length)} of ${w.matching.length}`,
      ),
      next,
    );
    list.append(paging);
    rows.onkeydown = (e) => {
      if (!["ArrowDown", "ArrowUp", "Enter"].includes(e.key)) return;
      e.preventDefault();
      const index = w.visible.findIndex((m) => m.id === w.selected);
      if (e.key === "Enter" && w.selected) {
        fullReader = true;
        w.changed();
        return;
      }
      const m =
        w.visible[
          Math.min(
            w.visible.length - 1,
            Math.max(0, index + (e.key === "ArrowDown" ? 1 : -1)),
          )
        ];
      if (m) open(m);
      root
        .querySelector(".mail-row.selected")
        ?.scrollIntoView({ block: "nearest" });
    };
    box.append(
      list,
      split("Message list width", w.preferences.listWidth, 280, 560, (n) =>
        w.savePreferences({ ...w.preferences, listWidth: n }),
      ),
      reader(),
    );
    return box;
  }
  let attachmentState:
    | {
        id: string;
        files?: ReceivedAttachment[];
        loading: boolean;
        busy?: string;
        error?: string;
        notice?: string;
      }
    | undefined;
  function incomingFiles(m: Mail) {
    const panel = el("div", "attachments");
    if (!gateway) {
      for (const name of m.attachments)
        panel.append(el("span", "attachment", name));
      return panel;
    }
    if (attachmentState?.id !== m.id) {
      const state = (attachmentState = {
        id: m.id,
        loading: true,
      } as NonNullable<typeof attachmentState>);
      void gateway.incomingAttachments
        .files(m.id)
        .then((files) => (state.files = files))
        .catch((error) => (state.error = error.message))
        .finally(() => {
          state.loading = false;
          if (attachmentState === state) w.changed();
        });
    }
    const state = attachmentState;
    if (state.loading) panel.append(el("span", "", "Loading attachments…"));
    for (const file of state.files ?? []) {
      const save = button(
        `Save ${file.name}`,
        async () => {
          if (state.busy) return;
          state.busy = file.id;
          state.error = undefined;
          state.notice = undefined;
          w.changed();
          try {
            const bytes = await gateway.incomingAttachments.read(m.id, file);
            const url = URL.createObjectURL(
              new Blob([bytes], { type: "application/octet-stream" }),
            );
            const link = document.createElement("a");
            link.href = url;
            link.download = file.name;
            document.body.append(link);
            link.click();
            link.remove();
            setTimeout(() => URL.revokeObjectURL(url), 30000);
            state.notice = `Download started for ${file.name}.`;
          } catch (error) {
            state.error =
              error instanceof Error
                ? error.message
                : "Could not save this attachment. Retry Save.";
          } finally {
            state.busy = undefined;
            if (attachmentState === state) w.changed();
          }
        },
        "file",
      );
      save.disabled = !!state.busy;
      save.append(el("span", "attachment-size", `${file.size} bytes`));
      panel.append(save);
    }
    if (state.error) {
      const status = el("p", "error", state.error);
      status.setAttribute("role", "alert");
      panel.append(status);
      panel.append(
        button(
          "Reload attachments",
          () => {
            attachmentState = undefined;
            w.changed();
          },
          "refresh",
        ),
      );
    }
    if (state.notice) {
      const status = el("p", "", state.notice);
      status.setAttribute("role", "status");
      panel.append(status);
    }
    return panel;
  }
  function reader() {
    const panel = el("section", "reader");
    panel.setAttribute("aria-label", "Message reader");
    const m = w.readerMessage;
    const toolbar = el("div", "reader-toolbar");
    if (fullReader)
      toolbar.append(
        button(
          "Close full reader",
          () => {
            void w.finishReading();
            fullReader = false;
            w.changed();
          },
          "back",
          true,
        ),
      );
    for (const [name, label, i] of [
      ["archive", "Archive", "archive"],
      ["trash", "Trash", "trash"],
      ["read", m?.unread ? "Mark read" : "Mark unread", "mail"],
      ["star", m?.starred ? "Unflag" : "Flag", "flag"],
    ] as const) {
      const b = button(label, () => act(name), i, true);
      b.disabled = !m;
      if (name === "star" && m?.starred) b.classList.add("flagged");
      toolbar.append(b);
    }
    const search = button("Find in message", openFind, "search", true);
    search.disabled = !m;
    toolbar.append(search);
    toolbar.append(el("span", "spacer"));
    const move = button("Move", () => act("move"), "move");
    move.disabled = !m;
    toolbar.append(move);
    const expand = button(
      "Open full reader",
      () => {
        fullReader = true;
        w.changed();
      },
      "expand",
      true,
    );
    expand.disabled = !m;
    toolbar.append(expand);
    panel.append(toolbar);
    if (!m) {
      find.setSource("", []);
      const empty = el("div", "empty");
      empty.append(
        icon("mail"),
        el("h2", "", "A little room to read"),
        el("p", "", "Choose a message from your inbox."),
      );
      panel.append(empty);
      return panel;
    }
    if (quoteState?.id !== m.id || quoteState.mode !== w.preferences.quoteMode)
      quoteState = {
        id: m.id,
        mode: w.preferences.quoteMode,
        open: w.preferences.quoteMode === "Expanded",
      };
    if (gateway && formattedState?.id !== m.id) loadFormatted(m);
    const state = formattedState;
    const html =
      !!state && !state.plain && !!state.prepared?.document && !state.error;
    if (html) panel.classList.add("has-formatted-message");
    const [latest, ...quote] = (state?.prepared?.text ?? m.body).split("\n>");
    if (html) find.setSource(`${m.id}:html:${state.generation}`, state.blocks);
    else
      find.setSource(`${m.id}:plain`, [
        latest,
        ...(quote.length &&
        w.preferences.quoteMode !== "Latest only" &&
        quoteState.open
          ? [quote.join("\n>")]
          : []),
      ]);
    if (find.open) panel.append(findBar());
    const content = el("div", "reader-content");
    content.dataset.scroll = "reader";
    content.append(el("h1", "", m.subject));
    const sender = el("div", "sender-details");
    sender.append(el("span", "avatar", m.sender[0]));
    const details = el("div");
    details.append(
      el("strong", "", m.sender),
      el("p", "", m.address),
      el("p", "", m.account),
    );
    sender.append(
      details,
      el("time", "", new Date(m.date).toLocaleString("en-GB")),
    );
    content.append(sender);
    if (state && (state.loading || state.error || state.prepared?.document)) {
      const options = el("div", "reader-format-controls");
      options.append(
        select(
          "Message format",
          state.plain ? "Plain text" : "Formatted",
          ["Formatted", "Plain text"],
          (value) => {
            state.plain = value === "Plain text";
            w.changed();
          },
        ),
      );
      if (state.loading)
        options.append(el("span", "muted", "Preparing formatted message…"));
      if (
        html &&
        state.hasQuotes &&
        w.preferences.quoteMode !== "Latest only"
      ) {
        const quotes = button(
          quoteState.open ? "Hide quoted history" : "Show quoted history",
          () => {
            quoteState!.open = !quoteState!.open;
            w.changed();
          },
        );
        quotes.setAttribute("aria-expanded", String(quoteState.open));
        options.append(quotes);
      }
      content.append(options);
    }
    if (state?.error) {
      const error = el("p", "error", state.error);
      error.setAttribute("role", "alert");
      content.append(
        error,
        button("Retry formatted message", () => {
          loadFormatted(m, state.plain);
          w.changed();
        }),
      );
    }
    if (html) {
      const count = state.prepared!.remote_images.length;
      if (count)
        content.append(
          el(
            "p",
            "muted reader-image-status",
            `${count} remote image${count === 1 ? "" : "s"} blocked.`,
          ),
        );
      for (const issue of state.prepared!.issues)
        content.append(el("p", "muted", issue));
      const viewport = el("div", "formatted-viewport");
      content.append(viewport);
    } else content.append(foundText(latest, 0));
    if (!html && quote.length && w.preferences.quoteMode !== "Latest only") {
      const quotes = el("details", "quoted");
      quotes.open = quoteState.open;
      quotes.ontoggle = () => {
        if (
          quotes.isConnected &&
          quoteState?.id === m.id &&
          quoteState.open !== quotes.open
        ) {
          quoteState.open = quotes.open;
          w.changed();
        }
      };
      quotes.append(
        el("summary", "", "Quoted history"),
        foundText(quote.join("\n>"), 1),
      );
      content.append(quotes);
    }
    // Older caches can lack counts for Content-Type name parameters. Inspect
    // cached MIME when opening a message so those files are still available.
    const files = incomingFiles(m);
    content.append(files);
    const actions = el("div", "reader-actions");
    const reply = button("Reply", () => composer(m), "reply");
    reply.classList.add("primary");
    actions.append(
      reply,
      button("Reply all", () => void composer(m, undefined, true), "reply"),
      Object.assign(
        button(
          forwarding.has(m.id) ? "Preparing forward…" : "Forward",
          () => void forward(m),
          "forward",
        ),
        { disabled: forwarding.has(m.id) },
      ),
      Object.assign(
        button(
          printer?.preparing(m.id) ? "Preparing print…" : "Print",
          () => printMessage(m),
          "print",
        ),
        { disabled: printer?.preparing(m.id) ?? false },
      ),
    );
    content.append(actions);
    panel.append(content);
    return panel;
  }
  function preferences() {
    const panel = el("section", "settings-panel");
    panel.setAttribute("aria-label", "Preferences");
    const p = w.preferences;
    function card(title: string, ...children: HTMLElement[]) {
      const c = el("section", "settings-card");
      c.append(el("h2", "", title), ...children);
      panel.append(c);
    }
    card(
      "Appearance",
      select("Theme", p.appearance, ["system", "light", "dark"], (v) =>
        w.savePreferences({ ...p, appearance: v as Preferences["appearance"] }),
      ),
    );
    card(
      "Message list",
      select(
        "Preview lines",
        `${p.previewLines}`,
        ["0", "1", "2", "3", "4"],
        (v) => w.savePreferences({ ...p, previewLines: Number(v) }),
      ),
      select(
        "Sender pictures",
        p.avatars ? "Show" : "Hide",
        ["Show", "Hide"],
        (v) => w.savePreferences({ ...p, avatars: v === "Show" }),
      ),
    );
    card(
      "Reading",
      select(
        "Quoted history",
        p.quoteMode,
        ["Collapsed", "Expanded", "Latest only"],
        (v) =>
          w.savePreferences({ ...p, quoteMode: v as Preferences["quoteMode"] }),
      ),
      el(
        "p",
        "muted",
        "External images are blocked. Messages use selectable text.",
      ),
    );
    const shortcuts = el("div", "shortcut-list");
    for (const [key, value] of Object.entries(p.shortcuts)) {
      const row = el("div", "shortcut");
      row.append(el("span", "", key[0].toUpperCase() + key.slice(1)));
      const capture = button(value || "Disabled", () => {
        capture.textContent = "Press a key…";
        capture.onkeydown = (e) => {
          e.preventDefault();
          e.stopPropagation();
          if (e.key === "Escape") {
            capture.textContent = value || "Disabled";
            capture.onkeydown = null;
            return;
          }
          if (["Control", "Meta", "Shift", "Alt"].includes(e.key)) return;
          const combo = [
            e.ctrlKey ? "Control" : e.metaKey ? "Meta" : "",
            e.altKey ? "Alt" : "",
            e.shiftKey ? "Shift" : "",
            e.key.length === 1 ? e.key.toLowerCase() : e.key,
          ]
            .filter(Boolean)
            .join("+");
          if (
            Object.entries(p.shortcuts).some(
              ([k, v]) => k !== key && v === combo,
            )
          ) {
            capture.textContent = "Already assigned";
            return;
          }
          w.savePreferences({
            ...p,
            shortcuts: { ...p.shortcuts, [key]: combo },
          });
        };
      });
      capture.setAttribute("aria-label", `Remap ${key}`);
      row.append(
        capture,
        button(
          `Clear ${key}`,
          () =>
            w.savePreferences({
              ...p,
              shortcuts: { ...p.shortcuts, [key]: "" },
            }),
          "close",
          true,
        ),
      );
      shortcuts.append(row);
    }
    card(
      "Shortcuts",
      shortcuts,
      el(
        "p",
        "muted",
        "Shortcuts apply when you are browsing mail. Typing in a field keeps its normal behavior.",
      ),
    );
    if (gateway)
      panel.append(
        accountPanel(gateway, (removed) => {
          if (removed) {
            w.accountRemoved(removed);
            attachmentState = undefined;
          }
          w.notice = removed
            ? "Account removed from this browser"
            : "Account preferences saved";
          w.error = null;
          w.changed();
        }),
      );
    card(
      "Calendars and backups",
      el(
        "p",
        "",
        "Google calendars and encrypted backups are not connected in this development build.",
      ),
      el(
        "p",
        "muted",
        "Google beta sign-in grants access to Shep. Calendar and Drive authorization will be connected separately.",
      ),
    );
    return panel;
  }
  function calendar() {
    const panel = el("section", "calendar-panel");
    const heading = el("div", "calendar-heading");
    heading.append(
      el(
        "h2",
        "",
        month.toLocaleDateString(undefined, { month: "long", year: "numeric" }),
      ),
      el("span", "spacer"),
      button(
        "Previous month",
        () => {
          month = new Date(month.getFullYear(), month.getMonth() - 1, 1);
          w.changed();
        },
        "back",
        true,
      ),
      button(
        "Next month",
        () => {
          month = new Date(month.getFullYear(), month.getMonth() + 1, 1);
          w.changed();
        },
        "chevron",
        true,
      ),
      button("New event", () => editEvent(), "calendar"),
    );
    panel.append(heading);
    const grid = el("div", "calendar-grid");
    for (const day of ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"])
      grid.append(el("div", "weekday", day));
    const offset =
      (new Date(month.getFullYear(), month.getMonth(), 1).getDay() + 6) % 7;
    const days = new Date(
      month.getFullYear(),
      month.getMonth() + 1,
      0,
    ).getDate();
    for (let i = 0; i < Math.ceil((days + offset) / 7) * 7; i++) {
      const day = i - offset + 1;
      const cell = el("div", "day");
      if (day > 0 && day <= days) {
        cell.append(el("span", "", `${day}`));
        for (const event of w.events.filter((e) => {
          const d = new Date(e.start);
          return (
            d.getFullYear() === month.getFullYear() &&
            d.getMonth() === month.getMonth() &&
            d.getDate() === day
          );
        }))
          cell.append(button(event.title, () => editEvent(event)));
      }
      grid.append(cell);
    }
    panel.append(grid);
    return panel;
  }
  function render() {
    if (formattedState && formattedState.id !== w.readerMessage?.id)
      clearFormatted();
    const active = document.activeElement as HTMLInputElement | null;
    const focus = active?.dataset.focus;
    const selection = active?.selectionStart;
    const selectionEnd = active?.selectionEnd;
    const scrolls = new Map(
      [...root.querySelectorAll<HTMLElement>("[data-scroll]")].map((n) => [
        n.dataset.scroll,
        n.scrollTop,
      ]),
    );
    document.documentElement.dataset.theme = w.preferences.appearance;
    root.style.setProperty("--sidebar", `${w.preferences.sidebarWidth}px`);
    root.style.setProperty("--list", `${w.preferences.listWidth}px`);
    const sizing = el("section", "splitter-region");
    sizing.setAttribute("aria-label", "Pane sizing");
    sizing.append(
      split("Sidebar width", w.preferences.sidebarWidth, 180, 320, (n) =>
        w.savePreferences({ ...w.preferences, sidebarWidth: n }),
      ),
    );
    root.replaceChildren(sidebar(), sizing);
    const main = el("main");
    const header = el("header");
    header.append(
      button(
        "Toggle navigation",
        () => {
          sidebarOpen = !sidebarOpen;
          w.changed();
        },
        "menu",
        true,
      ),
    );
    header.firstElementChild!.classList.add("mobile-menu");
    header.append(el("h1", "", tab === "Mail" ? w.folder : tab));
    if (tab === "Mail")
      header.append(
        el(
          "span",
          "muted",
          w.folder === "Drafts"
            ? `${w.drafts.size} ${w.drafts.size === 1 ? "draft" : "drafts"}`
            : `${w.matching.length} messages · ${w.unread} unread`,
        ),
      );
    header.append(el("span", "spacer"));
    if (w.repository.preview)
      header.append(el("span", "preview-badge", "PREVIEW"));
    if (tab === "Mail")
      header.append(
        button(
          w.syncing ? "Queue refresh" : "Refresh",
          () => void w.refresh(),
          "refresh",
          true,
        ),
      );
    main.append(header);
    if (w.error) {
      const error = el("div", "error-banner");
      error.setAttribute("role", "alert");
      error.append(el("span", "", w.error));
      if (w.retry) error.append(button("Retry", w.retry));
      error.append(
        button(
          "Dismiss error",
          () => {
            w.error = null;
            w.changed();
          },
          "close",
          true,
        ),
      );
      main.append(error);
    }
    main.append(
      tab === "Mail"
        ? inbox()
        : tab === "Calendar"
          ? calendar()
          : preferences(),
    );
    if (w.notice) {
      const status = el("div", "status");
      status.setAttribute("role", "status");
      status.setAttribute("aria-label", "Mail status");
      status.append(el("span", "", w.notice));
      if (!w.moves.visible && w.undo) status.append(button("Undo", w.undo));
      main.append(status);
    }
    if (w.moves.label) {
      const status = el("div", "status");
      status.setAttribute("role", "status");
      status.setAttribute("aria-label", "Move notification");
      status.append(el("span", "", w.moves.label));
      if (w.undo) status.append(button("Undo", w.undo));
      status.append(
        button(
          "Dismiss move notification",
          () => w.moves.dismiss(),
          "close",
          true,
        ),
      );
      main.append(status);
    }
    if (w.undoFailures.size) {
      const failure = el("div", "error-banner");
      failure.setAttribute("role", "alert");
      failure.append(
        el(
          "span",
          "",
          `${w.undoFailures.size} ${w.undoFailures.size === 1 ? "move needs" : "moves need"} review.`,
        ),
        ...([...w.undoFailures].some((r) => !r.restoreCommitted)
          ? [button("Retry Undo", () => w.retryUndos())]
          : []),
        ...([...w.undoFailures].some((r) => r.restoreCommitted)
          ? [button("Refresh restored mail", () => void w.refreshRestored())]
          : []),
        button(
          "Dismiss Undo errors",
          () => w.dismissUndoFailures(),
          "close",
          true,
        ),
      );
      main.append(failure);
    }
    root.append(main);
    for (const n of root.querySelectorAll<HTMLElement>("[data-scroll]"))
      n.scrollTop = scrolls.get(n.dataset.scroll) ?? 0;
    formattedFrame?.attach(
      root.querySelector<HTMLElement>(".formatted-viewport") ?? undefined,
    );
    formattedFrame?.configure(
      darkReader(),
      quoteState?.open ?? false,
      readerShortcuts(),
    );
    if (formattedState && !formattedState.plain && !formattedState.error)
      formattedFrame?.highlight(find);
    if (focus) {
      const target = [
        ...root.querySelectorAll<HTMLInputElement>("[data-focus]"),
      ].find((n) => n.dataset.focus === focus);
      target?.focus();
      if (selection != null)
        target?.setSelectionRange(selection, selectionEnd ?? selection);
    }
    if (
      !root.querySelector(".formatted-viewport") &&
      find.open &&
      !find.pending &&
      find.hits.length &&
      findJump !== find.jump
    ) {
      findJump = find.jump;
      const jump = findJump;
      requestAnimationFrame(() => {
        if (find.open && find.jump === jump)
          root
            .querySelector<HTMLElement>(`[data-find-hit="${find.active}"]`)
            ?.scrollIntoView({ block: "center", inline: "nearest" });
      });
    }
  }
  w.addEventListener("change", render);
  document.addEventListener("keydown", (e) => {
    if (
      document.querySelector("dialog[open]") ||
      e.target instanceof HTMLInputElement ||
      e.target instanceof HTMLTextAreaElement ||
      e.target instanceof HTMLSelectElement ||
      (e.target as HTMLElement).isContentEditable
    )
      return;
    const combo = [
      e.ctrlKey ? "Control" : e.metaKey ? "Meta" : "",
      e.altKey ? "Alt" : "",
      e.shiftKey ? "Shift" : "",
      e.key.length === 1 ? e.key.toLowerCase() : e.key,
    ]
      .filter(Boolean)
      .join("+");
    if (handleShortcut(combo)) e.preventDefault();
  });
  function handleShortcut(combo: string) {
    if (tab !== "Mail" || document.querySelector("dialog[open]")) return false;
    if (combo === "Escape" && find.open) {
      closeFind();
      return true;
    }
    if (combo === "Escape" && fullReader) {
      void w.finishReading();
      fullReader = false;
      w.changed();
      return true;
    }
    let entry = Object.entries(w.preferences.shortcuts).find(
      ([, value]) => value && value === combo,
    );
    if (
      !entry &&
      combo === "Meta+f" &&
      w.preferences.shortcuts.find === "Control+f"
    )
      entry = ["find", "Control+f"];
    if (
      !entry &&
      combo === "Meta+p" &&
      w.preferences.shortcuts.print === "Control+p"
    )
      entry = ["print", "Control+p"];
    if (!entry) return false;
    const action = entry[0];
    if (action === "find") openFind();
    else if (action === "search")
      root
        .querySelector<HTMLInputElement>('[aria-label="Search conversations"]')
        ?.focus();
    else if (action === "reply") {
      const m = w.readerMessage;
      if (m) composer(m);
    } else if (action === "forward") {
      const m = w.readerMessage;
      if (m) void forward(m);
    } else if (action === "print") {
      const m = w.readerMessage;
      if (m) printMessage(m);
    } else if (action === "reader") {
      if (w.selected) {
        w.beginReading(w.selected);
        fullReader = true;
        w.changed();
      }
    } else act(action as Action);
    return true;
  }
  render();
}
