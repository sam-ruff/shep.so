// Portable preference mapping for the browser client plus the device-side
// application contract used by enrollment: per-field revisions advance on
// every explicit local save (including change-and-revert), and a receipt
// freezes the revisions of the original application so a retry after a lost
// reply returns the same proof instead of a later snapshot.
import { type Preferences, type SettingsStore, defaults } from "./model";
import type { SettingKey } from "./profile_types";

export const BROWSER_SETTINGS = [
  "appearance",
  "preview_lines",
  "sender_pictures",
  "reply_display",
] as const satisfies readonly SettingKey[];
export type BrowserSettingKey = (typeof BROWSER_SETTINGS)[number];
export interface PortableValues {
  values: Record<BrowserSettingKey, unknown>;
  revisions: Record<BrowserSettingKey, number>;
}
export interface ApplyRequest {
  id: string;
  baseline: PortableValues;
  changes: Partial<Record<BrowserSettingKey, unknown>>;
}
export interface ApplyReceipt {
  id: string;
  applied: BrowserSettingKey[];
  kept: BrowserSettingKey[];
  revisions: Record<BrowserSettingKey, number>;
}
const quoteToWire: Record<Preferences["quoteMode"], string> = {
  Collapsed: "Collapsed",
  Expanded: "Expanded",
  "Latest only": "LatestOnly",
};
const wireToQuote = Object.fromEntries(
  Object.entries(quoteToWire).map(([k, v]) => [v, k]),
) as Record<string, Preferences["quoteMode"]>;
export function portableValues(
  p: Preferences,
): Record<BrowserSettingKey, unknown> {
  return {
    appearance: p.appearance[0].toUpperCase() + p.appearance.slice(1),
    preview_lines: p.previewLines,
    sender_pictures: p.avatars,
    reply_display: quoteToWire[p.quoteMode],
  };
}
/// Convert one portable value; unsupported or invalid values return null so
/// the caller can keep the local value rather than guess.
export function applyPortable(
  p: Preferences,
  key: BrowserSettingKey,
  value: unknown,
): Preferences | null {
  switch (key) {
    case "appearance":
      return typeof value === "string" &&
        ["System", "Light", "Dark"].includes(value)
        ? { ...p, appearance: value.toLowerCase() as Preferences["appearance"] }
        : null;
    case "preview_lines":
      return Number.isInteger(value) &&
        (value as number) >= 0 &&
        (value as number) <= 4
        ? { ...p, previewLines: value as number }
        : null;
    case "sender_pictures":
      return typeof value === "boolean" ? { ...p, avatars: value } : null;
    case "reply_display":
      return typeof value === "string" && value in wireToQuote
        ? { ...p, quoteMode: wireToQuote[value] }
        : null;
  }
}
export function defaultPortable(key: BrowserSettingKey): unknown {
  return portableValues(defaults)[key];
}
export function describeSetting(key: SettingKey, value: unknown): string {
  const labels: Record<SettingKey, string> = {
    appearance: "Theme",
    reply_display: "Quoted history",
    image_policy: "Remote images",
    unified_inbox: "Unified inbox",
    cross_account_moves: "Cross-account moves",
    group_conversations: "Conversation grouping",
    desktop_badges: "Unread badges",
    preview_lines: "Preview lines",
    left_swipe: "Left swipe",
    right_swipe: "Right swipe",
    sender_pictures: "Sender pictures",
    tooltips: "Tooltips",
  };
  const shown =
    value === undefined
      ? "reset to default"
      : typeof value === "boolean"
        ? value
          ? "on"
          : "off"
        : String(value);
  return `${labels[key] ?? key}: ${shown}`;
}

