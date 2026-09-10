import "fake-indexeddb/auto";
import { beforeEach, describe, expect, test } from "vitest";
import { ProfileStore } from "./profile_store";
import { InlineHistoryPort, ProfileJournal } from "./profile_history";
import { ProfileDiscovery } from "./profile_discovery";
import { ProfilePublication } from "./profile_publication";
import { ProfileEnrollment, type EnrollmentDevice } from "./profile_enrollment";
import {
  ProfilePreferenceDevice,
  ProfileSettingsStore,
  type ApplyReceipt,
} from "./profile_settings";
import { defaults, type Preferences, type SettingsStore } from "./model";
import type { Account } from "./provider";
import {
  FakeGoogleApi,
  IDENTITY,
  NAMESPACE,
  PRINCIPAL,
  publishedProfile,
  uuid,
  wasmModule,
} from "./testing/profile_fixtures";

class MemorySettings implements SettingsStore {
  value = structuredClone(defaults);
  read() {
    return structuredClone(this.value);
  }
  write(p: Preferences) {
    this.value = structuredClone(p);
  }
}
class MemoryStorage {
  map = new Map<string, string>();
  getItem(key: string) {
    return this.map.get(key) ?? null;
  }
  setItem(key: string, value: string) {
    this.map.set(key, value);
  }
}
function device(settings: ProfileSettingsStore) {
  const workspace = {
    preferences: settings.read(),
    savePreferences(value: Preferences) {
      workspace.preferences = value;
      settings.write(value);
    },
  };
  return {
    workspace,
    device: new ProfilePreferenceDevice(settings, workspace),
  };
}
function account(id: string, email: string): Account {
  return {
    id,
    name: email,
    email,
    protocol: "Imap",
    host: "imap.example.test",
    port: 993,
    username: email,
    incoming_security: "Tls",
    incoming_auth: "Password",
    smtp_host: "smtp.example.test",
    smtp_port: 465,
    smtp_username: email,
    smtp_security: "Tls",
    smtp_auth: "Automatic",
    smtp_separate_password: false,
    sent_copy: "Automatic",
    sent_folder: "",
  };
}
let serial = 0;
let identity = IDENTITY;
const scope = { namespace: NAMESPACE, principal: PRINCIPAL };
beforeEach(() => {
  identity = String.fromCharCode(65 + (serial++ % 26)).repeat(43);
});
async function harness(api = new FakeGoogleApi()) {
  const store = await ProfileStore.open(identity);
  const port = new InlineHistoryPort(wasmModule());
  const discovery = new ProfileDiscovery(store, api, port, scope);
  await discovery.load();
  return { store, port, discovery, api };
}

describe("profile store and history journal", () => {
  test("records restore an identical journal and staged edits retire with their record", async () => {
    const { store, port } = await harness();
    const binding = { ...scope, profile: uuid(1), generation: uuid(1, "9") };
    const key = "local:test";
    const journal = await ProfileJournal.open(
      port,
      key,
      binding,
      await store.deviceFor(key),
      [],
    );
    const [root, content, done] = publishedProfile(uuid(1), 10, "Laptop");
    for (const op of [root, content, done]) {
      await journal.execute(
        { kind: "import", record: JSON.stringify(op) },
        "state",
      );
      await store.saveRecord(key, await journal.record(op.operation));
    }
    const edit = {
      operation: uuid(50),
      expected_revision: (await journal.state()).revision,
      changes: [{ kind: "profile_name" as const, name: "Renamed" }],
    };
    await store.stageEdit(key, edit);
    expect(await store.stagedEdits(key)).toHaveLength(1);
    await journal.execute({ kind: "edit", edit }, "state");
    await store.saveRecord(key, await journal.record(edit.operation));
    expect(await store.stagedEdits(key)).toHaveLength(0);
    const records = await store.records(key);
    expect(records.map((r) => r.seq)).toEqual([1, 2, 3, 4]);
    expect(records[3].request).toBeDefined();
    const restored = await ProfileJournal.open(
      port,
      "restored",
      binding,
      await store.deviceFor(key),
      records,
    );
    expect((await restored.overview()).name).toBe("Renamed");
    expect(
      (await restored.execute({ kind: "next_upload" }, "upload"))?.operation,
    ).toBe(uuid(50));
    // Another device identity cannot own these local records.
    await expect(
      ProfileJournal.open(port, "other", binding, uuid(77, "d"), records),
    ).rejects.toMatchObject({ kind: "binding" });
    // Cleanup is marked durably before clearing every store.
    await store.markCleanup();
    expect(await store.cleanupPending()).toBe(true);
    await store.cleanup();
    expect(await store.records(key)).toEqual([]);
    expect(await store.cleanupPending()).toBe(false);
  });
  test("the inline port reports busy-free bounded errors with stable kinds", async () => {
    const { port } = await harness();
    const reply = await port.request({
      kind: "execute",
      key: "missing",
      command: { kind: "state" },
    });
    expect(reply).toMatchObject({ status: "error", kind: "stopped" });
    const invalid = await port.request({
      kind: "validate",
      record: "not json",
    });
    expect(invalid).toMatchObject({ status: "error", kind: "invalid" });
  });
});

