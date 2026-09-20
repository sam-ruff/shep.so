import { expect, test } from "vitest";
import { defaults, Workspace, UnconnectedRepository } from "./model";
import {
  ProfileSettingsStore,
  ProfilePreferenceDevice,
} from "./profile_settings";

test("pending change and revert retains local intent and blocks profile capture/application until committed", () => {
  let fail = false;
  const data = new Map<string, string>();
  const storage = {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => {
      if (fail) throw Error("Fixture full");
      data.set(key, value);
    },
  };
  const settings = new ProfileSettingsStore(
    { read: () => structuredClone(defaults), write: () => {} },
    "settings",
    storage,
  );
  settings.write(defaults);
  const workspace = new Workspace(new UnconnectedRepository(), settings);
  const device = new ProfilePreferenceDevice(settings, workspace);
  const baseline = device.capture();
  fail = true;
  workspace.savePreferences({ ...workspace.preferences, appearance: "dark" });
  workspace.savePreferences({
    ...workspace.preferences,
    appearance: defaults.appearance,
  });
  expect(() => device.capture()).toThrow("pending preference");
  expect(() =>
    device.apply({ id: "remote", baseline, changes: { appearance: "Light" } }),
  ).toThrow("pending preference");
  expect(settings.receipt()).toBeNull();
  expect(settings.capture()).toEqual(baseline);
  fail = false;
  workspace.retryPreferences();
  expect(device.capture().revisions.appearance).toBe(
    baseline.revisions.appearance + 1,
  );
  expect(
    device.apply({ id: "remote", baseline, changes: { appearance: "Light" } })
      .kept,
  ).toContain("appearance");
  expect(workspace.preferences.appearance).toBe(defaults.appearance);
});

test("preference values and ownership commit together even if the legacy mirror fails", () => {
  const data = new Map<string, string>();
  const storage = {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => {
      data.set(key, value);
    },
  };
  const legacy = {
    read: () => structuredClone(defaults),
    write: () => {
      throw Error("Fixture mirror full");
    },
  };
  const settings = new ProfileSettingsStore(legacy, "settings", storage);
  settings.write({ ...defaults, appearance: "dark" });
  const reopened = new ProfileSettingsStore(legacy, "settings", storage);
  expect(reopened.read().appearance).toBe("dark");
  expect(reopened.capture()).toMatchObject({
    values: { appearance: "Dark" },
    revisions: { appearance: 1 },
  });
});

test("failed admission preserves both baseline values and revisions and cannot acknowledge enrollment", () => {
  let fail = false;
  const data = new Map<string, string>();
  const storage = {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => {
      if (fail) throw Error("Fixture full");
      data.set(key, value);
    },
  };
  let mirrored = structuredClone(defaults);
  const settings = new ProfileSettingsStore(
    {
      read: () => mirrored,
      write: (value) => {
        mirrored = value;
      },
    },
    "settings",
    storage,
  );
  settings.write(defaults);
  const baseline = settings.capture();
  fail = true;
  expect(() => settings.write({ ...defaults, appearance: "dark" })).toThrow(
    "full",
  );
  expect(settings.capture()).toEqual(baseline);
  expect(settings.read().appearance).toBe(defaults.appearance);
  const device = new ProfilePreferenceDevice(settings, {
    preferences: structuredClone(defaults),
    savePreferences(value) {
      try {
        settings.write(value);
      } catch {
        /* Match the UI retaining its error. */
      }
    },
  });
  expect(() =>
    device.apply({ id: "review", baseline, changes: { appearance: "Dark" } }),
  ).toThrow("admitted locally");
  expect(settings.receipt()).toBeNull();
});
