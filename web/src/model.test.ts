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
    await tick();
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

it("account removal clears a retained reader and a late failure cannot restore its message", async () => {
  const repo = new Controlled();
  repo.cached = repo.cached.map((m) => ({ ...m, accountId: "removed" }));
  const w = new Workspace(repo, new Settings());
  w.selected = "1";
  const pending = w.action("1", "star");
  await tick();
  const draft = {
    id: "deleted-draft",
    accountId: "removed",
    to: "to@example.test",
    cc: "",
    bcc: "",
    subject: "Private draft",
    body: "Keep until removal",
  };
  w.rememberDraft(draft);
  w.accountRemoved("removed");
  expect(w.drafts.size).toBe(0);
  expect(() => w.rememberDraft(draft)).toThrow("account was removed");
  expect(w.readerMessage).toBeNull();
  expect(w.mail).toEqual([]);
  repo.jobs[0].reject();
  await pending;
  expect(w.mail).toEqual([]);
  expect(w.readerMessage).toBeNull();
  expect(w.pending).toBe(0);
});

describe("read-on-leave", () => {
  it("only a deliberate visit arms reading; refresh and initial selection do not", async () => {
    const repo = new Controlled(),
      w = new Workspace(repo, new Settings());
    w.selected = "1";
    await w.refresh();
    w.navigate("Inbox");
    await tick();
    expect(repo.jobs).toHaveLength(0);
    w.beginReading("1");
    expect(w.readerMessage?.unread).toBe(true);
    await w.refresh();
    await tick();
    expect(repo.jobs).toHaveLength(0);
    w.beginReading("2");
    expect(w.mail[0].unread).toBe(false);
    expect(w.selected).toBe("2");
    expect(w.notice).not.toBe("Message updated");
    await tick();
    repo.jobs[0].resolve();
    await tick();
    expect(repo.cached[0].unread).toBe(false);
  });
  it("explicit unread survives a delayed read acknowledgment and another visit", async () => {
    const repo = new Controlled(),
      w = new Workspace(repo, new Settings());
    w.beginReading("1");
    const read = w.finishReading();
    await tick();
    const unread = w.change("1", { unread: true });
    w.beginReading("2");
    repo.jobs[0].resolve();
    await read;
    await tick();
    expect(w.mail[0].unread).toBe(true);
    repo.jobs[1].resolve();
    await unread;
    expect(repo.cached[0].unread).toBe(true);
  });
  it("read failure rolls back just its flag and keeps a newer move and Undo", async () => {
    const repo = new Controlled(),
      w = new Workspace(repo, new Settings());
    w.beginReading("1");
    const move = w.action("1", "archive");
    const undo = w.undo;
    expect(w.mail[0].folder).toBe("Archive");
    expect(w.mail[0].unread).toBe(false);
    await tick();
    expect(repo.jobs).toHaveLength(1);
    repo.jobs[0].reject();
    await tick();
    expect(repo.jobs).toHaveLength(2);
    expect(w.mail[0].unread).toBe(true);
    expect(w.moves.label).toBe("Archived 1 message");
    expect(w.undo).toBe(undo);
    expect(w.error).toContain("restored");
    repo.jobs[1].resolve();
    await move;
    expect(repo.cached[0].folder).toBe("Archive");
    expect(repo.cached[0].unread).toBe(true);
  });
  it("explicit read controls cancel the visit, and automatic reads keep older Undo", async () => {
    const repo = new Controlled(),
      w = new Workspace(repo, new Settings());
    const starred = w.action("2", "star");
    await tick();
    repo.jobs[0].resolve();
    await starred;
    const undo = w.undo,
      notice = w.notice;
    w.beginReading("1");
    const read = w.finishReading();
    await tick();
    repo.jobs[1].resolve();
    await read;
    expect(w.undo).toBe(undo);
    expect(w.notice).toBe(notice);
    w.beginReading("1");
    const unread = w.change("1", { unread: true });
    await tick();
    repo.jobs[2].resolve();
    await unread;
    await w.finishReading();
    expect(repo.jobs).toHaveLength(3);
    expect(w.mail[0].unread).toBe(true);
  });
});