describe("discovery", () => {
  test("verifies files, imports originals, pages summaries and never treats an incomplete listing as empty", async () => {
    const api = new FakeGoogleApi();
    for (let i = 0; i < 52; i++)
      api.add(publishedProfile(uuid(100 + i), 1000 + i * 3, `Profile ${i}`)[0]);
    const [root, content, done] = publishedProfile(uuid(1), 10, "Laptop");
    for (const op of [root, content, done]) api.add(op);
    api.incompleteOnce = true;
    const { discovery, store } = await harness(api);
    await discovery.find();
    expect(discovery.state.failed).toBe(true);
    expect(discovery.state.error).toMatch(/incomplete listing/);
    expect(discovery.complete).toBe(false);
    expect(discovery.page).toEqual([]);
    // Retry continues from the saved step with the same revision.
    const revision = discovery.state.revision;
    await discovery.retry();
    expect(discovery.state.revision).toBe(revision);
    expect(discovery.complete).toBe(true);
    expect(discovery.state.scanned).toBe(55);
    expect(discovery.page).toHaveLength(50);
    const laptop = discovery.page.find((p) => p.name === "Laptop")!;
    expect(laptop).toMatchObject({
      accounts: 1,
      settings: 3,
      initialized: true,
      files: 3,
      conflicts: 0,
    });
    await discovery.nextPage();
    expect(discovery.page).toHaveLength(3);
    const preparing = discovery.page.find((p) => p.profile === uuid(151))!;
    expect(preparing.initialized).toBe(false);
    // A reload restores the saved catalog and summaries without Google.
    const again = new ProfileDiscovery(
      store,
      api,
      new InlineHistoryPort(wasmModule()),
      scope,
    );
    await again.load();
    expect(again.complete).toBe(true);
    expect(again.page).toHaveLength(50);
    // A rescan finding a known file missing is an explicit failure, not empty.
    api.files.delete("file-000c");
    await again.find();
    expect(again.state.failed).toBe(true);
    expect(again.state.error).toMatch(/no longer listed/);
    expect(await store.count("catalog")).toBe(55);
  });
  test("changed identity, altered media and pause are explicit", async () => {
    const api = new FakeGoogleApi();
    const [root, content, done] = publishedProfile(uuid(1), 10, "Laptop");
    for (const op of [root, content, done]) api.add(op);
    const { discovery } = await harness(api);
    await discovery.find();
    expect(discovery.complete).toBe(true);
    api.files.get("file-000b")!.media = api.files
      .get("file-000b")!
      .media.replace("Laptop", "Tamper");
    await discovery.find();
    expect(discovery.state.error).toMatch(/digest/);
    api.files.get("file-000b")!.media = api.files
      .get("file-000b")!
      .media.replace("Tamper", "Laptop");
    api.files.get("file-000b")!.appProperties.shepOperation = uuid(99);
    await discovery.find();
    expect(discovery.state.error).toMatch(
      /name does not match|changed identity/,
    );
    api.files.get("file-000b")!.appProperties.shepOperation = content.operation;
    // Pause finishes the accepted step and resumes from saved state.
    const paused = discovery.find();
    discovery.pause();
    await paused;
    expect(discovery.complete).toBe(false);
    await discovery.resume();
    expect(discovery.complete).toBe(true);
    // Replayed changes that add a file are verified too.
    const extra = publishedProfile(uuid(2), 20, "Phone")[0];
    api.add(extra);
    api.changes.push({
      fileId: `file-${extra.operation.slice(-4)}`,
      removed: false,
    });
    await discovery.find();
    expect(discovery.complete).toBe(true);
    expect(discovery.page.map((p) => p.name ?? "unnamed")).toContain("unnamed");
  });
});