interface Revisions {
  revisions: Record<BrowserSettingKey, number>;
  receipt: ApplyReceipt | null;
}
/// Wraps the plain settings store: every write bumps the revision of each
/// portable field whose value changed, in the same synchronous storage step.
export class ProfileSettingsStore implements SettingsStore {
  constructor(
    private inner: SettingsStore,
    private key: string,
    private storage: Pick<Storage, "getItem" | "setItem"> = localStorage,
  ) {}
  read(): Preferences {
    return this.inner.read();
  }
  write(value: Preferences): void {
    const before = portableValues(this.inner.read());
    const after = portableValues(value);
    const state = this.state();
    for (const key of BROWSER_SETTINGS)
      if (before[key] !== after[key]) state.revisions[key]++;
    this.inner.write(value);
    this.persist(state);
  }
  /// Explicit local intent even when the value is unchanged, used by
  /// enrollment to mark a kept field as newer local intent.
  bump(keys: BrowserSettingKey[]) {
    const state = this.state();
    for (const key of keys) state.revisions[key]++;
    this.persist(state);
  }
  capture(): PortableValues {
    return {
      values: portableValues(this.inner.read()),
      revisions: { ...this.state().revisions },
    };
  }
  receipt(): ApplyReceipt | null {
    return this.state().receipt;
  }
  saveReceipt(receipt: ApplyReceipt | null) {
    const state = this.state();
    state.receipt = receipt;
    this.persist(state);
  }
  private state(): Revisions {
    const fresh: Revisions = {
      revisions: Object.fromEntries(
        BROWSER_SETTINGS.map((key) => [key, 0]),
      ) as Record<BrowserSettingKey, number>,
      receipt: null,
    };
    try {
      const raw = this.storage.getItem(this.key);
      if (!raw) return fresh;
      const parsed = JSON.parse(raw) as Partial<Revisions>;
      for (const key of BROWSER_SETTINGS) {
        const value = parsed.revisions?.[key];
        if (Number.isInteger(value) && (value as number) >= 0)
          fresh.revisions[key] = value as number;
      }
      fresh.receipt =
        parsed.receipt && typeof parsed.receipt.id === "string"
          ? parsed.receipt
          : null;
      return fresh;
    } catch {
      return fresh;
    }
  }
  private persist(state: Revisions) {
    this.storage.setItem(this.key, JSON.stringify(state));
  }
}

/// The enrollment device: applies reviewed portable values only where the
/// local field is unchanged since the review, keeping newer local edits.
export class ProfilePreferenceDevice {
  constructor(
    private settings: ProfileSettingsStore,
    private workspace: {
      preferences: Preferences;
      savePreferences(value: Preferences): void;
    },
  ) {}
  capture(): PortableValues {
    return this.settings.capture();
  }
  apply(request: ApplyRequest): ApplyReceipt {
    const existing = this.settings.receipt();
    // A lost reply retries the identical request and gets the same receipt.
    if (existing && existing.id === request.id) return existing;
    if (existing)
      throw new Error(
        "Another profile application is still awaiting acknowledgment. Finish or cancel it first.",
      );
    const current = this.settings.capture();
    let next = this.workspace.preferences;
    const applied: BrowserSettingKey[] = [];
    const kept: BrowserSettingKey[] = [];
    for (const key of BROWSER_SETTINGS) {
      if (!(key in request.changes)) continue;
      const unchanged =
        current.values[key] === request.baseline.values[key] &&
        current.revisions[key] === request.baseline.revisions[key];
      const converted = unchanged
        ? applyPortable(next, key, request.changes[key])
        : null;
      if (converted) {
        next = converted;
        applied.push(key);
      } else kept.push(key);
    }
    if (applied.length) this.workspace.savePreferences(next);
    // Kept fields carry newer local intent forward for later publication.
    if (kept.length) this.settings.bump(kept);
    const receipt: ApplyReceipt = {
      id: request.id,
      applied,
      kept,
      revisions: { ...this.settings.capture().revisions },
    };
    this.settings.saveReceipt(receipt);
    return receipt;
  }
  acknowledge(id: string) {
    if (this.settings.receipt()?.id === id) this.settings.saveReceipt(null);
  }
}
