import { describe, expect, it } from "vitest";
import cases from "../../shared/recipient-cases.json";
import {
  History,
  RecipientEditor,
  chip,
  editBetween,
  historyStep,
  parseRecipients,
  pasteLines,
  recipientsValue,
  reopenLast,
  removeChip,
  splitRecipients,
  validRecipient,
} from "./recipients";

const texts = (r: { chips: readonly { text: string }[] }) =>
  r.chips.map((c) => c.text);

describe("shared recipient contract", () => {
  it("splits typed and pasted text", () => {
    expect(cases.split.length).toBeGreaterThanOrEqual(20);
    for (const c of cases.split)
      expect(splitRecipients(c.input), c.name).toEqual([c.chips, c.entry]);
  });
  it("turns every stored address into a chip", () => {
    for (const c of cases.stored) {
      const r = parseRecipients(c.value);
      expect(texts(r), c.name).toEqual(c.chips);
      expect(r.entry, c.name).toBe("");
    }
  });
  it("writes chips and entry back as the draft value", () => {
    for (const c of cases.value)
      expect(
        recipientsValue({ chips: c.chips.map(chip), entry: c.entry }),
        c.name,
      ).toBe(c.value);
  });
  it("maps pasted lines", () => {
    for (const c of cases.paste)
      expect(pasteLines(c.clipboard), c.name).toBe(c.text);
  });
  it("marks chips as Send would treat them", () => {
    for (const c of cases.valid)
      expect(validRecipient(c.chip), c.chip).toBe(c.valid);
  });
});

describe("recipient chips", () => {
  it("rewrites semicolon lists with commas", () => {
    const r = parseRecipients("a@example.com;b@example.com");
    expect(recipientsValue(r)).toBe("a@example.com, b@example.com");
    expect(r.chips.every((c) => c.valid)).toBe(true);
  });
  it("removes only the chip the view showed", () => {
    const r = parseRecipients("a@example.com, b@example.com");
    expect(removeChip(r, 0, "b@example.com")).toBe(r);
    expect(removeChip(r, 5, "a@example.com")).toBe(r);
    expect(recipientsValue(removeChip(r, 1, "b@example.com"))).toBe(
      "a@example.com",
    );
  });
  it("reopens the last chip only into an empty entry", () => {
    const r = parseRecipients("a@example.com, b@example.com");
    expect(reopenLast({ ...r, entry: "c" }).entry).toBe("c");
    const reopened = reopenLast(r);
    expect(reopened.entry).toBe("b@example.com");
    expect(texts(reopened)).toEqual(["a@example.com"]);
    expect(recipientsValue(reopened)).toBe(recipientsValue(r));
  });
});

describe("recipient history", () => {
  const editor = (value = "") => {
    let now = 0;
    const e = new RecipientEditor(value, () => now);
    return { e, advance: (ms: number) => (now += ms) };
  };
  const typeChars = (e: RecipientEditor, text: string) => {
    for (const c of text) e.type(e.state.entry + c);
  };

  it("undoes typing a word at a time and redoes it", () => {
    const { e } = editor();
    typeChars(e, "Alex Doe");
    expect(e.step("undo")).toBe(true);
    expect(e.state.entry).toBe("Alex");
    expect(e.step("undo")).toBe(true);
    expect(e.state.entry).toBe("");
    expect(e.step("undo")).toBe(false);
    e.step("redo");
    e.step("redo");
    expect(e.state.entry).toBe("Alex Doe");
  });

  it("makes separators, pastes, removals and reopening their own steps", () => {
    const { e } = editor("a@example.com");
    typeChars(e, "b@example.com,");
    expect(e.value).toBe("a@example.com, b@example.com");
    e.type("c@example.com; d@example.com;", true);
    e.remove(0, "a@example.com");
    e.reopen();
    expect(texts(e.state)).toEqual(["b@example.com", "c@example.com"]);
    expect(e.state.entry).toBe("d@example.com");
    e.step("undo");
    expect(texts(e.state)).toHaveLength(3);
    e.step("undo");
    expect(e.value).toBe(
      "a@example.com, b@example.com, c@example.com, d@example.com",
    );
    e.step("undo");
    expect(e.value).toBe("a@example.com, b@example.com");
    e.step("undo");
    expect(e.value).toBe("a@example.com, b@example.com");
    expect(e.state.entry).toBe("b@example.com");
    e.step("undo");
    expect(e.value).toBe("a@example.com");
  });

  it("commits on leaving and undo returns to before the typed text", () => {
    const { e } = editor();
    typeChars(e, "alex@example.com");
    expect(e.commit()).toBe(true);
    expect(texts(e.state)).toEqual(["alex@example.com"]);
    expect(e.commit()).toBe(false);
    e.step("undo");
    expect(e.value).toBe("");
  });

  it("starts a new step after a pause and drops redo after a new edit", () => {
    const { e, advance } = editor();
    typeChars(e, "ab");
    advance(2500);
    typeChars(e, "cd");
    e.step("undo");
    expect(e.state.entry).toBe("ab");
    typeChars(e, "x");
    expect(e.step("redo")).toBe(false);
  });

  it("is bounded to 100 steps", () => {
    const h = new History<string>((s) => s.length);
    for (let i = 0; i < 120; i++) h.record("other", 0, String(i));
    expect(h.depth).toBe(100);
    let last: string | undefined, state: string | undefined = "now";
    while ((state = h.undo(state ?? "")) !== undefined) last = state;
    expect(last).toBe("20");
  });

  it("classifies field changes and history keys", () => {
    expect(editBetween("ab", "abc")).toBe("word");
    expect(editBetween("ab", "ab ")).toBe("space");
    expect(editBetween("abc", "ab")).toBe("delete");
    expect(editBetween("ab", "xy")).toBe("other");
    const key = (key: string, mods: Partial<KeyboardEvent> = {}) => ({
      key,
      ctrlKey: false,
      metaKey: false,
      shiftKey: false,
      altKey: false,
      ...mods,
    });
    expect(historyStep(key("z", { ctrlKey: true }), false)).toBe("undo");
    expect(historyStep(key("Z", { ctrlKey: true, shiftKey: true }), false)).toBe(
      "redo",
    );
    expect(historyStep(key("y", { ctrlKey: true }), false)).toBe("redo");
    expect(historyStep(key("z", { metaKey: true }), true)).toBe("undo");
    expect(historyStep(key("z", { ctrlKey: true }), true)).toBeUndefined();
    expect(historyStep(key("z", { ctrlKey: true, altKey: true }), false)).toBe(
      undefined,
    );
    expect(historyStep(key("z"), false)).toBeUndefined();
  });
});