describe("publication", () => {
  test("freezes the review, stages exact edits, survives a lost upload reply and pause, and rejects a changed review", async () => {
    const api = new FakeGoogleApi();
    const { discovery, store, port } = await harness(api);
    await discovery.find();
    const settings = new ProfileSettingsStore(
      new MemorySettings(),
      "prefs",
      new MemoryStorage(),
    );
    const { device: dev } = device(settings);
    const accounts = [
      account("work", "work@example.test"),
      account(uuid(3), "home@example.test"),
    ];
    const pub = new ProfilePublication(store, api, port, discovery);
    await pub.load();
    await pub.prepare("Browser profile", accounts, dev.capture());
    expect(pub.review?.accounts).toHaveLength(2);
    expect(pub.review?.accounts[1].shared).toBe(uuid(3));
    expect(pub.review?.accounts[0].shared).not.toBe("work");
    await pub.chooseSetting("preview_lines", false);
    // A changed setup invalidates the frozen review.
    const changed = [...accounts, account(uuid(4), "third@example.test")];
    await expect(pub.approve(changed, dev.capture())).rejects.toThrow(
      /changed since this review/,
    );
    api.loseCreateReply = true;
    api.failNext.metadata = 0;
    await pub.approve(accounts, dev.capture());
    expect(pub.review?.failed).toBe(true);
    expect(pub.review?.error).toMatch(/lost the upload reply/);
    expect(pub.review?.staged).toBe(3);
    const created = api.calls.filter((c) => c === "create").length;
    // Retry verifies the reserved file instead of creating it again.
    await pub.retry();
    expect(pub.review?.phase).toBe("complete");
    expect(pub.review?.uploaded).toBe(3);
    expect(api.calls.filter((c) => c === "create").length).toBe(created + 2);
    expect(api.files.size).toBe(3);
    const catalog = await store.range("catalog", "", "￿", 100);
    expect(catalog).toHaveLength(3);
    expect(discovery.page[0]).toMatchObject({
      name: "Browser profile",
      accounts: 2,
      settings: 3,
      initialized: true,
    });
    // Reopened, the same review and receipts are visible.
    const again = new ProfilePublication(store, api, port, discovery);
    await again.load();
    expect(again.review?.uploaded).toBe(3);
    await again.cancel();
    expect(again.review).toBeNull();
    // Discovery afterwards recognises the own uploads as known files.
    await discovery.find();
    expect(discovery.complete).toBe(true);
    expect(discovery.state.scanned).toBe(3);
  });
  test("publication pauses after the accepted step and requires completed discovery", async () => {
    const api = new FakeGoogleApi();
    const { discovery, store, port } = await harness(api);
    const settings = new ProfileSettingsStore(
      new MemorySettings(),
      "prefs",
      new MemoryStorage(),
    );
    const { device: dev } = device(settings);
    const pub = new ProfilePublication(store, api, port, discovery);
    await expect(pub.prepare("Early", [], dev.capture())).rejects.toThrow(
      /Finish discovery/,
    );
    await discovery.find();
    await pub.prepare(
      "Paused",
      [account(uuid(5), "a@example.test")],
      dev.capture(),
    );
    // Pause while the first step is accepted: it finishes, no next step starts.
    let pausedOnce = false;
    pub.addEventListener("change", () => {
      if (pausedOnce || !pub.busy || pub.review?.staged !== 1) return;
      pausedOnce = true;
      pub.pause();
    });
    await pub.approve([account(uuid(5), "a@example.test")], dev.capture());
    expect(pub.review?.phase).toBe("staging");
    expect(pub.review?.staged).toBe(1);
    expect(pub.busy).toBe(false);
    await pub.resume();
    expect(pub.review?.phase).toBe("complete");
  });
});

