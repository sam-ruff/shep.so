import { afterEach, expect, test, vi } from "vitest";
import { BrowserSettings, defaults } from "./model";
import { applyPortable, defaultPortable, portableValues } from "./profile_settings";

afterEach(() => vi.unstubAllGlobals());

test("browser device settings retain explicit false and default old absent values", () => {
  const values = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
  });
  const settings = new BrowserSettings("reply-test");
  expect(settings.read().replyIncludeOriginal).toBe(true);
  settings.write({ ...defaults, replyIncludeOriginal: false });
  expect(new BrowserSettings("reply-test").read().replyIncludeOriginal).toBe(false);
  values.set("reply-test", JSON.stringify({ version: 1 }));
  expect(settings.read().replyIncludeOriginal).toBe(true);
});

test("portable reply default distinguishes false, reset and invalid values", () => {
  const falseValue = applyPortable(defaults, "reply_include_original", false);
  expect(falseValue?.replyIncludeOriginal).toBe(false);
  expect(portableValues(falseValue ?? defaults).reply_include_original).toBe(false);
  expect(defaultPortable("reply_include_original")).toBe(true);
  expect(applyPortable(defaults, "reply_include_original", "false")).toBeNull();
});
