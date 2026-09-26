// The Move chooser: ranked destination folders, account badges on folders in
// other IMAP accounts, and a confirmation step before moving one message to
// another account. Group moves hand every choice to the bulk review.
import { button, el, icon, modal } from "./ui";
import { keyCombo, keyConsumed, reviewDecision } from "./shortcut_keys";
import { initializeAttachments } from "./attachment_content";
import { rank_move_candidates } from "./wasm/shep_mail_content";
import type { Preferences } from "./model";
import {
  accountDisplay,
  foreignAccounts,
  gather,
  rank,
  type MoveAccount,
  type MoveCandidate,
  type MoveHome,
  type Ranker,
} from "./move_candidates";

export interface MoveChooserOptions {
  /** Selection mode reviews every choice; message mode moves at once. */
  group: boolean;
  accounts(): MoveAccount[];
  home: MoveHome;
  /** Folders for an account whose catalogue has not loaded yet. */
  fallback: string[];
  /** Moving between accounts is allowed and available in this browser. */
  crossAccount(): boolean;
  /** Other accounts' folders are offered while typing. */
  foreignEnabled(): boolean;
  shortcuts(): Preferences["shortcuts"];
  /** A folder in each message's own account. */
  move(folder: string): void;
  /** A folder in another account: a confirmed or reviewed foreign row, or a
   * folder of the explicitly chosen destination account. */
  transfer(account: string, folder: string, foreign: boolean): void;
}

let ranker: Promise<Ranker> | undefined;
const sharedRanker = () =>
  (ranker ??= initializeAttachments().then(
    () => rank_move_candidates,
    (error) => {
      ranker = undefined;
      throw error;
    },
  ));

/** Plain substring order, used only when the shared matcher cannot load. */
const plainRanker: Ranker = (query, json) => {
  const rows = JSON.parse(json) as { label: string; foreign: boolean }[];
  const needle = query.trim().toLowerCase();
  return JSON.stringify(
    rows.flatMap((row, i) =>
      !row.foreign && row.label.toLowerCase().includes(needle) ? [i] : [],
    ),
  );
};

/** The account badge with its secondary identity beside it. */
export function accountBadge(accounts: MoveAccount[], id: string) {
  const wrap = el("span", "account-identity");
  const display = accountDisplay(accounts, id) ?? id;
  const account = accounts.find((a) => a.id === id);
  const secondary = account
    ? display === account.email
      ? account.name.trim()
      : account.email
    : "";
  wrap.append(el("span", "account-badge", display));
  if (secondary) wrap.append(el("span", "muted", secondary));
  return wrap;
}