describe("enrollment", () => {
  function enrollmentDevice(settings: ProfileSettingsStore) {
    const { workspace, device: dev } = device(settings);
    const accounts: Account[] = [];
    const imported: Account[] = [];
    const d: EnrollmentDevice = {
      accounts: () => accounts,
      importAccount: async (a) => {
        imported.push(structuredClone(a));
        const index = accounts.findIndex((x) => x.id === a.id);
        if (index >= 0) accounts[index] = a;
        else accounts.push(a);
      },
      capture: () => dev.capture(),
      apply: (r) => dev.apply(r),
      acknowledge: (id) => dev.acknowledge(id),
    };
    return { d, accounts, imported, workspace, settings };
  }
  test("copies originals, reviews rows, applies accounts without passwords and keeps newer local settings", async () => {
    const api = new FakeGoogleApi();
    const [root, content, done] = publishedProfile(uuid(1), 10, "Laptop");
    for (const op of [root, content, done]) api.add(op);
    const { discovery, store, port } = await harness(api);
    await discovery.find();
    const settings = new ProfileSettingsStore(
      new MemorySettings(),
      "prefs",
      new MemoryStorage(),
    );
    const { d, accounts, imported, workspace } = enrollmentDevice(settings);
    const enrol = new ProfileEnrollment(store, port, discovery, d);
    await enrol.load();
    await enrol.prepare(discovery.page[0]);
    expect(enrol.review?.phase).toBe("review");
    expect(enrol.review?.copied).toBe(3);
    const rows = enrol.rows(0);
    expect(rows.map((r) => r.kind)).toEqual([
      "account",
      "name",
      "setting",
      "setting",
      "setting",
    ]);
    expect(rows.find((r) => r.key === "tooltips")).toMatchObject({
      available: false,
      selected: false,
    });
    expect(rows[0].reason).toMatch(/reconnect to activate/);
    // The user edits preview lines locally before applying: the local value wins.
    workspace.savePreferences({ ...workspace.preferences, previewLines: 1 });
    await enrol.choose(1, false);
    await enrol.approve(true, true);
    expect(enrol.review?.phase).toBe("complete");
    expect(imported).toHaveLength(1);
    expect(imported[0].name).toBe(imported[0].email);
    expect(accounts[0].id).not.toBe(uuid(10, "5"));
    expect(workspace.preferences.appearance).toBe("dark");
    expect(workspace.preferences.previewLines).toBe(1);
    expect(enrol.review?.settingsReceipt).toMatchObject({
      applied: ["appearance"],
      kept: ["preview_lines"],
    });
    expect(enrol.review?.settingsReceipt?.revisions.preview_lines).toBe(2);
    const mapping = await store.get<{ shared: string; reconnect: boolean }>(
      "mappings",
      accounts[0].id,
    );
    expect(mapping).toMatchObject({ shared: uuid(10, "5"), reconnect: true });
    // A second enrollment of the same profile keeps the mapped account identity.
    await enrol.cancel();
    await enrol.prepare(discovery.page[0]);
    expect(enrol.rows(0)[0].local?.id).toBe(accounts[0].id);
    expect(enrol.rows(0)[0].reason).toMatch(/Keeps this browser/);
  });
  test("a lost application reply retries with the same identities and receipt, and connection changes are separate accounts", async () => {
    const api = new FakeGoogleApi();
    const [root, content, done] = publishedProfile(uuid(1), 10, "Laptop");
    for (const op of [root, content, done]) api.add(op);
    const { discovery, store, port } = await harness(api);
    await discovery.find();
    const settings = new ProfileSettingsStore(
      new MemorySettings(),
      "prefs",
      new MemoryStorage(),
    );
    const { d, accounts, imported } = enrollmentDevice(settings);
    let failOnce = true;
    const flaky: EnrollmentDevice = {
      ...d,
      importAccount: async (a) => {
        await d.importAccount(a);
        if (failOnce) {
          failOnce = false;
          throw new Error("Lost account reply.");
        }
      },
    };
    const enrol = new ProfileEnrollment(store, port, discovery, flaky);
    await enrol.load();
    await enrol.prepare(discovery.page[0]);
    await enrol.approve(true, true);
    expect(enrol.review?.failed).toBe(true);
    expect(enrol.review?.error).toMatch(/Lost account reply/);
    const reserved = enrol.review!.accountReceipts[uuid(10, "5")].local;
    // Reload: the reserved identity and phase come back from the store.
    const again = new ProfileEnrollment(store, port, discovery, flaky);
    await again.load();
    expect(again.review?.phase).toBe("applying");
    await again.retry();
    expect(again.review?.phase).toBe("complete");
    expect(imported.map((a) => a.id)).toEqual([reserved, reserved]);
    expect(accounts).toHaveLength(1);
    const receipt = again.review!.settingsReceipt as ApplyReceipt;
    expect(receipt.id).toBe(again.review!.id);
    // A remotely changed connection is offered as a separate, unselected account.
    accounts[0] = { ...accounts[0], host: "old.example.test" };
    await again.cancel();
    await again.prepare(discovery.page[0]);
    expect(again.rows(0)[0]).toMatchObject({
      selected: false,
      local: undefined,
    });
    expect(again.rows(0)[0].reason).toMatch(/separate account/);
  });
  test("enrollment refuses incomplete profiles and discovery", async () => {
    const api = new FakeGoogleApi();
    api.add(publishedProfile(uuid(1), 10, "Only root")[0]);
    const { discovery, store, port } = await harness(api);
    const settings = new ProfileSettingsStore(
      new MemorySettings(),
      "prefs",
      new MemoryStorage(),
    );
    const enrol = new ProfileEnrollment(
      store,
      port,
      discovery,
      enrollmentDevice(settings).d,
    );
    await expect(
      enrol.prepare({
        profile: uuid(1),
        generation: uuid(1, "9"),
        name: null,
        nameConflict: false,
        accounts: 0,
        settings: 0,
        initialized: true,
        waiting: 0,
        ready: 0,
        conflicts: 0,
        removed: false,
        files: 1,
        revision: 1,
      }),
    ).rejects.toThrow(/Finish discovery/);
    await discovery.find();
    await expect(enrol.prepare(discovery.page[0])).rejects.toThrow(
      /not complete/,
    );
  });
});