describe("counted moves and grouped Undo", () => {
  it("keeps the surviving move after partial failure and retries only failed restoration", async () => {
    const repo = new Controlled(),
      w = new Workspace(repo, new Settings());
    try {
      const first = w.action("1", "archive"),
        second = w.action("2", "archive");
      await tick();
      expect(w.moves.label).toBe("Archived 2 messages");
      repo.jobs[0].reject();
      await first;
      expect(w.moves.label).toBe("Archived 1 message");
      w.undo!();
      expect(w.mail.find((m) => m.id === "2")!.folder).toBe("Inbox");
      expect(w.moves.label).toBe("Restored 1 message");
      expect(repo.jobs).toHaveLength(2);
      repo.jobs[1].resolve();
      await second;
      await tick();
      repo.jobs[2].reject();
      await tick();
      expect(w.mail.find((m) => m.id === "2")!.folder).toBe("Archive");
      expect(w.undoFailures.size).toBe(1);
      expect(w.moves.label).toBeNull();
      w.retryUndos();
      expect(w.mail.find((m) => m.id === "2")!.folder).toBe("Inbox");
      await tick();
      repo.jobs[3].resolve();
      await tick();
      expect(w.undoFailures.size).toBe(0);
      expect(w.error).toBeNull();
      expect(repo.cached.find((m) => m.id === "2")!.folder).toBe("Inbox");
      expect(w.moves.label).toBe("Restored 1 message");
    } finally {
      w.dispose();
    }
  });
  it("Undo cancels a move waiting behind read without sending either direction", async () => {
    const repo = new Controlled(),
      w = new Workspace(repo, new Settings());
    try {
      w.beginReading("1");
      const move = w.action("1", "archive");
      await tick();
      expect(repo.jobs).toHaveLength(1);
      w.undo!();
      expect(w.moves.label).toBe("Restored 1 message");
      repo.jobs[0].resolve();
      await move;
      await tick();
      expect(repo.jobs).toHaveLength(1);
      expect(repo.cached[0].folder).toBe("Inbox");
      expect(repo.cached[0].unread).toBe(false);
    } finally {
      w.dispose();
    }
  });
  it("a stale Undo and late failure cannot modify a newer destination notification", async () => {
    const repo = new Controlled(),
      w = new Workspace(repo, new Settings());
    try {
      const first = w.action("1", "archive");
      await tick();
      const oldUndo = w.undo;
      const second = w.action("2", "trash");
      await tick();
      const currentUndo = w.undo;
      oldUndo!();
      expect(w.mail[1].folder).toBe("Trash");
      repo.jobs[0].reject();
      await first;
      expect(w.moves.label).toBe("Deleted 1 message");
      expect(w.undo).toBe(currentUndo);
      w.moves.dismiss();
      repo.jobs[1].resolve();
      await second;
      expect(w.moves.label).toBeNull();
      expect(w.undo).toBeNull();
    } finally {
      w.dispose();
    }
  });
});

it("an acknowledged Undo metadata warning permits refresh but never another mutation", async () => {
  class CommittedUndo extends Controlled {
    calls = 0;
    override async mutate(id: string, fields: Fields) {
      const call = ++this.calls;
      await super.mutate(id, fields);
      if (call === 2)
        throw new MutationFailure(
          "Undo acknowledged; refresh saved metadata.",
          true,
        );
    }
  }
  const repo = new CommittedUndo(),
    w = new Workspace(repo, new Settings());
  try {
    const moved = w.action("1", "archive");
    await tick();
    repo.jobs[0].resolve();
    await moved;
    w.undo!();
    await tick();
    repo.jobs[1].resolve();
    await tick();
    expect([...w.undoFailures][0].restoreCommitted).toBe(true);
    w.retryUndos();
    await tick();
    expect(repo.calls).toBe(2);
    await w.refreshRestored();
    expect(w.undoFailures.size).toBe(0);
    expect(repo.calls).toBe(2);
    expect(repo.cached[0].folder).toBe("Inbox");
  } finally {
    w.dispose();
  }
});

describe("captured query ordering", () => {
  it("keeps ordinal identity ties stable in both date directions", () => {
    const repo = new Controlled();
    repo.cached = ["a", "Z", "A"].map((id) => ({
      ...repo.cached[0],
      id,
      folder: "INBOX",
    }));
    const workspace = new Workspace(repo, new Settings());
    expect(workspace.matching.map((m) => m.id)).toEqual(["A", "Z", "a"]);
    workspace.newestFirst = false;
    expect(workspace.matching.map((m) => m.id)).toEqual(["A", "Z", "a"]);
    workspace.dispose();
  });
});
