import { expect, test } from "vitest";
import { defaults } from "./model";
import { ProfileSettingsStore, ProfilePreferenceDevice } from "./profile_settings";

test("preference values and ownership commit together even if the legacy mirror fails", () => {
  const data = new Map<string, string>();
  const storage = { getItem: (key: string) => data.get(key) ?? null, setItem: (key: string, value: string) => { data.set(key, value); } };
  const legacy = { read: () => structuredClone(defaults), write: () => { throw Error("Fixture mirror full"); } };
  const settings = new ProfileSettingsStore(legacy, "settings", storage);
  settings.write({ ...defaults, appearance: "dark" });
  const reopened = new ProfileSettingsStore(legacy, "settings", storage);
  expect(reopened.read().appearance).toBe("dark");
  expect(reopened.capture()).toMatchObject({ values: { appearance: "Dark" }, revisions: { appearance: 1 } });
});

test("failed admission preserves both baseline values and revisions and cannot acknowledge enrollment", () => {
  let fail = false;
  const data = new Map<string, string>();
  const storage = { getItem: (key: string) => data.get(key) ?? null, setItem: (key: string, value: string) => { if (fail) throw Error("Fixture full"); data.set(key, value); } };
  let mirrored = structuredClone(defaults);
  const settings = new ProfileSettingsStore({ read: () => mirrored, write: value => { mirrored = value; } }, "settings", storage);
  settings.write(defaults);
  const baseline = settings.capture();
  fail = true;
  expect(() => settings.write({ ...defaults, appearance: "dark" })).toThrow("full");
  expect(settings.capture()).toEqual(baseline);
  expect(settings.read().appearance).toBe(defaults.appearance);
  const device = new ProfilePreferenceDevice(settings, {
    preferences: structuredClone(defaults),
    savePreferences(value) { try { settings.write(value); } catch { /* Match the UI retaining its error. */ } },
  });
  expect(() => device.apply({ id: "review", baseline, changes: { appearance: "Dark" } })).toThrow("admitted locally");
  expect(settings.receipt()).toBeNull();
});