describe("preference device", () => {
  test("revisions advance on change and revert, receipts are reused after a lost reply and newer edits are kept", () => {
    const settings = new ProfileSettingsStore(
      new MemorySettings(),
      "prefs",
      new MemoryStorage(),
    );
    const { workspace, device: dev } = device(settings);
    const baseline = dev.capture();
    workspace.savePreferences({ ...workspace.preferences, appearance: "dark" });
    workspace.savePreferences({
      ...workspace.preferences,
      appearance: "system",
    });
    expect(dev.capture().values.appearance).toBe("System");
    expect(dev.capture().revisions.appearance).toBe(2);
    const receipt = dev.apply({
      id: "r1",
      baseline,
      changes: {
        appearance: "Light",
        sender_pictures: false,
        reply_display: "LatestOnly",
      },
    });
    expect(receipt).toMatchObject({
      applied: ["sender_pictures", "reply_display"],
      kept: ["appearance"],
    });
    expect(workspace.preferences).toMatchObject({
      appearance: "system",
      avatars: false,
      quoteMode: "Latest only",
    });
    expect(receipt.revisions.appearance).toBe(3);
    // The identical retry returns the frozen receipt, not a fresh snapshot.
    workspace.savePreferences({ ...workspace.preferences, avatars: true });
    expect(
      dev.apply({ id: "r1", baseline, changes: { appearance: "Light" } }),
    ).toEqual(receipt);
    expect(() => dev.apply({ id: "r2", baseline, changes: {} })).toThrow(
      /awaiting acknowledgment/,
    );
    dev.acknowledge("r1");
    expect(
      dev.apply({
        id: "r2",
        baseline: dev.capture(),
        changes: { preview_lines: 9 },
      }).kept,
    ).toEqual(["preview_lines"]);
  });
});
