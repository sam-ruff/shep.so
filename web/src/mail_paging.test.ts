import { it, expect } from "vitest";
import {
  Workspace,
  defaults,
  MutationFailure,
  type Mail,
  type Fields,
  type Repository,
} from "./model";
import { MailBodies } from "./mail_paging";
import { mailMatches } from "./mail_query";
import type {
  MailboxPage,
  MailboxQuery,
  MailboxRepository,
} from "./mailbox_types";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
async function until(condition: () => boolean) {
  for (let i = 0; i < 40; i++) {
    if (condition()) return;
    await tick();
  }
  throw Error("Controlled work did not reach its expected state");
}
const metadata = (m: Mail): Mail => ({ ...m, body: "", bodyLoaded: false });
class Paged implements Repository {
  preview = false;
  cached: Mail[] = [];
  events = [];
  aliases = new Map<string, string>();
  accountIds = new Map([["work@example.test", "work"]]);
  source = new Map<string, Mail>();
  revision = 0;
  epoch = "source-one";
  holdPages = false;
  holdBodies = false;
  autoMutations = false;
  pageCalls: {
    query: MailboxQuery;
    value: MailboxPage;
    resolve: (p: MailboxPage) => void;
    reject: (e: Error) => void;
  }[] = [];
  bodyCalls: {
    id: string;
    resolve: (body: string) => void;
    reject: (e: Error) => void;
  }[] = [];
  jobs: {
    id: string;
    fields: Fields;
    resolve: (value?: { cache?: boolean; error?: Error }) => void;
    reject: (error: Error) => void;
  }[] = [];
  lookups: string[] = [];
  constructor(count = 125) {
    for (let i = 0; i < count; i++) {
      const id = `m${String(i).padStart(4, "0")}`;
      this.source.set(id, {
        id,
        accountId: "work",
        account: "work@example.test",
        sender: "Sender",
        address: "sender@example.test",
        subject: `Message ${i}`,
        preview: `Preview ${i}`,
        body: `Complete cached needle ${i}`,
        folder: "Inbox",
        date: new Date((1000000 - i) * 1000).toISOString(),
        unread: i % 2 === 0,
        starred: i % 3 === 0,
        attachments: [],
      });
    }
  }
  mailbox: MailboxRepository = {
    page: async (query) => {
      const matches = [...this.source.values()].map((m) => ({
        ...m,
        ...query.scope.projection?.[m.id],
      }));
      const all = matches
        .filter(
          (m) =>
            (!query.scope.account || query.scope.account === m.accountId) &&
            mailMatches(m, query.scope),
        )
        .sort(
          (a, b) =>
            (query.scope.oldest
              ? a.date.localeCompare(b.date)
              : b.date.localeCompare(a.date)) || a.id.localeCompare(b.id),
        );
      const rows = all.slice(query.offset, query.offset + 50).map(metadata);
      const value: MailboxPage = {
        rows,
        total: all.length,
        unread: matches.filter((m) => m.folder === "Inbox" && m.unread).length,
        epoch: this.epoch,
        revision: this.revision,
        aliases: Object.fromEntries(this.aliases),
        confirmed: Object.fromEntries(
          rows.map((m) => {
            const saved = this.source.get(m.id)!;
            return [
              m.id,
              {
                folder: saved.folder,
                unread: saved.unread,
                starred: saved.starred,
              },
            ];
          }),
        ),
      };
      const result = this.holdPages
        ? await new Promise<MailboxPage>((resolve, reject) =>
            this.pageCalls.push({ query, value, resolve, reject }),
          )
        : value;
      this.cached = result.rows;
      return result;
    },
    metadata: async (id) => {
      this.lookups.push(id);
      id = this.aliases.get(id) ?? id;
      const m = this.source.get(id);
      return {
        id,
        mail: m ? metadata(m) : undefined,
        revision: this.revision,
        epoch: this.epoch,
      };
    },
    detail: async (id) => {
      const epoch = this.epoch,
        revision = this.revision;
      const body = this.holdBodies
        ? await new Promise<string>((resolve, reject) =>
            this.bodyCalls.push({ id, resolve, reject }),
          )
        : this.source.get(id)!.body;
      return { id, body, epoch, revision };
    },
    close: async () => {},
  };
  async refresh() {
    return this.cached;
  }
  async mutate(id: string, fields: Fields): Promise<Fields> {
    const result = this.autoMutations
      ? undefined
      : await new Promise<{ cache?: boolean; error?: Error } | undefined>(
          (resolve, reject) => this.jobs.push({ id, fields, resolve, reject }),
        );
    if (result?.cache !== false) {
      const m = { ...this.source.get(id)!, ...fields };
      this.source.set(id, m);
      this.revision++;
      this.cached = [
        ...this.cached.filter((x) => x.id !== id),
        metadata(m),
      ].slice(-50);
    }
    if (result?.error) throw result.error;
    return fields;
  }
  async saveDraft() {}
  async send() {}
  async saveEvent() {}
}
async function create(count = 125) {
  const repo = new Paged(count),
    w = new Workspace(repo, {
      read: () => structuredClone(defaults),
      write: () => {},
    });
  await until(() => !w.pageLoading && w.visible.length > 0);
  return { repo, w };
}

