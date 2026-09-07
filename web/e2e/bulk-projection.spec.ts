import { test, expect } from "@playwright/test";
import { seed, profile } from "./mailbox-fixture";

test("full-group queries and captures project approval, newer individual fields, receipts and Undo beyond one page", async ({
  page,
}) => {
  await seed(page);
  const result = await page.evaluate(async (profile) => {
    const storagePath = "/src/storage.ts",
      journalPath = "/src/bulk_journal.ts",
      workerPath = "/src/mailbox_worker_client.ts",
      selectionPath = "/src/selection_worker_client.ts";
    const { BrowserStore } = await import(storagePath),
      { BulkJournal } = await import(journalPath),
      { MailboxWorkerClient } = await import(workerPath),
      { SelectionWorkerClient } = await import(selectionPath);
    const store = await BrowserStore.open(profile),
      worker = new MailboxWorkerClient(profile),
      selection = new SelectionWorkerClient(profile);
    const saved = await store.all("mail"),
      state = await store.get("cacheState", "mail");
    const id = crypto.randomUUID();
    const query = (folder: string, offset = 0) =>
      worker.page({ scope: { folder }, offset });
    const original = (m: any) => ({
      id: m.core.id,
      account: m.core.account_id,
      folder: m.core.folder,
      remoteId: m.core.remote_id,
      unread: m.core.unread,
      starred: m.core.starred,
    });
    const result = await BulkJournal.own(profile, async (j: any) => {
      async function* chunks() {
        for (let i = 0; i < saved.length; i += 50)
          yield saved.slice(i, i + 50).map((m: any, n: number) => ({
            id: m.core.id,
            account: m.core.account_id,
            position: i + n,
            original: original(m),
          }));
      }
      let job = await j.prepare(
        id,
        { kind: "move", folder: "Archive", account: null },
        saved.length,
        chunks(),
        state.epoch,
      );
      const unapproved = await query("Inbox");
      const revision = await store.intents.reserve();
      job = await j.decide(id, job.revision, "approve", revision);
      const inbox = await query("Inbox"),
        archive = await query("Archive", 100);
      const capture = await selection.selection(
        {
          kind: "capture",
          id: crypto.randomUUID(),
          revision: 0,
          scope: { folder: "Archive" },
          all: true,
        },
        ["m000", "m124"],
      );
      // Newer same-device intent wins even before its cache write completes.
      const newer = await store.intents.register("m124", { folder: "Trash" });
      const afterNewer = await query("Archive");
      const last = await store.get("mail", "m124");
      last.core.folder = "Trash";
      await store.commit([{ store: "mail", key: "m124", value: last }], newer);
      await store.intents.finish(newer, "applied");
      // One actual cache receipt; the rest remain unsent group metadata.
      let item = await j.claim(id);
      const lease = await store.intents.claim(item.id, revision, {
        folder: "Archive",
      });
      await j.attachIntent(id, item.position, item.attempt, lease);
      const first = await store.get("mail", item.id),
        before = original(first);
      first.core.folder = "Archive";
      await j.settle(id, item.position, item.attempt, {
        kind: "committed",
        receipt: { before, after: original(first) },
        applied: { folder: "Archive" },
        cacheApplied: false,
      });
      await store.commit(
        [{ store: "mail", key: item.id, value: first }],
        lease,
      );
      await store.intents.finish(lease, "applied");
      job = await j.cacheSaved(id, item.position, item.attempt);
      const afterReceipt = await query("Archive");
      const undo = await store.intents.reserve();
      job = await j.decide(id, job.revision, "undo", undo);
      const undoneInbox = await query("Inbox"),
        undoneArchive = await query("Archive"),
        trash = await query("Trash");
      const frozen = await selection.selection(
        { kind: "observe", id: capture.id },
        ["m000", "m124"],
      );
      return {
        unapproved,
        inbox,
        archive,
        capture,
        afterNewer,
        afterReceipt,
        undoneInbox,
        undoneArchive,
        trash,
        frozen,
      };
    });
    await worker.close();
    await selection.close();
    store.close();
    return result;
  }, profile);
  expect(result.unapproved.total).toBe(125);
  expect(result.inbox.total).toBe(0);
  expect(result.inbox.unread).toBe(0);
  expect(result.archive.total).toBe(125);
  expect(result.archive.rows).toHaveLength(25);
  expect(result.capture.selected).toBe(125);
  expect(result.capture.groups.every((g: any) => g.folder === "Archive")).toBe(
    true,
  );
  expect(result.afterNewer.total).toBe(124);
  expect(result.afterReceipt.total).toBe(124);
  expect(result.undoneInbox.total).toBe(124);
  expect(result.undoneInbox.unread).toBe(124);
  expect(result.undoneArchive.total).toBe(0);
  expect(result.trash.rows.map((m: any) => m.id)).toEqual(["m124"]);
  expect(result.frozen.selected).toBe(125);
  expect(
    result.frozen.groups.some((g: any) => g.folder === "INBOX" && g.total > 0),
  ).toBe(true);
});

test("journal observations never take execution ownership and replay only changed metadata across decisions", async ({
  page,
}) => {
  await seed(page);
  const result = await page.evaluate(async (profile) => {
    const path = "/src/bulk_journal.ts",
      storagePath = "/src/storage.ts";
    const { BulkJournal } = await import(path),
      { BrowserStore } = await import(storagePath),
      store = await BrowserStore.open(profile);
    const state = await store.get("cacheState", "mail"),
      saved = await store.get("mail", "m000");
    const result = await BulkJournal.own(profile, async (j: any) => {
      const id = crypto.randomUUID();
      async function* chunks() {
        yield [
          {
            id: "m000",
            account: "work",
            position: 0,
            original: {
              id: "m000",
              account: "work",
              folder: "INBOX",
              remoteId: saved.core.remote_id,
              unread: true,
              starred: false,
            },
          },
        ];
      }
      let job = await j.prepare(
        id,
        { kind: "flags", starred: true },
        1,
        chunks(),
        state.epoch,
      );
      let revision = -1,
        copied = 0;
      const observe = () =>
        BulkJournal.inspect(profile, (view: any) =>
          view.projection({
            begin: () => {},
            job: (job: any) => {
              const old = revision;
              revision = job.revision;
              return old;
            },
            item: () => copied++,
            end: () => {},
          }),
        );
      await observe();
      const first = copied;
      await observe();
      const unchanged = copied;
      job = await j.decideCurrent(
        job,
        "approve",
        await store.intents.reserve(),
      );
      await observe();
      const approved = copied;
      const expected = job,
        item = await j.claim(id);
      await observe();
      const claimed = copied;
      // Pointer-down's reviewed decision is still current after a receipt advances the revision.
      await j.settle(id, item.position, item.attempt, {
        kind: "rejected",
        error: "Synthetic rejection",
      });
      job = await j.decideCurrent(
        expected,
        "undo",
        await store.intents.reserve(),
      );
      let stale = false;
      try {
        await j.decideCurrent(expected, "resume");
      } catch {
        stale = true;
      }
      return { first, unchanged, approved, claimed, undo: job.undo, stale };
    });
    store.close();
    return result;
  }, profile);
  expect(result).toEqual({
    first: 1,
    unchanged: 1,
    approved: 1,
    claimed: 2,
    undo: true,
    stale: true,
  });
});
