// Recipient chips for the To, Cc and Bcc fields. Drafts keep the
// comma-separated string; chips are only the editing model.
// `shared/recipient-cases.json` is the contract shared with the other clients.

export interface Chip {
  readonly text: string;
  /** Whether Send would accept this address. */
  readonly valid: boolean;
}

export interface Recipients {
  readonly chips: readonly Chip[];
  /** The unfinished text in the field's input. */
  readonly entry: string;
}

export const emptyRecipients: Recipients = { chips: [], entry: "" };

export function chip(text: string): Chip {
  return { text, valid: validRecipient(text) };
}

/** A stored draft value: every address becomes a chip. */
export function parseRecipients(value: string): Recipients {
  return commitEntry(inputEntry(emptyRecipients, value));
}

/** The draft value: every chip, then the unfinished entry. */
export function recipientsValue(r: Recipients): string {
  const parts = r.chips.map((c) => c.text);
  if (r.entry) parts.push(r.entry);
  return parts.join(", ");
}

/** Replaces the entry text, turning each finished address into a chip. */
export function inputEntry(r: Recipients, text: string): Recipients {
  const [finished, entry] = splitRecipients(text);
  return { chips: [...r.chips, ...finished.map(chip)], entry };
}

/** Turns the unfinished entry into a chip. */
export function commitEntry(r: Recipients): Recipients {
  const text = r.entry.trim();
  if (!text) return r.entry ? { chips: r.chips, entry: "" } : r;
  return { chips: [...r.chips, chip(text)], entry: "" };
}

/** Removes a chip if it still holds the text the view showed. */
export function removeChip(
  r: Recipients,
  index: number,
  text: string,
): Recipients {
  if (r.chips[index]?.text !== text) return r;
  return { chips: r.chips.filter((_, i) => i !== index), entry: r.entry };
}

/** Moves the last chip back into an empty entry for editing. */
export function reopenLast(r: Recipients): Recipients {
  const last = r.chips.at(-1);
  if (r.entry || !last) return r;
  return { chips: r.chips.slice(0, -1), entry: last.text };
}

export function sameRecipients(a: Recipients, b: Recipients) {
  return (
    a.entry === b.entry &&
    a.chips.length === b.chips.length &&
    a.chips.every((c, i) => c.text === b.chips[i].text)
  );
}

const whitespace = /\s/u;

/**
 * Splits typed or pasted text into finished addresses and the unfinished rest.
 * Commas, semicolons and line breaks separate addresses, except inside a quoted
 * display name or `<...>`. Whitespace also finishes a complete bare address or
 * one ending in `>`.
 */
export function splitRecipients(text: string): [string[], string] {
  const finished: string[] = [];
  let current = "",
    quoted = false,
    escaped = false,
    depth = 0;
  const finish = () => {
    const t = current.trim();
    if (t) finished.push(t);
    current = "";
  };
  for (const c of text) {
    if (quoted) {
      current += c;
      if (escaped) escaped = false;
      else if (c === "\\") escaped = true;
      else if (c === '"') quoted = false;
      continue;
    }
    if (c === '"') {
      quoted = true;
      current += c;
    } else if (c === "<") {
      depth++;
      current += c;
    } else if (c === ">") {
      depth = Math.max(0, depth - 1);
      current += c;
    } else if (depth === 0 && (c === "," || c === ";" || c === "\n" || c === "\r"))
      finish();
    else if (depth === 0 && whitespace.test(c) && complete(current.trim()))
      finish();
    else current += c;
  }
  return [finished, current.trimStart()];
}

function complete(text: string) {
  if (text.endsWith(">")) return true;
  const at = text.indexOf("@");
  if (at < 0) return false;
  const local = text.slice(0, at),
    domain = text.slice(at + 1);
  return (
    !!local && !!domain && !domain.includes("@") && !/[\s<>"()]/u.test(text)
  );
}

/** Keeps a copied column of addresses apart: text inputs drop line breaks. */
export function pasteLines(text: string): string {
  if (!/[\n\r\t]/.test(text)) return text;
  return text
    .replaceAll("\t", " ")
    .split(/[\n\r]/)
    .map((line) => line.trim())
    .filter(Boolean)
    .join(", ");
}

// Checks one mailbox the way Send parses it: `local@domain`, optionally as
// `Display Name <local@domain>` with a plain or quoted display name.
const atext = String.raw`[^\s<>()\[\]\\,;:@"]+`;
const addrSpec = new RegExp(`^${atext}@${atext}$`, "u");
const phrase = new RegExp(`^${atext}(?:\\s+${atext})*$`, "u");
const quotedName = /^"(?:[^"\\]|\\.)*"$/u;

export function validRecipient(text: string): boolean {
  const t = text.trim();
  if (!t.endsWith(">")) return addrSpec.test(t);
  const open = t.lastIndexOf("<");
  if (open < 0) return false;
  const name = t.slice(0, open).trim();
  const address = t.slice(open + 1, -1);
  if (!addrSpec.test(address)) return false;
  return !name || quotedName.test(name) || phrase.test(name);
}