it("keeps only a metadata page while retaining an independently loaded reader across pages", async () => {
  const { repo, w } = await create(1000);
  expect(w.total).toBe(1000);
  expect(w.unread).toBe(500);
  expect(w.mail).toHaveLength(50);
  w.beginReading("m0000");
  await until(() => !!w.readerMessage?.bodyLoaded);
  for (let i = 1; i < 20; i++) {
    w.page = i;
    w.changed();
    await until(() => !w.pageLoading);
    expect(w.mail).toHaveLength(50);
    expect(w.mail.every((m) => m.body === "" && !m.bodyLoaded)).toBe(true);
  }
  expect(w.readerMessage?.body).toBe("Complete cached needle 0");
  expect((w as any).confirmed.size).toBeLessThanOrEqual(51);
  expect(repo.source.size).toBe(1000);
  w.dispose();
});

it("projects a pending archive into other folders and rolls back to confirmed storage after navigation", async () => {
  const { repo, w } = await create();
  const move = w.action("m0000", "archive");
  expect(w.visible.some((m) => m.id === "m0000")).toBe(false);
  expect(w.total).toBe(124);
  expect(w.unread).toBe(62);
  w.navigate("Archive");
  await until(() => !w.pageLoading && w.visible.length === 1);
  expect(w.visible[0].folder).toBe("Archive");
  await until(() => repo.jobs.length === 1);
  repo.jobs[0].reject(Error("Offline fixture"));
  await move;
  await until(() => !w.pageLoading);
  expect(w.visible).toHaveLength(0);
  expect(w.error).toContain("restored");
  w.navigate("Inbox");
  await until(() => !w.pageLoading);
  expect(w.visible[0].id).toBe("m0000");
  expect(w.total).toBe(125);
  expect(w.unread).toBe(63);
  w.dispose();
});

it("does not let delayed pages or errors replace a newer query and coalesces intervening navigation", async () => {
  const { repo, w } = await create();
  repo.holdPages = true;
  w.search("Message 9");
  await until(() => repo.pageCalls.length === 1);
  w.search("Message 11");
  w.search("Message 12");
  await tick();
  expect(repo.pageCalls).toHaveLength(1);
  repo.pageCalls[0].reject(Error("Old query failure"));
  await until(() => repo.pageCalls.length === 2);
  expect(repo.pageCalls[1].query.scope.query).toBe("Message 12");
  expect(w.pageError).toBeNull();
  repo.pageCalls[1].resolve(repo.pageCalls[1].value);
  await until(() => !w.pageLoading);
  expect(w.visible.map((m) => m.subject)).toEqual([
    "Message 12",
    "Message 112",
    "Message 120",
    "Message 121",
    "Message 122",
    "Message 123",
    "Message 124",
  ]);
  expect(w.pageError).toBeNull();
  w.dispose();
});

