import type { Preferences } from "./model";

export type ShortcutKey = keyof Preferences["shortcuts"];
/** Review keys act only inside group review/History dialogs; they are never
 * forwarded from the formatted reader frame or run while browsing mail. */
export const dialogShortcuts: ShortcutKey[] = ["approve", "decline"];
const names: Partial<Record<ShortcutKey, string>> = {
  selectAll: "select all messages",
  approve: "approve review",
  decline: "decline review",
};
export const shortcutName = (key: ShortcutKey) => names[key] ?? key;
export const shortcutLabel = (key: ShortcutKey) => {
  const name = shortcutName(key);
  return name[0].toUpperCase() + name.slice(1);
};
export interface KeyLike {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}
export function keyCombo(e: KeyLike) {
  return [
    e.ctrlKey ? "Control" : e.metaKey ? "Meta" : "",
    e.altKey ? "Alt" : "",
    e.shiftKey ? "Shift" : "",
    e.key.length === 1 ? e.key.toLowerCase() : e.key,
  ]
    .filter(Boolean)
    .join("+");
}
/** True when the key belongs to its target: an already handled event, a text
 * field or editor, or native Enter/Space activation on a control. */
export function keyConsumed(e: KeyLike & { defaultPrevented: boolean }) {
  const target = (e as unknown as Event).target;
  if (e.defaultPrevented || !(target instanceof Element)) return true;
  const plain = !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey;
  if (
    plain &&
    (e.key === "Enter" || e.key === " ") &&
    target.closest(
      "button, summary, a[href], input[type=checkbox], input[type=radio]",
    )
  )
    return true;
  return (
    (target instanceof HTMLInputElement && target.type !== "checkbox") ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement ||
    (target as HTMLElement).isContentEditable
  );
}
export function reviewDecision(
  combo: string,
  shortcuts: Preferences["shortcuts"],
): "approve" | "decline" | undefined {
  if (combo === "Enter" || (shortcuts.approve && combo === shortcuts.approve))
    return "approve";
  if (combo === "Escape" || (shortcuts.decline && combo === shortcuts.decline))
    return "decline";
  return undefined;
}