// Bounded undo/redo for one field. Typing coalesces into word-sized steps.
const STEPS = 100;
const BYTES = 4 << 20;
const PAUSE_MS = 2000;

export type Edit = "word" | "space" | "delete" | "other";

/** Classifies a single-line field change by comparing its values. */
export function editBetween(before: string, after: string): Edit {
  const old = [...before].length,
    next = [...after].length;
  if (next === old + 1 && after.startsWith(before))
    return whitespace.test([...after].at(-1) ?? "") ? "space" : "word";
  if (next === old + 1) return "word";
  if (next + 1 === old) return "delete";
  return "other";
}

function continues(last: Edit, next: Edit) {
  if (last === "word" && next === "space") return false;
  if (last === "word" || last === "space")
    return next === "word" || next === "space";
  return last === "delete" && next === "delete";
}

export class History<T> {
  private undoStack: T[] = [];
  private redoStack: T[] = [];
  private bytes = 0;
  private open: { edit: Edit; at: number } | undefined;
  constructor(private readonly weight: (state: T) => number) {}

  get depth() {
    return this.undoStack.length;
  }

  /** Saves the state before an edit unless the edit continues the open step. */
  record(edit: Edit, now: number, before: T) {
    const open = this.open;
    const joined =
      !!open && now - open.at < PAUSE_MS && continues(open.edit, edit);
    for (const state of this.redoStack) this.bytes -= this.weight(state);
    this.redoStack = [];
    this.open = edit === "other" ? undefined : { edit, at: now };
    if (!joined) this.push(before);
  }

  /** Ends the open step, so the next edit starts a new one. */
  seal() {
    this.open = undefined;
  }

  undo(current: T): T | undefined {
    const previous = this.undoStack.pop();
    if (previous === undefined) return undefined;
    this.bytes += this.weight(current) - this.weight(previous);
    this.redoStack.push(current);
    this.open = undefined;
    return previous;
  }

  redo(current: T): T | undefined {
    const next = this.redoStack.pop();
    if (next === undefined) return undefined;
    this.bytes -= this.weight(next);
    this.push(current);
    this.open = undefined;
    return next;
  }

  private push(state: T) {
    this.bytes += this.weight(state);
    this.undoStack.push(state);
    while (
      this.undoStack.length > STEPS ||
      (this.bytes > BYTES && this.undoStack.length > 1)
    )
      this.bytes -= this.weight(this.undoStack.shift() as T);
  }
}

export type Step = "undo" | "redo";

/** Ctrl+Z undoes; Ctrl+Shift+Z and Ctrl+Y redo (Command on macOS). */
export function historyStep(
  e: {
    key: string;
    ctrlKey: boolean;
    metaKey: boolean;
    shiftKey: boolean;
    altKey: boolean;
  },
  mac: boolean,
): Step | undefined {
  const command = mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  if (!command || e.altKey) return undefined;
  const key = e.key.toLowerCase();
  if (key === "z") return e.shiftKey ? "redo" : "undo";
  if (key === "y" && !e.shiftKey) return "redo";
  return undefined;
}

function weight(r: Recipients) {
  return r.entry.length + r.chips.reduce((n, c) => n + c.text.length, 0);
}

/** One recipient field's chips plus its own undo history. */
export class RecipientEditor {
  private history = new History<Recipients>(weight);
  state: Recipients;
  constructor(
    value: string,
    private readonly clock: () => number = () => performance.now(),
  ) {
    this.state = parseRecipients(value);
  }

  get value() {
    return recipientsValue(this.state);
  }

  private apply(next: Recipients, edit: Edit) {
    if (sameRecipients(next, this.state)) return false;
    this.history.record(edit, this.clock(), this.state);
    this.state = next;
    return true;
  }

  /** The entry's raw text after typing; `paste` makes it its own step. */
  type(text: string, paste = false) {
    const next = inputEntry({ chips: this.state.chips, entry: "" }, text);
    const edit =
      paste || next.chips.length !== this.state.chips.length
        ? "other"
        : editBetween(this.state.entry, next.entry);
    return this.apply(next, edit);
  }

  remove(index: number, text: string) {
    return this.apply(removeChip(this.state, index, text), "other");
  }

  reopen() {
    return this.apply(reopenLast(this.state), "other");
  }

  /** Replaces the whole field, keeping unfinished text as the entry. */
  set(value: string) {
    return this.apply(inputEntry(emptyRecipients, value), "other");
  }

  /** Undo after a commit returns to the state before the typed text. */
  commit() {
    const next = commitEntry(this.state);
    if (next === this.state) return false;
    this.state = next;
    this.history.seal();
    return true;
  }

  step(step: Step) {
    const state =
      step === "undo"
        ? this.history.undo(this.state)
        : this.history.redo(this.state);
    if (!state) return false;
    this.state = state;
    return true;
  }
}