it("late bodies cannot replace a newer reader and a current body failure has explicit retry", async () => {
  const { repo, w } = await create();
  repo.holdBodies = true;
  w.beginReading("m0000");
  await until(() => repo.bodyCalls.length === 1);
  w.beginReading("m0001");
  await until(() => repo.bodyCalls.length === 2);
  repo.bodyCalls[1].reject(Error("Damaged body fixture"));
  await until(() => !!w.bodyError);
  expect(w.readerMessage?.id).toBe("m0001");
  expect(w.bodyError).toContain("Damaged body");
  repo.bodyCalls[0].resolve("Old body must be ignored");
  await tick();
  expect(w.readerMessage?.body).toBe("");
  w.retryBody();
  await until(() => repo.bodyCalls.length === 3);
  repo.bodyCalls[2].resolve("Current reader body");
  await until(() => !!w.readerMessage?.bodyLoaded);
  expect(w.readerMessage?.body).toBe("Current reader body");
  expect(w.bodyError).toBeNull();
  repo.jobs[0]?.resolve();
  await tick();
  w.dispose();
});

it("retains an acknowledged cache gap as a projection until explicit refresh reconciles it", async () => {
  const { repo, w } = await create();
  const move = w.action("m0000", "archive");
  await until(() => repo.jobs.length === 1);
  repo.jobs[0].resolve({
    cache: false,
    error: new MutationFailure(
      "Accepted but cache failed",
      true,
      undefined,
      false,
      { folder: "Archive" },
    ),
  });
  await move;
  await until(() => !w.pageLoading);
  expect(w.visible.some((m) => m.id === "m0000")).toBe(false);
  await w.refresh();
  expect(w.error).toContain("cache recovery");
  expect(w.visible.some((m) => m.id === "m0000")).toBe(false);
  repo.source.set("m0000", { ...repo.source.get("m0000")!, folder: "Archive" });
  repo.revision++;
  await w.refresh();
  await until(() => !w.pageLoading);
  expect(w.error).toBeNull();
  expect(w.total).toBe(124);
  w.navigate("Archive");
  await until(() => !w.pageLoading);
  expect(w.visible[0].id).toBe("m0000");
  w.dispose();
});

it("enforces both speculative-body budgets without retaining an oversized active body", () => {
  const cache = new MailBodies();
  for (let i = 0; i < 20; i++) cache.put(String(i), `body ${i}`);
  expect(cache.size).toBe(8);
  expect(cache.get("0")).toBeUndefined();
  const fourMiB = "x".repeat(2 * 1024 * 1024);
  for (let i = 0; i < 9; i++) cache.put(`large${i}`, fourMiB);
  expect(cache.size).toBe(8);
  expect(cache.byteLength).toBe(32 * 1024 * 1024);
  cache.put("oversized", "x".repeat(17 * 1024 * 1024));
  expect(cache.get("oversized")).toBeUndefined();
  expect(cache.byteLength).toBe(32 * 1024 * 1024);
  cache.clear();
  expect(cache.size).toBe(0);
});

it("shows page recovery immediately and does not silently retry a current failure", async () => {
  const { repo, w } = await create();
  repo.holdPages = true;
  w.page = 1;
  w.changed();
  await until(() => repo.pageCalls.length === 1);
  repo.pageCalls[0].reject(Error("Page storage fixture failed"));
  await until(() => !!w.pageError);
  expect(w.visible).toHaveLength(0);
  await tick();
  expect(repo.pageCalls).toHaveLength(1);
  const recovery = w.retryPage();
  expect(w.pageLoading).toBe(true);
  await until(() => repo.pageCalls.length === 2);
  repo.pageCalls[1].resolve(repo.pageCalls[1].value);
  await recovery;
  expect(w.pageError).toBeNull();
  expect(w.visible[0].id).toBe("m0050");
  w.dispose();
});

