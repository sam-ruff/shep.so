import { GroupUI } from "./bulk_ui";
import { DraftSession } from "./draft_session";
import { observeDraft } from "./draft_revision";
import { connectionActivity, connectionStatus } from "./connection_activity";
import { folderStatus } from "./folder_actions";
import { calendarAfter, calendarBefore, calendarStatus } from "./calendar_actions";
import { folderSteps, type FolderAction, type FolderMutationReview } from "./folder_mutations";
import {
  dialogShortcuts,
  keyCombo,
  keyConsumed,
  shortcutLabel,
  shortcutName,
  type ShortcutKey,
} from "./shortcut_keys";
import { renderReaderTree } from "./reader_actions";
import { openMoveChooser } from "./move_ui";
import { knownFolders } from "./move_candidates";
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
import { clockTime, readerDate, rowDate } from "./format";
import { GatewayRepository } from "./provider";
import { accountPanel } from "./accounts";
import type { ProfilesUI } from "./profiles_ui";

// The desktop client's icon markup (src/ui/components.rs), so both clients
// draw the same stroked shapes. Static strings only; never user content.
const paths: Record<string, string> = {
  up: '<path d="m6 15 6-6 6 6"/>',
  down: '<path d="m6 9 6 6 6-6"/>',
  mail: '<rect x="3" y="5" width="18" height="14" rx="2"/><path d="m3 6 9 7 9-7"/>',
  "mail-open":
    '<path d="m3 9 9-6 9 6v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V9Z"/><path d="m3 9 9 6 9-6M3 20l6-7m12 7-6-7"/>',
  inbox: '<path d="M4 4h16l2 11v5H2v-5L4 4Z"/><path d="M2 15h6l2 3h4l2-3h6"/>',
  calendar:
    '<rect x="3" y="5" width="18" height="16" rx="2"/><path d="M16 3v4M8 3v4M3 11h18M8 15h2M14 15h2"/>',
  edit: '<path d="m15 4 5 5M4 20l4-1L21 6a2 2 0 0 0-3-3L5 16l-1 4ZM13 4H5a2 2 0 0 0-2 2v14a1 1 0 0 0 1 1h14a2 2 0 0 0 2-2v-6"/>',
  archive:
    '<rect x="3" y="3" width="18" height="4" rx="1"/><path d="M5 7v14h14V7M10 11h4"/>',
  trash: '<path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7"/>',
  flag: '<path d="M5 21V4c5-4 9 4 14 0v11c-5 4-9-4-14 0"/>',
  folder: '<path d="M3 7V4h6l2 3h10v13H3V7Z"/>',
  move: '<path d="M3 7V4h6l2 3h10v13H3V7ZM8 14h8m-3-3 3 3-3 3"/>',
  reply: '<path d="m9 4-6 6 6 6M3 10h11a7 7 0 0 1 7 7v3"/>',
  "reply-all": '<path d="m8 5-5 5 5 5m5-10-5 5 5 5M8 10h6a7 7 0 0 1 7 7v3"/>',
  print:
    '<path d="M6 9V3h12v6M6 18H3V9h18v9h-3M6 14h12v7H6z"/><path d="M17 11h1"/>',
  forward: '<path d="m15 4 6 6-6 6m6-6H10a7 7 0 0 0-7 7v3"/>',
  search: '<circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 5 5"/>',
  refresh:
    '<path d="M20 9a8.25 8.25 0 0 0-14-3L3 9m0-6v6h6M4 15a8.25 8.25 0 0 0 14 3l3-3m0 6v-6h-6"/>',
  settings:
    '<path d="m9 3 1-1h4l1 3 3 1 3 3-1 3 1 3-3 3-3 1-1 3h-4l-1-3-3-1-3-3 1-3-1-3 3-3 3-1V3Z"/><circle cx="12" cy="12" r="3"/>',
  chevron: '<path d="m9 5 7 7-7 7"/>',
  back: '<path d="m15 5-7 7 7 7"/>',
  close: '<path d="m6 6 12 12M6 18 18 6"/>',
  send: '<path d="m22 2-7 20-4-9-9-4L22 2ZM22 2 11 13"/>',
  expand: '<path d="M8 3H3v5M16 3h5v5M3 16v5h5M21 16v5h-5"/>',
  file: '<path d="M14 2H5v20h14V7l-5-5ZM14 2v6h5M8 13h8M8 17h6"/>',
  clip: '<path d="m21 11-9 9a6 6 0 0 1-8-8L14 2a4 4 0 0 1 6 6L10 18a2 2 0 0 1-3-3l9-9"/>',
  menu: '<path d="M3 6h18M3 12h18M3 18h18"/>',
  check: '<path d="m5 12 4 4L19 6"/>',
  lock: '<rect x="3" y="11" width="18" height="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>',
  shield:
    '<path d="m12 2 9 4v6c0 5-9 10-9 10S3 17 3 12V6l9-4Z"/><path d="m8 12 3 3 5-6"/>',
  cloud: '<path d="M6 18a5 5 0 0 1-1-10 7 7 0 0 1 13-2 6 6 0 0 1 0 12H6Z"/>',
  download: '<path d="M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
};
export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className = "",
  text?: string,
) {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}
export function icon(name: string) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "1.6");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  svg.setAttribute("aria-hidden", "true");
  svg.dataset.icon = name;
  svg.innerHTML = paths[name] ?? paths.mail;
  return svg;
}
// Initials on the desktop avatar palette, cycled by list position.
function avatar(name: string, index: number) {
  const initials = name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((word) => word[0])
    .join("")
    .toUpperCase();
  return el("span", `avatar avatar-${index % 5}`, initials);
}
export function button(
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
// Desktop placeholder copy for the matching fields.
const placeholders: Record<string, string> = {
  "Search conversations": "Search conversations…",
  To: "name@example.com",
  Cc: "name@example.com",
  Bcc: "name@example.com",
  Subject: "Add a subject",
  Message: "Write your message…",
};
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
  input.setAttribute("placeholder", placeholders[label] ?? "");
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
function checkbox(
  label: string,
  checked: boolean,
  onChange: (checked: boolean) => void,
  disabled = false,
) {
  const wrap = el("label", "checkbox-field");
  const input = el("input");
  input.type = "checkbox";
  input.checked = checked;
  input.disabled = disabled;
  input.onchange = () => onChange(input.checked);
  wrap.append(input, el("span", "", label));
  return wrap;
}
export function modal(title: string) {
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
  profiles?: ProfilesUI,
) {
  const root = document.querySelector<HTMLDivElement>("#app")!;
  let tab = "Mail",
    fullReader = false,
    searchTimer: ReturnType<typeof setTimeout> | undefined;
  let searchInput: string | undefined;
  let shortcutCapture:
    | { key: keyof Preferences["shortcuts"]; conflict: boolean }
    | undefined;
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
  const groupUI = gateway ? new GroupUI(w, gateway) : undefined;
  let activitySummary = "", activityReading: Promise<void> | undefined, activityAgain = false;
  function refreshActivitySummary() {
    if (!gateway?.actionActivity) return;
    activityAgain = true;
    return (activityReading ??= readActivitySummary(gateway, gateway.actionActivity));
  }
  async function readActivitySummary(gateway: GatewayRepository, activity: NonNullable<GatewayRepository["actionActivity"]>) {
    try {
      do {
        activityAgain = false;
        let summary: string;
        try {
          const [page, connections, folders, calendars] = await Promise.all([
            activity.page(), connectionActivity(gateway), gateway.folderActivity?.page(), gateway.calendar?.journal.summary(),
          ]);
          const count = page.rows.length + connections.rows.length + (folders?.rows.length ?? 0) + (calendars?.pending ?? 0);
          summary = count ? `${count}${page.next || connections.more || folders?.next ? "+" : ""}` : "";
        } catch { summary = "!"; }
        if (summary !== activitySummary) { activitySummary = summary; render(); }
      } while (activityAgain);
    } finally {
      // Cleared in the same turn as the last check, so no request is missed.
      activityReading = undefined;
    }
  }
  w.addEventListener("change", () => void refreshActivitySummary());
  void refreshActivitySummary();
  // Group progress waits for this summary instead of stacking reads on it.
  gateway?.groups.addProgressConsumer(() => activityReading);
  function newFolder(account: string) {
    if (!gateway?.folderActivity) return;
    const d = modal("Create folder"), name = el("input"), parent = el("select"), status = el("p", "form-status");
    const id = crypto.randomUUID();
    name.type = "text"; name.maxLength = 1024; name.setAttribute("aria-label", "Folder name");
    parent.setAttribute("aria-label", "Parent folder");
    const root = el("option", "", "Account root"); root.value = ""; parent.append(root);
    for (const folder of gateway.folders.get(account) ?? []) {
      const option = el("option", "", folder); option.value = folder; parent.append(option);
    }
    status.role = "status";
    const save = button("Create folder", async () => {
      save.disabled = true; name.disabled = true; parent.disabled = true;
      status.textContent = "Saving folder request…";
      try {
        const job = await gateway!.createFolder(account, parent.value || null, name.value, id);
        status.textContent = job.status === "Succeeded" ? "Folder saved on this device." : "Folder request saved. Server confirmation will appear in Folder changes.";
        w.changed();
        save.remove();
        d.append(button("Review folder changes", () => { d.close(); void folderActivity(); }));
      } catch (error) {
        status.textContent = error instanceof Error ? error.message : "The request could not be saved. Keep this name and retry.";
        save.disabled = false; name.disabled = false; parent.disabled = false;
      }
    });
    d.append(el("p", "muted", "The folder becomes available after its saved request is confirmed."), parent, name, status, save);
    name.focus();
  }
  function manageFolders(account: string) {
    if (!gateway?.folderActivity) return;
    const d = modal("Change folder"), source = el("select"), operation = el("select"), destination = el("input"), parent = el("select"), status = el("p", "form-status"), details = el("div");
    source.setAttribute("aria-label", "Folder to change");
    operation.setAttribute("aria-label", "Folder change");
    destination.setAttribute("aria-label", "New folder name"); destination.maxLength = 1024;
    parent.setAttribute("aria-label", "Destination parent");
    const root = el("option", "", "Account root"); root.value = ""; parent.append(root);
    for (const name of gateway.folders.get(account) ?? []) {
      const option = el("option", "", name); option.value = name; source.append(option);
      const target = el("option", "", name); target.value = name; parent.append(target);
    }
    for (const label of ["Rename", "Move", "Delete"]) operation.append(el("option", "", label));
    let review: FolderMutationReview | undefined;
    const requestId = crypto.randomUUID();
    const confirm = button("Confirm folder change", async () => {
      if (!review) return;
      confirm.disabled = true; status.textContent = "Saving this reviewed request…";
      try {
        await gateway!.changeFolder(review, requestId);
        status.textContent = "Folder change saved. It will continue in the background.";
        confirm.remove(); inspect.remove();
        d.append(button("Review folder changes", () => { d.close(); void folderActivity(); }));
        w.changed();
      } catch (error) {
        status.textContent = error instanceof Error ? error.message : "The request could not be saved. Keep this review and retry.";
        confirm.disabled = false;
      }
    });
    confirm.hidden = true;
    const inputs = [source, operation, destination, parent];
    const inspect = button("Review folder change", async () => {
      review = undefined; confirm.hidden = true; inspect.disabled = true;
      for (const input of inputs) input.disabled = true;
      details.replaceChildren(); status.textContent = "Checking the exact folder and its children…";
      const action: FolderAction = operation.value === "Delete" ? "Delete" : operation.value === "Move" ? { Move: { parent: parent.value || null } } : { Rename: { name: destination.value } };
      try {
        const checked = await gateway!.reviewFolder(account, source.value, action);
        if (!d.isConnected) return;
        review = checked;
        details.append(el("p", "", `${checked.plan.members.length} folders and ${checked.messages} cached messages are included.`));
        for (const member of checked.plan.members) details.append(el("p", "", member.destination ? `${member.path} → ${member.destination}` : member.path));
        details.append(el("p", "", action === "Delete" ? "Delete permanently removes these server folders and all messages in them, including mail that is not cached here. This cannot be undone." : "These exact folders and their messages will move together. The original folders remain available until the server confirms the change."));
        status.textContent = "Review the folders before confirming.";
        confirm.hidden = false;
      } catch (error) {
        status.textContent = error instanceof Error ? error.message : "The folder could not be reviewed. Retry after reconnecting.";
        for (const input of inputs) input.disabled = false;
      } finally { inspect.disabled = false; }
    });
    operation.onchange = () => {
      destination.hidden = operation.value !== "Rename";
      parent.hidden = operation.value !== "Move";
    };
    parent.hidden = true; status.role = "status";
    d.append(source, operation, destination, parent, details, status, inspect, confirm);
  }
  async function folderActivity() {
    if (!gateway?.folderActivity) return;
    const d = modal("Folder changes"), content = el("div", "outbox-entries"), status = el("p", "form-status");
    d.classList.add("folder-activity");
    status.role = "status";
    let after: string | undefined, next: string | undefined, completed = false, busy = false, generation = 0;
    const refresh = button("Refresh folder changes", () => { void gateway!.resumeActions(); void draw(); });
    const older = button("Next folder changes", () => { after = next; void draw(); });
    const first = button("First folder changes", () => { after = undefined; void draw(); });
    const recent = button("Recent folder changes", () => { completed = !completed; after = undefined; recent.textContent = completed ? "Pending folder changes" : "Recent folder changes"; void draw(); });
    const controls = el("div", "outbox-actions");
    controls.append(refresh, first, older, recent);
    d.append(content, status, controls);
    async function draw() {
      const request = ++generation;
      try {
        const page = await gateway!.folderActivity!.page(after, completed);
        if (!d.isConnected || generation !== request) return;
        next = page.next; older.disabled = !next; first.disabled = !after;
        content.replaceChildren();
        if (!page.rows.length) content.append(el("p", "empty", "No folder changes in this view."));
        for (const job of page.rows) {
          const card = el("section", "settings-card");
          card.append(el("h3", "", job.target?.name ?? (job.parent ? `${job.parent} / ${job.name}` : job.name)), el("p", "", folderStatus(job)));
          if (job.mutation) {
            const mutation = job.mutation, plan = mutation.review.plan;
            const operation = plan.action === "Delete" ? "Delete" : "Rename" in plan.action ? "Rename" : "Move";
            card.append(el("p", "", `${operation}: ${mutation.completed} of ${folderSteps(plan).length} steps saved. ${mutation.review.messages} reviewed cached messages.`));
          }
          if (job.error) card.append(el("p", "", job.error));
          const decide = async (decision: "retry" | "check" | "dismiss" | "accept" | "repair") => {
            if (busy) return; busy = true;
            for (const button of card.querySelectorAll("button")) button.disabled = true;
            try { await gateway!.decideFolder(job, decision); status.textContent = "Decision saved."; await draw(); w.changed(); }
            catch (error) { status.textContent = error instanceof Error ? error.message : "This decision could not be saved. Refresh and retry."; }
            finally { busy = false; for (const button of card.querySelectorAll("button")) button.disabled = false; }
          };
          if (["Waiting", "Rejected"].includes(job.status) || job.status === "Uncertain" && job.mutation?.checked === "original") card.append(button(`Retry ${job.name}`, () => void decide("retry")));
          if ((job.target || job.mutation) && ["Uncertain", "Repair", "Rejected"].includes(job.status)) card.append(button(`Check ${job.name}`, () => void decide("check")));
          if (job.status === "Repair" && job.mutation?.receipt) card.append(button(`Retry saving ${job.name}`, () => void decide("repair")));
          if (job.status === "Repair" && job.mutation?.checkedCache) card.append(button(`Keep cached mail and stop tracking ${job.name}`, () => {
            const review = modal("Keep cached mail and stop tracking");
            const location = gateway!.accounts.find(account => account.id === job.account)?.protocol === "Pop3" ? "The local folder change remains." : "The server change remains.";
            review.append(el("p", "", `${location} This keeps the current cached messages and the acknowledged receipt in history, without completing the cache repair. The cache may need refreshing. Continue?`), button("Keep cached mail and stop tracking", () => { review.close(); void decide("dismiss"); }));
          }));
          if (job.status === "Uncertain" && job.mutation?.checked === "applied" && job.mutation.review.plan.action === "Delete") card.append(button(`Accept checked deletion of ${job.name}`, () => {
            const review = modal("Accept checked folder deletion");
            review.append(el("p", "", "The server check found this exact folder absent. Remove its unchanged reviewed messages from this browser's cache? This does not send another delete."), button("Accept checked deletion", () => { review.close(); void decide("accept"); }));
          }));
          if (!["Running", "Checking", "Succeeded", "Dismissed", ...(job.mutation ? ["Repair"] : [])].includes(job.status)) card.append(button(`Stop tracking ${job.name}`, () => {
            const review = modal("Stop tracking folder change");
            review.append(el("p", "", "This only dismisses the saved request on this browser. It does not delete a server folder or confirm an unknown result."), button("Stop tracking", () => { review.close(); void decide("dismiss"); }));
          }));
          content.append(card);
        }
      } catch (error) { status.textContent = error instanceof Error ? error.message : "Folder changes could not load. Refresh to retry."; }
    }
    await draw();
  }
  async function actionActivity() {
    if (!gateway?.actionActivity) return;
    const d = modal("Activity"), content = el("div", "outbox-entries"), status = el("p", "form-status");
    status.role = "status";
    let after: string | undefined, next: string | undefined, busy = false, generation = 0, completed = false;
    const refresh = button("Refresh activity", () => void draw());
    const older = button("Next actions", () => { after = next; void draw(); });
    const first = button("First actions", () => { after = undefined; void draw(); });
    const controls = el("div", "outbox-actions");
    const recent = button("Recent changes", () => { completed = !completed; after = undefined; recent.textContent = completed ? "Needs attention" : "Recent changes"; void draw(); });
    controls.append(refresh, first, older, recent);
    if (groupUI) controls.append(button("Group history", () => { d.close(); groupUI.history(); }));
    controls.append(button("Outbox", () => { d.close(); void outbox(); }));
    if (gateway.folderActivity) controls.append(button("Folder changes", () => { d.close(); void folderActivity(); }));
    if (gateway.calendar) controls.append(button("Calendar changes", () => { d.close(); void calendarActivity(); }));
    controls.append(button("Accounts and profile sync", () => { d.close(); tab = "Preferences"; w.changed(); }));
    d.append(content, status, controls);
    async function draw() {
      const request = ++generation;
      status.textContent = "Loading saved actions…";
      try {
        let connectionError = "";
        const [page, connections] = await Promise.all([
          gateway!.actionActivity!.page(after, completed),
          completed ? Promise.resolve({ rows: [], more: false }) : connectionActivity(gateway!).catch(() => {
            connectionError = "Saved connection progress could not load. Refresh Activity to retry.";
            return { rows: [], more: false };
          }),
        ]);
        if (!d.isConnected || request !== generation) return;
        next = page.next;
        content.replaceChildren();
        for (const attempt of connections.rows) {
          const card = el("section", "settings-card");
          card.append(el("h3", "", `Connection: ${attempt.account.email}`),
            el("p", "", connectionStatus(attempt)));
          if (attempt.error) card.append(el("p", "form-status", attempt.error));
          card.append(button(`Reconnect ${attempt.account.email}`, () => {
            d.close(); tab = "Preferences"; w.changed();
          }));
          const dismiss = button(`Dismiss connection for ${attempt.account.email}`, async () => {
            if (busy) return;
            busy = true; dismiss.disabled = true;
            try { await gateway!.dismissConnection(attempt); await draw(); }
            catch (error) { status.textContent = error instanceof Error ? error.message : "Could not dismiss this connection attempt. Refresh Activity."; }
            finally { busy = false; dismiss.disabled = false; void refreshActivitySummary(); }
          });
          card.append(dismiss);
          content.append(card);
        }
        if (connections.more) content.append(el("p", "muted", "More connection attempts are available in Accounts and profile sync."));
        if (connectionError) content.append(el("p", "form-status", connectionError));
        if (!page.rows.length) content.append(el("p", "empty", completed ? "No recent completed changes." : "No individual mail changes need attention."));
        for (const entry of page.rows) {
          const card = el("section", "settings-card");
          const other = entry.lease.fields.accountId && entry.lease.fields.accountId !== entry.account
            ? gateway!.accounts.find(a => a.id === entry.lease.fields.accountId)?.email ?? entry.lease.fields.accountId
            : undefined;
          const operation = entry.lease.fields.folder ? `Move to ${entry.lease.fields.folder}${other ? ` in ${other}` : ""}` : "Update message flags";
          const labels = { Queued: "Saved, not yet confirmed", Waiting: "Waiting for reconnect", Running: "Started, awaiting confirmation", Rejected: "Not applied", Uncertain: "Needs checking", Repair: "Change acknowledged, local progress needs repair", Succeeded: "Change confirmed" };
          card.append(el("h3", "", operation), el("p", "", labels[entry.status]));
          const target = el("p", "", "Loading message details…");
          card.append(target);
          void gateway!.mailbox?.metadata(entry.lease.id).then(result => {
            if (request === generation && target.isConnected)
              target.textContent = result.mail?.subject || "Message unavailable in this cache";
          }).catch(() => { if (target.isConnected) target.textContent = "Message details could not load. Refresh Activity."; });
          if (entry.error) card.append(el("p", "form-status", entry.error));
          card.append(el("p", "muted", `Account: ${entry.account} · Original folder: ${entry.source.folder}`));
          if (entry.status === "Succeeded") {
            const undo = button(entry.undoneBy ? "Undo already saved" : "Undo saved change", async () => {
              if (busy) return;
              busy = true; undo.disabled = true;
              try { await gateway!.undoSavedAction(entry); await w.retryPage(); await draw(); }
              catch (error) { status.textContent = error instanceof Error ? error.message : "Could not undo this change. Refresh Activity."; }
              finally { busy = false; undo.disabled = !!entry.undoneBy; void refreshActivitySummary(); }
            });
            undo.disabled = !!entry.undoneBy || !!(entry.receipt?.recovery && !entry.receipt.after.remoteId);
            card.append(undo);
          } else if (entry.status === "Repair" && entry.receipt) {
            const repair = button("Repair local cache", async () => {
              if (busy) return;
              busy = true; repair.disabled = true;
              try { await gateway!.repairAction(entry.id); await w.retryPage(); await draw(); }
              catch (error) { status.textContent = error instanceof Error ? error.message : "Could not repair the cache. Retry."; }
              finally { busy = false; repair.disabled = false; void refreshActivitySummary(); }
            });
            card.append(repair);
          } else if (entry.status === "Rejected") {
            card.append(button("Dismiss reviewed failure", async () => {
              try { await gateway!.actionActivity!.dismissRejected(entry.id); await draw(); void refreshActivitySummary(); }
              catch (error) { status.textContent = error instanceof Error ? error.message : "Could not dismiss the failure. Retry."; }
            }));
          } else if (entry.status === "Running" || entry.status === "Uncertain") {
            const check = el("input"), label = el("label", "selection-checkbox");
            check.type = "checkbox";
            check.setAttribute("aria-label", "I checked the source and destination folders");
            label.append(check, el("span", "", "I checked the source and destination folders"));
            const accept = button("Accept current state", async () => {
              if (busy || !check.checked) return;
              busy = true; accept.disabled = true;
              try { await gateway!.acceptActionReview(entry, check.checked); await draw(); }
              catch (error) { status.textContent = error instanceof Error ? error.message : "The saved action changed. Refresh its review."; }
              finally { busy = false; accept.disabled = !check.checked; void refreshActivitySummary(); }
            });
            accept.disabled = true;
            check.onchange = () => { accept.disabled = !check.checked || busy; };
            card.append(el("p", "muted", "This retires only the local request. It does not repeat the operation or claim that the server accepted or rejected it."), label, accept);
          } else {
            card.append(el("p", "muted", "Saved changes continue after the original tab closes and this account is connected."));
            card.append(button("Cancel saved change", async () => {
              if (busy) return;
              busy = true;
              try { await gateway!.cancelSavedAction(entry); w.cancelSavedProjection(entry.id); await w.retryPage(); await draw(); }
              catch (error) { status.textContent = error instanceof Error ? error.message : "The action changed. Refresh Activity."; }
              finally { busy = false; void refreshActivitySummary(); }
            }));
            if (entry.status === "Waiting") card.append(button("Open Preferences", () => { d.close(); tab = "Preferences"; w.changed(); }));
          }
          content.append(card);
        }
        older.disabled = !next || busy;
        first.disabled = !after || busy;
        status.textContent = "";
      } catch (error) {
        if (d.isConnected && request === generation) status.textContent = error instanceof Error ? error.message : "Could not load saved actions. Retry.";
      }
    }
    await draw();
  }
  window.addEventListener("pagehide", () => groupUI?.dispose(), { once: true });
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
      ...Object.entries(w.preferences.shortcuts)
        .filter(
          ([action, value]) =>
            action !== "selectAll" &&
            !dialogShortcuts.includes(action as ShortcutKey) &&
            value,
        )
        .map(([, value]) => value),
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
  function go(folder: string) {
    tab = "Mail";
    fullReader = false;
    sidebarOpen = false;
    w.navigate(folder);
  }
  function act(action: Action, id = w.selected) {
    if (!id) return;
    if (action === "move") {
      openMoveChooser({
        group: false,
        accounts: () => w.moveAccounts(),
        home: { source: { kind: "message", account: w.accountOf(id) } },
        fallback: knownFolders(w.moveAccounts()),
        crossAccount: () => w.crossAccountMovesEnabled(),
        foreignEnabled: () => w.foreignMovesEnabled(),
        shortcuts: () => w.preferences.shortcuts,
        move: (folder) => void w.action(id!, action, folder),
        transfer: (account, folder, foreign) =>
          void w.transfer(id!, account, folder, foreign),
      });
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
            queued: "Queued on this browser",
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
          if (entry.queueError) card.append(el("p", "form-status", entry.queueError));
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
          if (entry.state === "queued") {
            actions.append(button("Continue queued send", () => { void gateway!.resumeOutgoing(); status.textContent = "Queued messages will continue when their accounts are connected."; }));
            actions.append(button("Open Preferences", () => { d.close(); tab = "Preferences"; w.changed(); }));
          } else actions.append(button("Check delivery status", () => void recover("check")));
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
  const draftSessions = new Map<string, DraftSession>();
  let signingOut = false;
  window.addEventListener("beforeunload", (event) => {
    if (!w.preferenceError && ![...draftSessions.values()].some((session) => session.pending || session.saving)) return;
    event.preventDefault();
    event.returnValue = "";
  });
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
    let preparedReply = false;
    if (original && gateway && !existing) {
      try {
        existing = await gateway.reply(original.id, all);
        preparedReply = true;
      } catch (error) {
        w.error =
          error instanceof Error
            ? error.message
            : "Could not prepare a reply. Refresh this folder.";
        w.changed();
        return;
      }
    }
    const initial: Draft = existing
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
    const retained = draftSessions.get(initial.id);
    if (retained && !retained.pending && !retained.saving) retained.retire();
    const session = retained && (retained.pending || retained.saving) ? retained : new DraftSession(
      initial,
      !!existing && !preparedReply,
      (snapshot, expected) => w.repository.saveDraft(snapshot, expected),
      () => {
        if (!session.draft.accountId || !gateway?.removedAccounts.has(session.draft.accountId))
          w.rememberDraft(session.draft);
        if (!document.querySelector("dialog.composer[open]")) w.changed();
      },
    );
    draftSessions.set(initial.id, session);
    const draft = session.draft;
    const d = modal(draft.forward ? "Forward message" : "New message");
    d.classList.add("composer");
    const fields = el("div", "composer-fields");
    const status = el("p", "form-status");
    status.setAttribute("role", "status");
    function edited() {
      session.edited();
    }
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
    const attach = button("Attach files", () => fileInput.click(), "clip");
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
        remove.disabled = busy || deliveryLocked || session.filesPending;
        row.append(remove);
        filePanel.append(row);
      }
    }
    async function changeFiles(files: File[], remove?: string) {
      if (!gateway || busy || deliveryLocked || session.filesPending) return;
      const ids = files.map(() => crypto.randomUUID());
      await session.changeFiles(() => remove
        ? gateway.removeFile(draft.id, remove)
        : gateway.addFiles(draft.id, files, ids));
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
    let sendError: string | undefined;
    const retry = button("Retry save", () => void session.flush(true));
    const reviewText = button("Review saved draft", () => void reviewConflict());
    const savedFiles = button("Use saved attachments", () => {
      if (gateway) void session.useSavedFiles(() => gateway.reviewDraftFiles(draft.id));
    });
    const savedStatus = el("span", "draft-save-status");
    d.querySelector(".dialog-heading")!.insertBefore(savedStatus, d.querySelector('[aria-label="Close"]'));
    let shownAttachments = draft.attachments;
    function showSaveStatus() {
      savedStatus.textContent = w.repository.preview ? "Preview" : session.error ? "Not saved" : session.pending || session.saving ? "Saving…" : "Saved";
      status.textContent = session.error ? `${session.status}. ${session.error}` : sendError ?? (w.repository.preview ? "Preview draft" : session.status);
      retry.hidden = !session.error || session.needsReview;
      retry.disabled = session.saving || busy || deliveryLocked;
      reviewText.hidden = !session.needsReview;
      reviewText.disabled = session.saving || busy || deliveryLocked;
      savedFiles.hidden = !session.fileError;
      savedFiles.disabled = session.saving || busy || deliveryLocked;
      attach.disabled = busy || deliveryLocked || session.filesPending;
      if (shownAttachments !== draft.attachments) {
        shownAttachments = draft.attachments;
        renderFiles();
      }
      for (const b of filePanel.querySelectorAll<HTMLButtonElement>("button"))
        b.disabled = busy || deliveryLocked || session.filesPending;
    }
    const unsubscribe = session.subscribe(showSaveStatus);
    d.addEventListener("close", unsubscribe);
    function setControls(disabled: boolean) {
      for (const input of d.querySelectorAll<
        HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement
      >("input,textarea,select"))
        input.disabled = disabled || deliveryLocked;
      for (const b of d.querySelectorAll<HTMLButtonElement>("button"))
        b.disabled = disabled;
      attach.disabled = disabled || deliveryLocked || session.filesPending;
      for (const b of filePanel.querySelectorAll<HTMLButtonElement>("button"))
        b.disabled = disabled || deliveryLocked || session.filesPending;
      retry.disabled = disabled || deliveryLocked || session.saving;
      reviewText.disabled = disabled || deliveryLocked || session.saving;
      savedFiles.disabled = disabled || deliveryLocked || session.saving;
    }
    async function save(send: boolean, retrySave = false) {
      if (busy) return;
      if (!send) {
        void session.flush(retrySave);
        d.close();
        w.changed();
        return;
      }
      if (
        send &&
        (!(draft.to.trim() || draft.cc.trim() || draft.bcc.trim()) ||
          !draft.subject.trim())
      ) {
        status.textContent = "Add a recipient and subject before sending.";
        return;
      }
      busy = true;
      sendError = undefined;
      setControls(true);
      for (const b of actions.querySelectorAll("button")) b.disabled = true;
      for (const input of d.querySelectorAll<
        HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement
      >("input,textarea,select"))
        input.disabled = true;
      try {
        if (!await session.flush()) return;
        if (send) await (deliveryLocked ? w.repository.send(draft) : (w.repository.queueSend?.(draft) ?? w.repository.send(draft)));
        if (send) {
          session.retire();
          draftSessions.delete(draft.id);
          w.drafts.delete(draft.id);
        }
        else w.rememberDraft(draft);
        if (send) {
          if (gateway) w.addCachedMail(gateway.cached);
          w.notice = w.repository.queueSend && !deliveryLocked ? "Message queued in Outbox" : "Message accepted by SMTP";
        }
        d.close();
        w.changed();
      } catch (error) {
        session.recordConflict(error);
        sendError =
          error instanceof Error
            ? error.message
            : send
              ? "Mail was not sent. Your draft is still open; connect a provider before retrying."
              : "Draft could not be saved. Keep this editor open and retry.";
        showSaveStatus();
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
    async function reviewConflict() {
      if (!gateway || session.saving || !session.needsReview) return;
      const review = modal("Review draft changes");
      review.classList.add("draft-conflict");
      const info = el("p", "", "Review both versions before choosing which text to keep. Saved attachments are preserved.");
      const versions = el("div", "draft-versions");
      const result = el("p", "form-status", "Reading saved draft…");
      result.setAttribute("role", "status");
      let current: Draft | undefined;
      let localRevision = draft.revision ?? 0;
      const buttons = el("div", "dialog-actions");
      const mine = button("Save my text", () => void decide(true));
      const saved = button("Use saved text", () => void decide(false));
      const refresh = button("Refresh saved version", () => void load());
      buttons.append(button("Keep editing", () => review.close()), refresh, saved, mine);
      review.append(info, versions, result, buttons);
      function textVersion(label: string, value: Draft) {
        const from = gateway!.accounts.find(account => account.id === value.accountId)?.email ?? "No sending account";
        const view = field(label, `From: ${from}\nTo: ${value.to}\nCc: ${value.cc}\nBcc: ${value.bcc}\nSubject: ${value.subject}\n\n${value.body}`, () => {}, true);
        view.querySelector("textarea")!.readOnly = true;
        return view;
      }
      async function load() {
        mine.disabled = saved.disabled = refresh.disabled = true;
        current = undefined;
        try {
          const latest = await gateway!.reviewDraft(draft.id);
          if (!review.isConnected) return;
          current = latest;
          localRevision = draft.revision ?? 0;
          versions.replaceChildren(textVersion("My text", draft), textVersion("Saved text", latest));
          result.textContent = "Saving your text replaces only the saved text shown here. A newer change will require another review.";
          mine.disabled = saved.disabled = false;
        } catch (error) {
          if (review.isConnected) result.textContent = error instanceof Error ? error.message : "Could not read the saved draft.";
        } finally {
          refresh.disabled = false;
        }
      }
      async function decide(keepMine: boolean) {
        if (!current) return;
        mine.disabled = saved.disabled = refresh.disabled = true;
        try {
          const checked = keepMine ? current : await gateway!.reviewDraft(draft.id, observeDraft(current));
          if (!review.isConnected) return;
          if (!session.acceptReview(checked, localRevision, keepMine))
            throw Error("Your text changed during this review. Refresh the saved version before choosing.");
          review.close();
          d.close();
          void session.flush();
          await composer(undefined, session.draft);
        } catch (error) {
          if (review.isConnected) result.textContent = error instanceof Error ? error.message : "Could not apply this choice. Refresh the saved version.";
        } finally {
          refresh.disabled = false;
        }
      }
      await load();
    }
    async function updateDelivery() {
      if (!gateway || !d.isConnected) return;
      setControls(true);
      let delivery;
      try {
        if (!session.pending && !session.saving) {
          const expected = observeDraft(draft);
          const current = await gateway.readDraft(draft.id);
          if (d.isConnected && session.refreshSaved(current, expected)) {
            for (const [key, label] of [["to", "To"], ["cc", "Cc"], ["bcc", "Bcc"], ["subject", "Subject"], ["body", "Message"]] as const) {
              const input = d.querySelector<HTMLInputElement | HTMLTextAreaElement>(`[aria-label="${label}"]`);
              if (input) input.value = draft[key];
            }
            const from = d.querySelector<HTMLSelectElement>('[aria-label="From account"]');
            if (from) from.value = draft.accountId ?? "";
          }
        }
        if (!session.filesPending) {
          const previous = draft.attachments;
          const files = await gateway.attachments(draft.id);
          if (!session.filesPending && draft.attachments === previous) draft.attachments = files;
        }
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
    const send = button("Send", () => void save(true));
    send.classList.add("primary");
    actions.append(
      send,
      button("Save draft", () => void save(false, true)),
      retry,
      reviewText,
      savedFiles,
    );
    actions.classList.add("composer-actions");
    d.append(fields, status, actions);
    renderFiles();
    showSaveStatus();
    setControls(!!gateway);
    void updateDelivery();
  }
  function editEvent(original?: CalendarEntry, retained?: import("./calendar_actions").ProviderEvent) {
    let baseline = original ? structuredClone(original) : null;
    let revision = 0, persistedRevision = 0, busy = false, requestId = crypto.randomUUID();
    let saveAttempt: { entry: CalendarEntry; before: CalendarEntry | null; revision: number; id: string } | undefined;
    let deleteAttempt: { entry: CalendarEntry; id: string } | undefined;
    const chosen = gateway?.calendar?.sources.find(source => source.id === (retained?.source_id ?? gateway.calendar?.source));
    const entry: CalendarEntry = original
      ? structuredClone(original)
      : {
          id: crypto.randomUUID(),
          localKey: crypto.randomUUID(),
          title: "",
          start: new Date(Date.UTC(month.getFullYear(), month.getMonth(), 1)).toISOString(),
          end: new Date(Date.UTC(month.getFullYear(), month.getMonth(), 2)).toISOString(),
          calendar: chosen?.name ?? "Personal",
          location: "",
          readOnly: false,
        };
    if (!original && chosen) entry.provider = { id: entry.id, source_id: chosen.id, title: entry.title, start: entry.start, end: entry.end, location: entry.location, description: "", all_day: true, etag: null, remote_url: null };
    if (retained && entry.provider) {
      entry.title = retained.title; entry.location = retained.location; entry.start = retained.start; entry.end = retained.end;
      entry.provider = { ...retained, id: entry.provider.id, source_id: entry.provider.source_id, etag: entry.provider.etag, remote_url: entry.provider.remote_url };
    }
    if (!original && gateway) entry.readOnly = !chosen || chosen.read_only;
    const startDate = new Date(entry.start);
    const day = entry.provider?.all_day
      ? new Date(startDate.getUTCFullYear(), startDate.getUTCMonth(), startDate.getUTCDate())
      : startDate;
    const d = modal(
      entry.readOnly ? "View event" : original ? "Edit event" : "New event",
    );
    const requestClose = () => {
      if (busy) return;
      if (revision === persistedRevision) { d.close(); return; }
      const review = modal("Discard unsaved event edits?");
      review.append(el("p", "", "These edits have not been saved on this browser. Keep the editor open to retry, or discard them explicitly."), button("Keep editing", () => review.close()), button("Discard event edits", () => { review.close(); d.close(); }));
    };
    d.addEventListener("cancel", event => { event.preventDefault(); requestClose(); });
    d.querySelector<HTMLButtonElement>(".dialog-heading button")!.onclick = requestClose;
    const status = el("p", "form-status");
    status.setAttribute("role", "status");
    d.append(
      el(
        "p",
        "muted",
        `${day.toLocaleDateString(undefined, { dateStyle: "long" })} · ${entry.provider?.all_day === false ? `${new Date(entry.start).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })} to ${new Date(entry.end).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}` : "All day"}`,
      ),
      field("Event title", entry.title, (v) => { entry.title = v; revision++; }),
      field("Location", entry.location, (v) => { entry.location = v; revision++; }),
    );
    if (entry.provider) d.append(field("Description", entry.provider.description, value => { entry.provider!.description = value; revision++; }, true));
    if (entry.provider?.all_day) {
      const start = field("Starts on", entry.start.slice(0, 10), value => { entry.start = `${value}T00:00:00.000Z`; revision++; });
      const end = field("Ends on", new Date(Date.parse(entry.end) - 86400000).toISOString().slice(0, 10), value => { const stamp = Date.parse(`${value}T00:00:00.000Z`); entry.end = Number.isFinite(stamp) ? new Date(stamp + 86400000).toISOString() : ""; revision++; });
      start.querySelector("input")!.type = "date"; end.querySelector("input")!.type = "date"; d.append(start, end);
    }
    if (entry.readOnly) {
      for (const input of d.querySelectorAll("input")) input.readOnly = true;
      for (const input of d.querySelectorAll("textarea")) input.readOnly = true;
      d.append(el("p", "muted", chosen ? "This calendar is read-only." : "Refresh Calendar and choose a writable calendar before creating an event."));
    } else
      d.append(
        button("Save event", async () => {
          if (busy) return;
          if (!saveAttempt && !entry.title.trim()) {
            status.textContent = "Enter an event title.";
            return;
          }
          busy = true;
          if (!saveAttempt) {
            const captured = structuredClone(entry);
            if (captured.provider) captured.provider = { ...captured.provider, title: captured.title, location: captured.location, start: captured.start, end: captured.end };
            saveAttempt = { entry: captured, before: structuredClone(baseline), revision, id: requestId };
          }
          const attempt = saveAttempt, savedRevision = attempt.revision, captured = attempt.entry;
          status.textContent = "Saving on this browser…";
          try {
            const admitted = await w.repository.saveEvent(captured, attempt.before, attempt.id);
            if (admitted && ["Rejected", "Cancelled", "Dismissed"].includes(admitted.status)) {
              saveAttempt = undefined; requestId = crypto.randomUUID();
              status.textContent = `${calendarStatus(admitted)}. Your entered edits remain here. Review Calendar changes before saving again.`;
              return;
            }
            if (admitted?.status === "Succeeded" && admitted.receipt?.after) {
              const current = admitted.receipt.after;
              captured.id = current.id; captured.provider = structuredClone(current);
              captured.title = current.title; captured.location = current.location; captured.start = current.start; captured.end = current.end;
              if (entry.provider) {
                entry.id = current.id;
                entry.provider = { ...entry.provider, id: current.id, source_id: current.source_id, etag: current.etag, remote_url: current.remote_url };
              }
            }
            baseline = captured; persistedRevision = savedRevision; requestId = crypto.randomUUID();
            saveAttempt = undefined;
            w.events = gateway ? structuredClone(gateway.events) : [...w.events.filter((e) => e.id !== captured.id), captured];
            if (revision === savedRevision) d.close();
            else status.textContent = "Earlier edits saved. Your newer edits are still here; save them when ready.";
            w.changed();
          } catch (error) { status.textContent = error instanceof Error ? error.message : "Event could not be saved. Keep the form open and retry."; }
          finally { busy = false; }
        }),
      );
    if (original && !entry.readOnly && gateway) d.append(button("Delete event", () => {
      if (saveAttempt) { status.textContent = "Retry Save event to confirm the earlier saved request before deleting this event. Your newer edits will stay here."; return; }
      if (!baseline) return;
      const review = modal("Delete event?");
      review.append(el("p", "", `Delete “${original.title}” from ${original.calendar}? This change will sync in the background.`), button("Delete this event", async () => {
        if (busy) return; busy = true;
        deleteAttempt ??= { entry: structuredClone(baseline!), id: crypto.randomUUID() };
        const attempt = deleteAttempt;
        try {
          const admitted = await gateway.deleteEvent(attempt.entry, attempt.id);
          if (["Rejected", "Cancelled", "Dismissed"].includes(admitted.status)) {
            deleteAttempt = undefined;
            status.textContent = `${calendarStatus(admitted)}. The event remains open. Review Calendar changes before deleting again.`;
            review.close();
            return;
          }
          review.close(); d.close(); w.events = structuredClone(gateway.events); w.changed();
        }
        catch (error) { status.textContent = error instanceof Error ? error.message : "The delete could not be saved. Retry."; review.close(); }
        finally { busy = false; }
      }));
    }));
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
          if (name === "Calendar") void showCalendar();
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
      ["Drafts", "file"],
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
      if (account && gateway?.folderActivity) details.append(button(`Create folder for ${name}`, () => newFolder(account.id)));
      if (account && gateway?.folderActivity) details.append(button(`Manage folders for ${name}`, () => manageFolders(account.id)));
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
        sidebarOpen = false;
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
          async () => {
            if (signingOut) return;
            signingOut = true;
            try {
              if (w.preferenceError) {
                tab = "Preferences";
                sidebarOpen = false;
                w.changed();
                return;
              }
              const saved = await Promise.all([...draftSessions.values()]
                .filter(session => session.pending || session.saving)
                .map(session => session.flush(true)));
              if (saved.some(result => !result)) {
                w.error = "Some drafts are not saved. Open Drafts and retry before signing out.";
                tab = "Mail";
                go("Drafts");
                return;
              }
              await w.finishReading();
              find.dispose();
              searchWorker.dispose();
              printer?.dispose();
              login.signOut();
            } finally {
              signingOut = false;
            }
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
      for (const d of w.drafts.values()) {
        const row = el("div", "draft-row");
        const open = button(
            d.subject || "Untitled draft",
            () => composer(undefined, d),
            "edit",
          );
        open.dataset.stable = `draft:${d.id}`;
        row.append(open);
        const session = draftSessions.get(d.id);
        if (session) row.append(el("span", "draft-save-status", session.status));
        box.append(row);
      }
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
    const search = field(
      "Search conversations",
      searchInput ?? w.query,
      (v) => {
        searchInput = v;
        if (w.selection.mode) w.selection.done();
        clearTimeout(searchTimer);
        searchTimer = setTimeout(() => {
          searchInput = undefined;
          w.search(v);
        }, 100);
      },
    );
    search
      .querySelector("input")
      ?.addEventListener("focus", () => void w.finishReading());
    search.classList.add("search-field");
    const searchRow = el("div", "selection-search");
    const selectionToggle = button(w.selection.mode ? "Done" : "Select", () => {
      if (w.selection.mode) w.selection.done();
      else {
        if (searchInput !== undefined) {
          clearTimeout(searchTimer);
          const query = searchInput;
          searchInput = undefined;
          w.search(query);
        }
        w.selection.start();
      }
      root.querySelector<HTMLElement>(".rows")?.focus();
    });
    selectionToggle.dataset.focus = "selection-toggle";
    searchRow.append(search, selectionToggle);
    controls.append(searchRow);
    list.append(controls);
    if (w.selection.mode) {
      const choices = el("div", "selection-toolbar");
      const status = el(
        "span",
        "selection-count",
        `${w.selection.count} selected${!w.selection.error && (w.selection.pending || w.selection.observing) ? " · Updating…" : ""}`,
      );
      status.setAttribute("role", "status");
      status.setAttribute("aria-label", "Selection status");
      status.dataset.pending = String(
        !w.selection.error && (w.selection.pending || w.selection.observing),
      );
      const all = button("Select all messages", () => w.selection.all());
      all.dataset.focus = "selection-all";
      all.querySelector("span")!.textContent = "Select all";
      const clear = button("Clear selection", () => w.selection.clear());
      clear.dataset.focus = "selection-clear";
      clear.querySelector("span")!.textContent = "Clear";
      choices.append(status, all, clear);
      list.append(choices);
    }
    if (w.selection.error || w.selection.warning) {
      const failure = el(
        "div",
        "selection-error",
        w.selection.error ?? w.selection.warning,
      );
      failure.setAttribute("role", "alert");
      failure.setAttribute("aria-label", "Selection error");
      if (w.selection.error)
        failure.append(button("Retry selection", () => w.selection.retry()));
      list.append(failure);
    }
    const rows = el("div", "rows");
    rows.tabIndex = 0;
    rows.dataset.focus = "mail-list";
    rows.dataset.scroll = "mail-list";
    rows.dataset.scope = JSON.stringify([
      w.folder,
      w.account,
      w.query,
      w.filter,
      w.newestFirst,
      w.page,
      w.selection.mode,
    ]);
    rows.setAttribute("aria-label", "Emails");
    for (const [index, m] of w.visible.entries()) {
      const row = el(
        "article",
        `mail-row${!w.selection.mode && w.selected === m.id ? " selected" : ""}${w.selection.selected(m.id) ? " bulk-selected" : ""}${m.unread ? " unread" : ""}`,
      );
      row.dataset.id = m.id;
      const main = button(m.subject, () => open(m));
      main.className = "row-open";
      main.dataset.focus = `mail-row:${m.id}`;
      main.onclick = (event) => {
        if (event.shiftKey)
          w.selection.range(m.id, event.ctrlKey || event.metaKey);
        else if (w.selection.mode || event.ctrlKey || event.metaKey)
          w.selection.toggle(m.id);
        else open(m);
      };
      main.ondblclick = (event) => {
        if (
          w.selection.mode ||
          event.ctrlKey ||
          event.metaKey ||
          event.shiftKey
        )
          return;
        w.beginReading(m.id);
        fullReader = true;
        w.changed();
      };
      const top = el("div", "row-top");
      top.append(el("span", m.unread ? "unread-dot" : "unread-dot read"));
      if (w.preferences.avatars && !w.selection.mode)
        top.append(avatar(m.sender, index));
      top.append(el("span", "sender", m.sender));
      if (m.attachments.length) top.append(icon("clip"));
      top.append(el("time", "", rowDate(new Date(m.date))));
      main.replaceChildren(top, el("span", "subject", m.subject));
      if (w.preferences.previewLines) {
        const preview = el("span", "preview", m.preview);
        preview.style.setProperty("--lines", `${w.preferences.previewLines}`);
        main.append(preview);
      }
      main.append(el("span", "row-meta", m.account));
      const flag = button(
        `${m.starred ? "Unflag" : "Flag"} ${m.subject}`,
        () => void w.action(m.id, "star"),
        "flag",
        true,
      );
      if (m.starred) flag.classList.add("flagged");
      flag.dataset.focus = `flag-row:${m.id}`;
      if (w.selection.mode) {
        const label = el("label", "selection-checkbox");
        const checkbox = el("input");
        checkbox.type = "checkbox";
        checkbox.checked = w.selection.selected(m.id);
        checkbox.setAttribute("aria-label", `Select ${m.subject}`);
        checkbox.dataset.focus = `select-row:${m.id}`;
        checkbox.onclick = (event) => {
          event.stopPropagation();
          if (event.shiftKey)
            w.selection.range(m.id, event.ctrlKey || event.metaKey);
          else w.selection.toggle(m.id);
        };
        label.append(checkbox);
        row.append(label);
      }
      row.append(main, flag);
      rows.append(row);
    }
    if (w.pageError) {
      const error = el("div", "mail-error");
      error.setAttribute("role", "alert");
      error.append(
        el("p", "", w.pageError),
        button("Retry page", () => void w.retryPage()),
      );
      rows.append(error);
    }
    if (w.pageLoading) {
      const loading = el("p", "muted", "Loading cached mail…");
      loading.setAttribute("role", "status");
      rows.append(loading);
    }
    if (!w.visible.length && !w.pageLoading && !w.pageError) {
      const empty = el("div", "empty");
      empty.append(
        icon("search"),
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
    next.disabled = (w.page + 1) * 50 >= w.total;
    paging.append(
      prev,
      el(
        "span",
        "",
        `${w.total ? w.page * 50 + 1 : 0}–${Math.min((w.page + 1) * 50, w.total)} of ${w.total}`,
      ),
      next,
    );
    list.append(paging);
    rows.onkeydown = (e) => {
      if (!["ArrowDown", "ArrowUp", "Enter"].includes(e.key)) return;
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      if (
        e.key === "Enter" &&
        ((e.target as Element).closest("button:not(.row-open)") ||
          (!w.selection.mode &&
            (e.shiftKey || w.preferences.shortcuts.reader !== "Enter")))
      )
        return;
      e.preventDefault();
      const focused = (
        document.activeElement as HTMLElement
      )?.closest<HTMLElement>(".mail-row")?.dataset.id;
      const index = w.visible.findIndex(
        (m) => m.id === (w.selection.mode ? focused : w.selected),
      );
      if (w.selection.mode && e.key === "Enter") {
        if (focused) w.selection.toggle(focused);
        return;
      }
      if (e.key === "Enter") {
        const id = focused ?? w.selected;
        if (id) {
          fullReader = true;
          w.beginReading(id);
        }
        return;
      }
      const m =
        w.visible[
          Math.min(
            w.visible.length - 1,
            Math.max(0, index + (e.key === "ArrowDown" ? 1 : -1)),
          )
        ];
      if (m && w.selection.mode) {
        if (e.shiftKey) w.selection.range(m.id);
        const target = [
          ...root.querySelectorAll<HTMLElement>(".mail-row"),
        ].find((row) => row.dataset.id === m.id);
        target
          ?.querySelector<HTMLInputElement>('input[type="checkbox"]')
          ?.focus();
        target?.scrollIntoView({ block: "nearest" });
        return;
      }
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
      w.selection.mode ? selectionSummary() : reader(),
    );
    w.selection.setObserved(w.visible.map((m) => m.id));
    return box;
  }
  function selectionSummary() {
    const panel = el("section", "reader selection-summary");
    panel.setAttribute("aria-label", "Selected messages");
    panel.append(
      el(
        "h2",
        "",
        `${w.selection.count} ${w.selection.count === 1 ? "message" : "messages"} selected`,
      ),
    );
    if (!w.selection.count)
      panel.append(
        el(
          "p",
          "muted",
          "Choose messages using their checkboxes, or select all messages in this view.",
        ),
      );
    else
      panel.append(
        el("p", "muted", "Your selection is kept when you change pages."),
      );
    if (w.selection.snapshot && !w.selection.pending) {
      const snapshot = w.selection.snapshot;
      if (snapshot.available !== snapshot.selected)
        panel.append(
          el(
            "p",
            "",
            `${snapshot.available} of ${snapshot.selected} selected messages are available on this device.`,
          ),
        );
      const groups = el("ul", "selection-groups");
      for (const g of snapshot.groups) {
        const account =
          gateway?.accounts.find((a) => a.id === g.account)?.email ?? g.account;
        groups.append(
          el(
            "li",
            "",
            `${g.total} in ${g.folder === "INBOX" ? "Inbox" : g.folder} · ${account}`,
          ),
        );
      }
      panel.append(groups);
    }
    if (groupUI) panel.append(groupUI.toolbar());
    return panel;
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
      for (const name of m.attachments) {
        const chip = el("span", "attachment");
        chip.append(icon("clip"), el("span", "", name));
        panel.append(chip);
      }
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
      [
        "read",
        m?.unread ? "Mark read" : "Mark unread",
        m?.unread ? "mail-open" : "mail",
      ],
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
    sender.append(avatar(m.sender, 0));
    const details = el("div");
    details.append(
      el("strong", "", m.sender),
      el("p", "", m.address),
      el("p", "", m.account),
    );
    const when = new Date(m.date);
    const time = el("time", "");
    time.dateTime = when.toISOString();
    time.append(
      el("span", "", readerDate(when)),
      el("br"),
      el("span", "", clockTime(when)),
    );
    sender.append(details, time);
    content.append(sender);
    if (w.bodyLoading && !m.bodyLoaded) {
      const loading = el("p", "muted", "Loading message…");
      loading.setAttribute("role", "status");
      content.append(loading);
    }
    if (w.bodyError) {
      const error = el("div", "mail-error");
      error.setAttribute("role", "alert");
      error.append(
        el("p", "", w.bodyError),
        button("Retry message", () => w.retryBody()),
      );
      content.append(error);
    }
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
    actions.setAttribute("role", "group");
    actions.setAttribute("aria-label", "Message actions");
    const reply = button("Reply", () => composer(m), "reply");
    reply.classList.add("primary");
    const forwardBusy = forwarding.has(m.id),
      printBusy = printer?.preparing(m.id) ?? false;
    const forwardButton = button(
      "Forward",
      () => void forward(m),
      forwardBusy ? "refresh" : "forward",
    );
    const printButton = button(
      "Print",
      () => printMessage(m),
      printBusy ? "refresh" : "print",
    );
    for (const [control, busy, label] of [
      [forwardButton, forwardBusy, "Preparing forward…"],
      [printButton, printBusy, "Preparing print…"],
    ] as const) {
      control.disabled = busy;
      control.setAttribute("aria-busy", String(busy));
      if (busy) {
        control.setAttribute("aria-label", label);
        control.title = label;
      }
    }
    actions.append(
      reply,
      button("Reply all", () => void composer(m, undefined, true), "reply-all"),
      forwardButton,
      printButton,
    );
    actions.dataset.message = m.id;
    for (const [index, child] of [...actions.children].entries())
      (child as HTMLElement).dataset.focus = `reader-action:${m.id}:${index}`;
    panel.append(content, actions);
    return panel;
  }
  function preferences() {
    const panel = el("section", "settings-panel");
    panel.setAttribute("aria-label", "Preferences");
    const feedback = el("section", "settings-card preference-feedback");
    feedback.setAttribute("aria-label", "Preference saving");
    const status = el("p", "", w.preferenceError ?? (w.preferenceSaved ? "Preferences saved on this browser" : "Changes are saved on this browser"));
    status.setAttribute("role", "status");
    feedback.append(status);
    if (w.preferenceError) {
      const retry = button("Retry preference save", () => w.retryPreferences());
      retry.dataset.stable = "preference-save-retry";
      feedback.append(retry);
    }
    panel.append(feedback);
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
      checkbox("Allow moving mail between accounts", p.crossAccountMoves, (v) =>
        w.savePreferences({ ...w.preferences, crossAccountMoves: v }),
      ),
      checkbox(
        "Search other accounts' folders when moving",
        p.foreignMoveFolders,
        (v) => w.savePreferences({ ...w.preferences, foreignMoveFolders: v }),
        !p.crossAccountMoves,
      ),
      el(
        "p",
        "muted",
        "Matches in other IMAP accounts show the account and ask before moving.",
      ),
    );
    const shortcuts = el("div", "shortcut-list");
    for (const key of Object.keys(
      p.shortcuts,
    ) as (keyof Preferences["shortcuts"])[]) {
      const value = p.shortcuts[key];
      const row = el("div", "shortcut");
      row.append(el("span", "", shortcutLabel(key)));
      const capturing = shortcutCapture?.key === key;
      const capture = button(
        capturing
          ? shortcutCapture?.conflict
            ? "Already assigned"
            : "Press a key…"
          : value || "Disabled",
        () => {
          shortcutCapture = { key, conflict: false };
          render();
        },
      );
      // Both the button and its ancestors survive provider/query redraws.
      // The capture itself belongs to the mounted UI, not a disposable node.
      capture.dataset.stable = `shortcut-capture:${key}`;
      capture.setAttribute("aria-label", `Remap ${shortcutName(key)}`);
      capture.onkeydown = (e) => {
        if (shortcutCapture?.key !== key) return;
        e.preventDefault();
        e.stopPropagation();
        if (e.key === "Escape") {
          shortcutCapture = undefined;
          render();
          return;
        }
        if (["Control", "Meta", "Shift", "Alt"].includes(e.key)) return;
        const combo = keyCombo(e);
        const latest = w.preferences;
        if (
          Object.entries(latest.shortcuts).some(
            ([k, v]) => k !== key && v === combo,
          )
        ) {
          shortcutCapture = { key, conflict: true };
          render();
          return;
        }
        shortcutCapture = undefined;
        w.savePreferences({
          ...latest,
          shortcuts: { ...latest.shortcuts, [key]: combo },
        });
      };
      capture.onblur = (e) => {
        if (shortcutCapture?.key !== key) return;
        shortcutCapture = undefined;
        // Do not redraw in the middle of native focus transfer. The retained
        // listener must address its real target, not the discarded fresh node.
        (e.currentTarget as HTMLElement).querySelector("span")!.textContent =
          w.preferences.shortcuts[key] || "Disabled";
      };
      const clear = button(
        `Clear ${shortcutName(key)}`,
        () => {
          shortcutCapture = undefined;
          const latest = w.preferences;
          w.savePreferences({
            ...latest,
            shortcuts: { ...latest.shortcuts, [key]: "" },
          });
        },
        "close",
        true,
      );
      clear.dataset.stable = `shortcut-clear:${key}`;
      row.append(capture, clear);
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
        accountPanel(gateway, (removed, notice) => {
          if (removed) {
            w.accountRemoved(removed);
            gateway.groups.refreshAttention();
            attachmentState = undefined;
          }
          w.notice = notice ?? (removed
            ? "Account removed from this browser"
            : "Account preferences saved");
          w.error = removed ? gateway.warning : null;
          w.changed();
        }),
      );
    if (profiles) panel.append(...profiles.section());
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
        profiles
          ? "Google beta sign-in grants access to Shep. Calendar and Drive permissions are requested separately under Profiles and sync."
          : "Google beta sign-in grants access to Shep. Calendar and Drive authorization will be connected separately.",
      ),
    );
    return panel;
  }
  let calendarLoading = false;
  const calendarWindow = (): [string, string] => [new Date(month.getFullYear(), month.getMonth(), 1).toISOString(), new Date(month.getFullYear(), month.getMonth() + 1, 1).toISOString()];
  async function showCalendar(refresh = false) {
    if (!gateway?.calendar || calendarLoading && refresh) return;
    if (refresh) { calendarLoading = true; w.changed(); }
    try {
      if (refresh) await gateway.calendar.refresh(...calendarWindow());
      else await gateway.calendar.show(...calendarWindow());
      w.events = structuredClone(gateway.calendar.events);
    } catch (error) { gateway.calendar.error = error instanceof Error ? error.message : "Calendar could not load."; }
    finally { if (refresh) calendarLoading = false; w.changed(); }
  }
  async function calendarActivity() {
    if (!gateway?.calendar) return;
    const calendar = gateway.calendar, d = modal("Calendar changes"), content = el("div", "outbox-entries"), status = el("p", "form-status");
    d.classList.add("calendar-activity");
    status.role = "status";
    let after: string | undefined, next: string | undefined, completed = false, generation = 0, busy = false;
    const undoIds = new Map<string, string>();
    const older = button("Next Calendar changes", () => { after = next; void draw(); });
    const recent = button("Recent Calendar changes", () => { completed = !completed; after = undefined; recent.textContent = completed ? "Pending Calendar changes" : "Recent Calendar changes"; void draw(); });
    const controls = el("div", "outbox-actions");
    controls.append(button("Refresh Calendar changes", () => { void gateway!.resumeActions(); void draw(); }), recent, older);
    d.append(content, status, controls);
    async function draw() {
      const observed = ++generation;
      try {
        const page = await calendar.journal.page(after, completed);
        if (!d.isConnected || observed !== generation) return;
        next = page.next; older.disabled = !next; content.replaceChildren();
        if (!page.rows.length) content.append(el("p", "empty", "No Calendar changes in this view."));
        for (const job of page.rows) {
          const entry = calendarAfter(job.requested) ?? calendarBefore(job.requested)!;
          const card = el("section", "settings-card");
          card.append(el("h3", "", entry.title || "Untitled event"), el("p", "", calendarStatus(job)));
          if (job.error) card.append(el("p", "", job.error));
          const decide = async (decision: "retry" | "repair" | "check" | "adopt" | "cancel" | "undo") => {
            if (busy) return; busy = true;
            for (const control of card.querySelectorAll("button")) control.disabled = true;
            try {
              if (decision === "undo") {
                if (!undoIds.has(job.id)) undoIds.set(job.id, crypto.randomUUID());
                await calendar.undo(job, undoIds.get(job.id)!);
              } else await calendar.decide(job, decision);
              status.textContent = decision === "undo" ? "Undo saved. Syncing in the background." : "Decision saved.";
              await draw();
            }
            catch (error) { calendar.error = status.textContent = error instanceof Error ? error.message : "This Calendar decision could not be saved. Keep the change and retry."; }
            finally { busy = false; for (const control of card.querySelectorAll("button")) control.disabled = false; void gateway!.resumeActions(); w.events = structuredClone(calendar.events); w.changed(); }
          };
          if (["Waiting", "Rejected"].includes(job.status)) card.append(button(`Retry ${entry.title}`, () => void decide("retry")));
          if (["Queued", "Waiting"].includes(job.status)) card.append(button(`Cancel ${entry.title}`, () => void decide("cancel")));
          if (["Uncertain", "Repair", "Rejected"].includes(job.status)) card.append(button(`Check ${entry.title}`, () => void decide("check")));
          if (job.status === "Repair") card.append(button(`Finish saving ${entry.title}`, () => void decide("repair")));
          if (job.undoAction) card.append(el("p", "muted", "Undo is saved as a separate Calendar change."));
          else if (job.status === "Succeeded" && job.receipt?.after) card.append(button(`Undo ${entry.title}`, () => void decide("undo")));
          if (job.observation) card.append(button(`Review checked state for ${entry.title}`, () => {
            const review = modal("Use checked Calendar state?");
            review.append(el("p", "", job.observation!.current ? `The checked event is “${job.observation!.current.title}”.` : "The exact event is absent from this calendar."),
              el("p", "", "This replaces the local projection with the checked server state. It does not repeat a save or delete, or confirm an unknown provider result. Saved newer edits remain available in Calendar changes."),
              button("Use checked server state", () => { review.close(); void decide("adopt"); }));
          }));
          if (["Rejected", "Uncertain", "Repair", "Dismissed"].includes(job.status)) card.append(button(`Review saved edits for ${entry.title}`, () => {
            const review = modal("Saved Calendar edits");
            review.append(el("p", "", entry.title), el("p", "", entry.location), el("p", "", entry.description), el("p", "muted", "These are the retained requested edits. Check the event before applying them to its current version."));
            if (calendarAfter(job.requested) && ["Rejected", "Dismissed"].includes(job.status)) review.append(button("Edit retained version", async () => {
              const current = await calendar.journal.currentEvent(job.key);
              if (!current) {
                review.append(el("p", "form-status", "The original event is absent. Creating a new event will use a new saved request."), button("Create a new event from these edits", () => { review.close(); d.close(); editEvent(undefined, calendarAfter(job.requested)!); }));
                return;
              }
              const source = calendar.sources.find(source => source.id === current.event.source_id);
              review.close(); d.close();
              editEvent({ id: current.event.id, localKey: current.key, provider: current.event, title: current.event.title, location: current.event.location, start: current.event.start, end: current.event.end, calendar: source?.name ?? "Calendar", readOnly: source?.read_only ?? true }, calendarAfter(job.requested)!);
            }));
          }));
          content.append(card);
        }
      } catch (error) { status.textContent = error instanceof Error ? error.message : "Calendar changes could not load. Retry Refresh."; }
    }
    await draw();
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
          void showCalendar();
          w.changed();
        },
        "back",
        true,
      ),
      button(
        "Next month",
        () => {
          month = new Date(month.getFullYear(), month.getMonth() + 1, 1);
          void showCalendar();
          w.changed();
        },
        "chevron",
        true,
      ),
      button("New event", () => editEvent(), "calendar"),
    );
    if (gateway?.calendar) {
      const refresh = button("Refresh Calendar", () => void showCalendar(true));
      refresh.disabled = calendarLoading;
      heading.append(refresh, button("Calendar changes", () => void calendarActivity()), button("Choose calendar", () => {
        const choices = modal("Choose calendar");
        for (const source of gateway.calendar!.sources) choices.append(button(source.name + (source.read_only ? " (read-only)" : ""), () => {
          gateway.calendar!.source = source.id; choices.close(); void showCalendar(true);
        }));
        if (!gateway.calendar!.sources.length) choices.append(el("p", "", "Refresh Calendar to load the calendars granted in Preferences."));
      }));
      for (const control of heading.querySelectorAll("button")) control.dataset.stable = `calendar-heading:${control.getAttribute("aria-label") ?? control.textContent}`;
    }
    panel.append(heading);
    if (gateway?.calendar) {
      const status = el("p", "calendar-owned-status", gateway.calendar.error ? "Calendar could not refresh. Retry or review Calendar changes." : gateway.calendar.attention ? `${gateway.calendar.attention} Calendar change${gateway.calendar.attention === 1 ? " needs" : "s need"} attention.` : gateway.calendar.pending ? `${gateway.calendar.pending} Calendar change${gateway.calendar.pending === 1 ? " is" : "s are"} saved and syncing.` : "");
      status.role = "status"; panel.append(status);
    }
    if (gateway?.calendar && !gateway.calendar.sources.length) panel.append(el("p", "empty", "Connect Google Calendar in Preferences, then choose Refresh Calendar."));
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
          const allDay = e.provider?.all_day;
          return (
            (allDay ? d.getUTCFullYear() : d.getFullYear()) === month.getFullYear() &&
            (allDay ? d.getUTCMonth() : d.getMonth()) === month.getMonth() &&
            (allDay ? d.getUTCDate() : d.getDate()) === day
          );
        }))
          { const control = button(event.title, () => editEvent(event)); control.dataset.stable = `calendar-event:${event.localKey ?? event.id}`; cell.append(control); }
      }
      grid.append(cell);
    }
    panel.append(grid);
    if (gateway?.calendar?.error) { const status = el("p", "form-status", gateway.calendar.error); status.role = "status"; panel.append(status); }
    return panel;
  }
  function render() {
    for (const [id, session] of draftSessions) {
      if (session.draft.accountId && gateway?.removedAccounts.has(session.draft.accountId)) {
        session.retire();
        draftSessions.delete(id);
      } else if (session.pending || session.saving) {
        w.rememberDraft(session.draft);
      }
    }
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
    const next = el("div");
    next.append(sidebar(), sizing);
    const main = el("main", tab === "Preferences" ? "preferences-main" : "");
    const notices = tab === "Preferences" ? el("div", "preferences-notices") : main;
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
    if (tab === "Preferences")
      header.append(el("span", "muted", "Make Shep feel like home."));
    if (tab === "Mail")
      header.append(
        el(
          "span",
          "muted",
          w.folder === "Drafts"
            ? `${w.drafts.size} ${w.drafts.size === 1 ? "draft" : "drafts"}`
            : `${w.total} messages · ${w.unread} unread`,
        ),
      );
    header.append(el("span", "spacer"));
    if (gateway?.actionActivity) {
      const activity = button("Activity", () => void actionActivity());
      activity.querySelector("span")!.textContent = `Activity${activitySummary ? ` (${activitySummary})` : ""}`;
      activity.dataset.stable = "action-activity";
      header.append(activity);
    }
    if (groupUI) {
      const history = button("Group history", () => groupUI.history());
      history.querySelector("span")!.textContent = "History";
      history.dataset.stable = "group-history";
      header.append(history);
    }
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
    const groupError = groupUI?.errorBanner();
    if (groupError) main.append(groupError);
    const groupRecovery = groupUI?.recoveryBanner();
    if (groupRecovery) main.append(groupRecovery);
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
      (tab === "Preferences" ? notices : main).append(error);
    }
    const offer = tab === "Mail" ? profiles?.banner() : null;
    if (offer) main.append(offer);
    main.append(
      tab === "Mail"
        ? inbox()
        : tab === "Calendar"
          ? calendar()
          : preferences(),
    );
    if (tab !== "Mail" || w.folder === "Drafts") w.selection.setObserved([]);
    if (notices !== main) main.append(notices);
    if (w.notice) {
      const status = el("div", "status");
      status.setAttribute("role", "status");
      status.setAttribute("aria-label", "Mail status");
      status.append(icon("check"), el("span", "", w.notice));
      if (!w.moves.visible && w.undo) status.append(button("Undo", w.undo));
      notices.append(status);
    }
    if (w.moves.label) {
      const status = el("div", "status");
      status.setAttribute("role", "status");
      status.setAttribute("aria-label", "Move notification");
      status.append(icon("check"), el("span", "", w.moves.label));
      if (w.undo) status.append(button("Undo", w.undo));
      status.append(
        button(
          "Dismiss move notification",
          () => w.moves.dismiss(),
          "close",
          true,
        ),
      );
      notices.append(status);
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
      notices.append(failure);
    }
    const groupNotice = groupUI?.notification();
    if (groupNotice) main.append(groupNotice);
    next.append(main);
    renderReaderTree(root, next);
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
    if (document.querySelector("dialog[open]") || keyConsumed(e)) return;
    if (handleShortcut(keyCombo(e))) e.preventDefault();
  });
  function handleShortcut(combo: string) {
    if (tab !== "Mail" || document.querySelector("dialog[open]")) return false;
    if (combo === "Escape" && w.selection.mode) {
      w.selection.done();
      root.querySelector<HTMLElement>(".rows")?.focus();
      return true;
    }
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
      ([key, value]) =>
        value &&
        value === combo &&
        !dialogShortcuts.includes(key as ShortcutKey),
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
    if (
      !entry &&
      combo === "Meta+a" &&
      w.preferences.shortcuts.selectAll === "Control+a"
    )
      entry = ["selectAll", "Control+a"];
    if (!entry) return false;
    const action = entry[0];
    if (action === "selectAll") {
      if (!root.querySelector(".mail-list")?.contains(document.activeElement))
        return false;
      w.selection.all();
      return true;
    }
    if (w.selection.mode && action !== "search") return true;
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
      const focused = (
        document.activeElement as Element | null
      )?.closest<HTMLElement>(".mail-row")?.dataset.id;
      const id = focused ?? w.selected;
      if (id) {
        fullReader = true;
        w.beginReading(id);
      }
    } else act(action as Action);
    return true;
  }
  render();
  return {
    openPreferences() {
      tab = "Preferences";
      w.changed();
    },
  };
}
