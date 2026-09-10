import { expect, test } from "vitest";
import { defaults } from "./model";
import {
  dialogShortcuts,
  keyCombo,
  reviewDecision,
  shortcutLabel,
  shortcutName,
} from "./shortcut_keys";

const press = (
  key: string,
  modifiers: Partial<Parameters<typeof keyCombo>[0]> = {},
) =>
  keyCombo({
    key,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    ...modifiers,
  });

test("review keys follow Y/N/Enter/Escape by default and honour remapped or disabled bindings", () => {
  const s = defaults.shortcuts;
  expect(reviewDecision(press("y"), s)).toBe("approve");
  expect(reviewDecision(press("Y", { shiftKey: true }), s)).toBeUndefined();
  expect(reviewDecision(press("Enter"), s)).toBe("approve");
  expect(reviewDecision(press("n"), s)).toBe("decline");
  expect(reviewDecision(press("Escape"), s)).toBe("decline");
  expect(reviewDecision(press("m"), s)).toBeUndefined();
  const remapped = { ...s, approve: "Control+Enter", decline: "" };
  expect(reviewDecision(press("y"), remapped)).toBeUndefined();
  expect(reviewDecision(press("Enter", { ctrlKey: true }), remapped)).toBe(
    "approve",
  );
  expect(reviewDecision(press("n"), remapped)).toBeUndefined();
  expect(reviewDecision(press("Escape"), remapped)).toBe("decline");
});

test("dialog shortcuts are named for Preferences and excluded from mail browsing", () => {
  expect(dialogShortcuts).toEqual(["approve", "decline"]);
  expect(shortcutName("approve")).toBe("approve review");
  expect(shortcutLabel("decline")).toBe("Decline review");
  expect(shortcutLabel("selectAll")).toBe("Select all messages");
  expect(shortcutLabel("reply")).toBe("Reply");
  expect(press("a", { ctrlKey: true })).toBe("Control+a");
  expect(press("F", { metaKey: true, shiftKey: true })).toBe("Meta+Shift+f");
});