it("removes pending account projections and ignores the late provider result", async () => {
  const { repo, w } = await create();
  const change = w.action("m0000", "star");
  await until(() => repo.jobs.length === 1);
  repo.source.clear();
  repo.revision++;
  w.accountRemoved("work");
  repo.jobs[0].reject(Error("Removed account fixture"));
  await change;
  await until(() => !w.pageLoading);
  expect(w.mail).toHaveLength(0);
  expect((w as any).pendingFields.size).toBe(0);
  expect((w as any).painted.size).toBe(0);
  expect((w as any).confirmed.size).toBe(0);
  expect(w.readerMessage).toBeNull();
  w.dispose();
});

it("restores counted moves after paging evicts original metadata and exposes an absent source", async () => {
  const { repo, w } = await create(200);
  repo.autoMutations = true;
  for (let i = 0; i < 61; i++) {
    await until(() => !w.pageLoading);
    await w.action(`m${String(i).padStart(4, "0")}`, "archive");
  }
  await until(() => !w.pageLoading);
  expect(w.moves.label).toBe("Archived 61 messages");
  expect((w as any).confirmed.has("m0000")).toBe(false);
  repo.source.delete("m0000");
  repo.revision++;
  w.undo!();
  await until(() => !w.pending && w.undoFailures.size > 0);
  await until(() => !w.pageLoading);
  expect(repo.lookups).toContain("m0000");
  expect(w.undoFailures.size).toBe(1);
  expect(w.moves.label).toBe("Restored 60 messages");
  expect(w.error).toContain("no longer available");
  expect(
    [...repo.source.values()].filter((m) => m.folder === "Archive"),
  ).toHaveLength(0);
  w.dispose();
});

it("rolls back synchronous reservation failures without losing the action slot", async () => {
  const { repo, w } = await create();
  Object.assign(repo, {
    registerMutation: () => {
      throw Error("Reservation fixture");
    },
  });
  await w.action("m0000", "star");
  expect(w.visible[0].starred).toBe(true);
  expect(w.error).toContain("Reservation fixture");
  expect(w.pending).toBe(0);
  expect((w as any).commandCount).toBe(0);
  expect((w as any).pendingFields.size).toBe(0);
  w.dispose();
});

it("a late mutation cannot paint into a recreated cache that reuses a message ID", async () => {
  const { repo, w } = await create();
  const change = w.action("m0000", "star");
  await until(() => repo.jobs.length === 1);
  repo.epoch = "replacement-source";
  await w.retryPage();
  await until(() => !w.pageLoading);
  expect(w.visible[0].starred).toBe(true);
  repo.jobs[0].resolve({ cache: false });
  await change;
  expect(w.visible[0].starred).toBe(true);
  expect((w as any).confirmed.get("m0000").starred).toBe(true);
  expect(w.undo).toBeNull();
  expect(w.error).toContain("cache was replaced");
  w.dispose();
});

it("rapid unread intent preserves filtered counts when the older write fails", async () => {
  const { repo, w } = await create();
  w.filter = "Unread";
  w.changed();
  await until(() => !w.pageLoading);
  expect(w.total).toBe(63);
  const first = w.action("m0000", "read");
  expect(w.visible.some((m) => m.id === "m0000")).toBe(false);
  expect(w.total).toBe(62);
  const second = w.change("m0000", { unread: true });
  expect(w.unread).toBe(63);
  await until(() => repo.jobs.length === 1);
  repo.jobs[0].reject(Error("Older unread fixture"));
  await first;
  await until(() => repo.jobs.length === 2);
  expect(w.unread).toBe(63);
  repo.jobs[1].resolve();
  await second;
  await until(() => !w.pageLoading);
  expect(w.total).toBe(63);
  expect(w.visible[0].unread).toBe(true);
  w.dispose();
});
