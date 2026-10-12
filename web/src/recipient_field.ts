import { el, icon } from "./ui";
import {
  RecipientEditor,
  historyStep,
  pasteLines,
  type Chip,
} from "./recipients";

const mac = /Mac|iPhone|iPad/.test(navigator.platform);
let ids = 0;

export interface RecipientField {
  view: HTMLElement;
  /** Shows a saved value without marking the draft edited. */
  set(value: string): void;
}

type ChipNode = { text: string; valid: boolean; node: HTMLLIElement };

/**
 * A To/Cc/Bcc box of removable chips followed by an entry input. `onChange`
 * receives the comma-separated draft value whenever it changes.
 */
export function recipientField(
  label: string,
  value: string,
  placeholder: string,
  onChange: (value: string) => void,
): RecipientField {
  const editor = new RecipientEditor(value);
  const view = el("div", "field recipient-field");
  const id = `recipient-entry-${++ids}`;
  const caption = el("label", "", label);
  caption.htmlFor = id;
  const box = el("div", "recipient-box");
  const list = el("ul", "recipient-chips");
  list.setAttribute("aria-label", `${label} recipients`);
  const entry = el("input", "recipient-entry");
  entry.id = id;
  entry.setAttribute("aria-label", label);
  entry.setAttribute("placeholder", placeholder);
  entry.dataset.focus = label;
  entry.autocomplete = "off";
  entry.spellcheck = false;
  box.append(list, entry);
  view.append(caption, box);
  let shown: ChipNode[] = [];
  let written = editor.value;

  function chipNode(c: Chip): ChipNode {
    const node = el("li", c.valid ? "recipient-chip" : "recipient-chip invalid");
    if (!c.valid) {
      const mark = el("span", "recipient-warning", "!");
      mark.setAttribute("aria-hidden", "true");
      node.title = "Send cannot use this address. Remove it or edit it.";
      node.append(mark);
    }
    node.append(el("span", "recipient-text", c.text));
    if (!c.valid) node.append(el("span", "visually-hidden", " (invalid address)"));
    const remove = el("button", "recipient-remove");
    remove.type = "button";
    remove.setAttribute("aria-label", `Remove ${c.text}`);
    remove.title = `Remove ${c.text}`;
    remove.disabled = entry.disabled;
    remove.append(icon("close"));
    remove.onclick = () => {
      if (entry.disabled) return;
      const index = shown.findIndex((s) => s.node === node);
      if (index >= 0 && editor.remove(index, c.text)) update();
      entry.focus();
    };
    node.append(remove);
    return { text: c.text, valid: c.valid, node };
  }

  // Keeps unchanged chip nodes in place so a press on one survives a redraw.
  function renderChips() {
    const chips = editor.state.chips;
    const same = (s: ChipNode, c: Chip) => s.text === c.text && s.valid === c.valid;
    let start = 0;
    while (start < shown.length && start < chips.length && same(shown[start], chips[start]))
      start++;
    let end = 0;
    while (
      end < shown.length - start &&
      end < chips.length - start &&
      same(shown[shown.length - 1 - end], chips[chips.length - 1 - end])
    )
      end++;
    const removed = shown.slice(start, shown.length - end);
    const suffix = shown.slice(shown.length - end);
    const added = chips.slice(start, chips.length - end).map(chipNode);
    for (const r of removed) r.node.remove();
    const before = suffix[0]?.node ?? null;
    for (const a of added) list.insertBefore(a.node, before);
    shown = [...shown.slice(0, start), ...added, ...suffix];
  }

  function update(caret?: number) {
    renderChips();
    if (entry.value !== editor.state.entry) {
      entry.value = editor.state.entry;
      const at = caret ?? entry.value.length;
      if (document.activeElement === entry) entry.setSelectionRange(at, at);
    }
    const next = editor.value;
    if (next !== written) {
      written = next;
      onChange(next);
    }
  }

  entry.addEventListener("input", () => {
    if (editor.type(entry.value)) update();
  });
  entry.addEventListener("paste", (e) => {
    const text = e.clipboardData?.getData("text/plain");
    if (text == null || entry.disabled) return;
    e.preventDefault();
    const mapped = pasteLines(text);
    const from = entry.selectionStart ?? entry.value.length;
    const to = entry.selectionEnd ?? from;
    const raw = entry.value.slice(0, from) + mapped + entry.value.slice(to);
    if (!editor.type(raw, true)) return;
    update(editor.state.entry === raw ? from + mapped.length : undefined);
  });
  entry.addEventListener("keydown", (e) => {
    if (e.defaultPrevented || e.isComposing) return;
    const step = historyStep(e, mac);
    if (step) {
      e.preventDefault();
      if (editor.step(step)) update();
      return;
    }
    const plain = !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey;
    if (plain && e.key === "Enter") {
      e.preventDefault();
      if (editor.commit()) update();
    } else if (
      plain &&
      e.key === "Backspace" &&
      !entry.value &&
      editor.reopen()
    ) {
      e.preventDefault();
      update();
    }
  });
  // Edit menu Undo/Redo also goes through the field's own history.
  entry.addEventListener("beforeinput", (e) => {
    if (e.inputType !== "historyUndo" && e.inputType !== "historyRedo") return;
    e.preventDefault();
    if (editor.step(e.inputType === "historyUndo" ? "undo" : "redo")) update();
  });
  entry.addEventListener("blur", () => {
    if (!entry.disabled && editor.commit()) update();
  });
  box.addEventListener("mousedown", (e) => {
    if (e.target !== box && e.target !== list) return;
    e.preventDefault();
    entry.focus();
  });
  // Disabling the entry (while sending) also disables the chip buttons.
  new MutationObserver(() => {
    for (const s of shown)
      s.node.querySelector("button")!.disabled = entry.disabled;
  }).observe(entry, { attributes: true, attributeFilter: ["disabled"] });

  renderChips();
  entry.value = editor.state.entry;
  return {
    view,
    set(next) {
      if (next === editor.value) return;
      editor.set(next);
      written = editor.value;
      update();
    },
  };
}
