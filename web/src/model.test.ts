import { describe, it, expect } from "vitest";
import fixture from "../../shared/preview.json";
import {
  Workspace,
  MutationFailure,
  defaults,
  type Repository,
  type Fields,
  type Preferences,
  type Draft,
  type CalendarEntry,
} from "./model";
class Settings {
  value = structuredClone(defaults);
  fail = false;
  read() {
    return this.value;
  }
  write(p: Preferences) {
    if (this.fail) throw Error("quota");
    this.value = structuredClone(p);
  }
}
class Controlled implements Repository {
  preview = true;
  cached = structuredClone(fixture.messages);
  events = structuredClone(fixture.events);
  jobs: { resolve: () => void; reject: () => void }[] = [];
  async refresh() {
    return structuredClone(this.cached);
  }
  async mutate(id: string, fields: Fields) {
    await new Promise<void>((resolve, reject) =>
      this.jobs.push({ resolve, reject: () => reject(Error("rejected")) }),
    );
    this.cached = this.cached.map((m) =>
      m.id === id ? { ...m, ...fields } : m,
    );
  }
  async saveDraft(_d: Draft) {}
  async send() {
    throw Error("Cannot send preview");
  }
  async saveEvent(_e: CalendarEntry) {}
}
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
describe("optimistic provider contract", () => {
  it("removes archive immediately and projects into destination", async () => {
    const repo = new Controlled();
    const w = new Workspace(repo, new Settings());
    const job = w.action("1", "archive");
    expect(w.visible.some((m) => m.id === "1")).toBe(false);
    expect(w.pending).toBe(1);
    w.navigate("Archive");
    expect(w.visible.some((m) => m.id === "1")).toBe(true);
    await tick();
    repo.jobs[0].resolve();
    await job;
    expect(repo.cached[0].folder).toBe("Archive");
  });
  it("rollback preserves navigation", async () => {
    const repo = new Controlled();
    const w = new Workspace(repo, new Settings());
    const job = w.action("1", "archive");
    w.navigate("Sent");
    await tick();
    repo.jobs[0].reject();
    await job;
    expect(w.folder).toBe("Sent");
    expect(w.mail[0].folder).toBe("Inbox");
    expect(w.error).toContain("restored");
  });
  it("older failed input cannot clobber newer field intent", async () => {
    const repo = new Controlled();
    const w = new Workspace(repo, new Settings());
    const a = w.action("1", "star");
    const b = w.action("1", "star");
    const c = w.action("1", "read");
    await tick();
    repo.jobs[0].reject();
    await a;
    await tick();
    repo.jobs[1].resolve();
    await b;
    await tick();
    repo.jobs[2].resolve();
    await c;
    expect(w.mail[0].starred).toBe(false);
    expect(w.mail[0].unread).toBe(false);
    expect(repo.cached[0].unread).toBe(false);
  });
  it("all failures restore last confirmed state", async () => {
    const repo = new Controlled();
    const w = new Workspace(repo, new Settings());
    const jobs = [
      w.action("1", "star"),
      w.action("1", "star"),
      w.action("1", "star"),
    ];
    for (let i = 0; i < 3; i++) {
      await tick();
      repo.jobs[i].reject();
      await jobs[i];
    }
    expect(w.mail[0].starred).toBe(false);
  });
  it("undo is ordered behind pending mutation", async () => {
    const repo = new Controlled();
    const w = new Workspace(repo, new Settings());
    const a = w.action("1", "archive");
    w.undo!();
    expect(w.mail[0].folder).toBe("Inbox");
    await tick();
    repo.jobs[0].resolve();
    await a;
    await tick();
    repo.jobs[1].resolve();
    await tick();
    expect(repo.cached[0].folder).toBe("Inbox");
  });
  it("refresh cannot replace newer mutation", async () => {
    const repo = new Controlled();
    const w = new Workspace(repo, new Settings());
    const a = w.action("1", "archive");
    await w.refresh();
    expect(w.mail[0].folder).toBe("Archive");
    await tick();
    repo.jobs[0].resolve();
    await a;
  });
  it("failed preferences retain current edit for retry", () => {
    const settings = new Settings();
    const w = new Workspace(new Controlled(), settings);
    settings.fail = true;
    w.savePreferences({ ...w.preferences, appearance: "dark" });
    expect(w.error).toContain("Retry");
    settings.fail = false;
    w.retry!();
    expect(settings.value.appearance).toBe("dark");
  });
  it("search filters sender/body and excludes other folders", () => {
    const w = new Workspace(new Controlled(), new Settings());
    w.search("Morgan sketches");
    expect(w.visible.map((m) => m.id)).toEqual(["1"]);
    w.search("zzzzz");
    expect(w.visible).toEqual([]);
  });
});

describe("server acknowledgments and recovery", () => {
  it("keeps a committed server action visible when the following cache save fails", async () => {
    const repo = new Controlled();
    repo.mutate = async () => {
      throw new MutationFailure(
        "Server committed; browser storage failed. Refresh.",
        true,
      );
    };
    const w = new Workspace(repo, new Settings());
    await w.action("1", "archive");
    expect(w.mail.find((m) => m.id === "1")?.folder).toBe("Archive");
    expect(w.error).toContain("Server committed");
    expect(w.undo).toBeNull();
  });
  it("offers a refresh rather than repeating an unconfirmed mutation", async () => {
    const repo = new Controlled();
    let calls = 0;
    let refreshes = 0;
    repo.mutate = async () => {
      calls++;
      throw new MutationFailure("Move not confirmed. Refresh.");
    };
    repo.refresh = async () => {
      refreshes++;
      return structuredClone(repo.cached);
    };
    const w = new Workspace(repo, new Settings());
    await w.action("1", "archive");
    w.retry?.();
    await tick();
    expect(calls).toBe(1);
    expect(refreshes).toBe(1);
  });
});

describe("local Sent insertion", () => {
  it("preserves pending actions and invalidates an older refresh without replacing existing records", async () => {
    const repo = new Controlled();
    const w = new Workspace(repo, new Settings());
    let completeRefresh!: (mail: typeof repo.cached) => void;
    repo.refresh = () =>
      new Promise((resolve) => {
        completeRefresh = resolve;
      });
    const refresh = w.refresh();
    const archive = w.action("1", "archive");
    const sent = {
      ...repo.cached[0],
      id: "new-sent",
      folder: "Sent",
      body: "Immutable Sent copy",
    };
    w.addCachedMail([...repo.cached, sent]);
    expect(w.mail.find((m) => m.id === "1")?.folder).toBe("Archive");
    expect(w.mail.find((m) => m.id === sent.id)?.body).toBe(sent.body);
    await tick();
    repo.jobs[0].resolve();
    await archive;
    completeRefresh(structuredClone(fixture.messages));
    await refresh;
    expect(w.mail.find((m) => m.id === "1")?.folder).toBe("Archive");
    expect(w.mail.filter((m) => m.id === sent.id)).toHaveLength(1);
    w.addCachedMail([sent]);
    expect(w.mail.filter((m) => m.id === sent.id)).toHaveLength(1);
  });
});