export function openMoveChooser(options: MoveChooserOptions) {
  const title = options.group ? "Move selected messages" : "Move message";
  const d = modal(title);
  d.classList.add("move-dialog");
  const heading = d.querySelector("h2")!;
  const listView = el("div", "move-choose");
  const source =
    options.home.source.kind === "message" ? options.home.source.account : null;
  const selection =
    options.home.source.kind === "selection"
      ? (options.home.source.accounts ?? [])
      : [];
  let explicit: string | null = null;
  const imap = (id: string) =>
    options.accounts().some((a) => a.id === id && a.imap);
  // Desktop shows the destination account list only when every source is IMAP.
  if (
    options.crossAccount() &&
    (options.group ? selection.length > 0 && selection.every(imap) : true)
  ) {
    const wrap = el("label", "select-field");
    wrap.append(el("span", "", "Destination account"));
    const choice = el("select");
    choice.setAttribute("aria-label", "Destination account");
    if (options.group) {
      const each = el("option", "", "Each message’s account");
      each.value = "";
      choice.append(each);
    }
    for (const account of options.accounts().filter((a) => a.imap)) {
      const option = el("option", "", account.name || account.email);
      option.value = account.id;
      option.selected = !options.group && account.id === source;
      choice.append(option);
    }
    choice.onchange = () => {
      explicit = choice.value || null;
      render();
    };
    wrap.append(choice);
    listView.append(wrap);
  }
  const search = el("label", "field");
  const input = el("input");
  input.placeholder = "Find a folder…";
  input.autocomplete = "off";
  const inputLabel = options.group ? "Destination folder" : "Find a folder";
  input.setAttribute("aria-label", inputLabel);
  search.append(el("span", "", inputLabel), input);
  const note = el("p", "muted move-note");
  const list = el("div", "folder-choices");
  list.setAttribute("role", "group");
  list.setAttribute("aria-label", "Destination folders");
  const empty = el("p", "muted move-empty");
  listView.append(search, note, list, empty);
  const confirmView = el("div", "move-confirm");
  confirmView.tabIndex = -1;
  confirmView.hidden = true;
  d.append(listView, confirmView);

  let loaded: Ranker | undefined;
  let rankFailed = false;
  let candidates: MoveCandidate[] = [];
  let pending: MoveCandidate | undefined;
  const display = (account: string) =>
    accountDisplay(options.accounts(), account) ?? account;
  const home = (): MoveHome => ({ ...options.home, explicit });

  function render() {
    const query = input.value;
    const ready = !!loaded || rankFailed;
    candidates = ready
      ? rank(
          query,
          gather(
            options.accounts(),
            home(),
            options.fallback,
            options.foreignEnabled() && !rankFailed,
            !query.trim(),
          ),
          loaded ?? plainRanker,
        )
      : [];
    list.replaceChildren();
    candidates.forEach((candidate, index) => {
      const row = el("button", `button move-choice${index ? "" : " target"}`);
      row.type = "button";
      const account = candidate.foreign ? display(candidate.account) : "";
      row.setAttribute(
        "aria-label",
        candidate.foreign
          ? `${candidate.label} in ${account}`
          : candidate.label,
      );
      row.append(icon("folder"), el("span", "move-label", candidate.label));
      if (candidate.foreign) row.append(el("span", "account-badge", account));
      const trailing = index ? icon("chevron") : el("span", "move-hint", "Enter ↵");
      trailing.setAttribute("aria-hidden", "true");
      row.append(trailing);
      row.onclick = () => choose(candidate);
      list.append(row);
    });
    const text = query.trim();
    empty.textContent = !ready
      ? ""
      : !text
          ? "No shared destination folders. Refresh mail to load each account’s folders."
          : options.foreignEnabled()
            ? "No matching folders in any account."
            : "No matching folders.";
    empty.hidden = !ready || candidates.length > 0;
    note.textContent = rankFailed
      ? "Folder search could not load. Other accounts' folders are unavailable; reload Shep to retry."
      : !options.group
        ? ""
        : explicit
          ? "The messages move to the chosen account after review."
          : foreignAccounts(options.accounts(), selection, options.foreignEnabled())
                .length
            ? "Folders without an account badge keep each message in its original account. A badged folder moves the messages to that account after review."
            : "Each message stays in its original account.";
    note.hidden = !note.textContent;
  }
  /** A folder in the explicit account, or in each message's own account. */
  function moveTo(folder: string) {
    if (explicit && (options.group || explicit !== source))
      options.transfer(explicit, folder, false);
    else options.move(folder);
  }
  function choose(candidate: MoveCandidate) {
    if (!candidate.foreign) {
      d.close();
      moveTo(candidate.folder);
      return;
    }
    if (!options.foreignEnabled()) return;
    if (options.group) {
      d.close();
      options.transfer(candidate.account, candidate.folder, true);
      return;
    }
    confirm(candidate);
  }
  function confirm(candidate: MoveCandidate) {
    pending = candidate;
    heading.textContent = "Move to another account?";
    d.setAttribute("aria-label", "Move to another account?");
    const question = el("h3", "", `Move to ${candidate.label}?`);
    question.id = "move-confirm-question";
    const actions = el("div", "move-confirm-actions");
    const move = button("Move", accept);
    move.classList.add("primary");
    actions.append(button("Cancel", back), move);
    confirmView.replaceChildren(
      question,
      accountBadge(options.accounts(), candidate.account),
      el("p", "muted", "Enter moves it, Escape returns to the folder list."),
      actions,
    );
    confirmView.setAttribute("role", "group");
    confirmView.setAttribute("aria-labelledby", question.id);
    listView.hidden = true;
    confirmView.hidden = false;
    confirmView.focus();
  }
  function back() {
    if (!pending) return;
    pending = undefined;
    heading.textContent = title;
    d.setAttribute("aria-label", title);
    confirmView.hidden = true;
    confirmView.replaceChildren();
    listView.hidden = false;
    render();
    input.focus();
  }
  function accept() {
    const chosen = pending;
    if (!chosen) return;
    pending = undefined;
    d.close();
    options.transfer(chosen.account, chosen.folder, true);
  }
  input.oninput = render;
  const ready = sharedRanker().then(
    (value) => {
      loaded = value;
    },
    () => {
      rankFailed = true;
    },
  );
  input.onkeydown = (e) => {
    if (e.defaultPrevented || keyCombo(e) !== "Enter" || e.isComposing) return;
    e.preventDefault();
    if (e.repeat) return;
    // Enter resolves the latest typed text once the shared matcher is ready.
    void ready.then(() => {
      if (!d.open || pending) return;
      render();
      // Only listed folders are destinations; an unknown name opens nothing.
      const first = candidates[0];
      if (first) choose(first);
    });
  };
  // The confirmation owns its keys; a held Enter that chose a row never repeats.
  d.addEventListener(
    "keydown",
    (e) => {
      if (!pending) return;
      if (e.key === "Enter" && e.repeat) {
        e.preventDefault();
        e.stopPropagation();
        return;
      }
      if (keyConsumed(e)) return;
      const decision = reviewDecision(keyCombo(e), options.shortcuts());
      if (!decision) return;
      e.preventDefault();
      e.stopPropagation();
      if (decision === "approve") accept();
      else back();
    },
    true,
  );
  d.addEventListener("cancel", (e) => {
    if (!pending) return;
    e.preventDefault();
    back();
  });
  render();
  input.focus();
  void ready.then(() => {
    if (d.open && !pending) render();
  });
  return d;
}
