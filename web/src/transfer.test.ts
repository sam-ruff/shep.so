import "fake-indexeddb/auto";
import { it, expect } from "vitest";
import { BrowserStore, type LocalStore } from "./storage";
import { GatewayRepository, type Account, type CoreMail } from "./provider";
import { MutationFailure } from "./model";

const account = (id: string, protocol: Account["protocol"] = "Imap"): Account => ({
  id,
  name: `${id} name`,
  email: `${id}@example.test`,
  protocol,
  host: "mail.example.test",
  port: protocol === "Imap" ? 993 : 995,
  username: id,
  incoming_security: "Tls",
  incoming_auth: "Password",
  smtp_host: "mail.example.test",
  smtp_port: 465,
  smtp_username: "",
  smtp_security: "Tls",
  smtp_auth: "Automatic",
  smtp_separate_password: false,
  sent_copy: "ServerManaged",
  sent_folder: "",
});
const work = account("work"),
  personal = account("personal"),
  pop = account("pop", "Pop3");
const summary: CoreMail = {
  id: "work:INBOX:42.7",
  account_id: "work",
  remote_id: "42.7",
  folder: "INBOX",
  sender: "Alex <alex@example.test>",
  recipient: work.email,
  subject: "Plans for the weekend",
  preview: "Plans",
  timestamp: 1788692400,
  unread: true,
  starred: false,
  attachment_count: 0,
};
const raw = btoa("Subject: Plans for the weekend\r\n\r\nPlans");
const session = { email: "owner@example.test", csrf: "C".repeat(43) };
const unlock = async <T>(_name: string, run: () => Promise<T>) => run();
const json = (value: unknown, status = 200) =>
  new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });
const stream = (events: unknown[]) =>
  new Response(events.map((e) => JSON.stringify(e)).join("\n") + "\n", {
    headers: { "content-type": "application/x-ndjson" },
  });

let profiles = 0;
async function setup() {
  const profile = `T${++profiles}`.padEnd(43, "T");
  const store = await BrowserStore.open(profile);
  const requests: { path: string; body: any }[] = [];
  const control = {
    upload: "ok" as "ok" | "refused" | "lost" | "no-uid",
    finish: "ok" as "ok" | "fail",
    listed: true,
  };
  const request: typeof fetch = async (input, init) => {
    const path = String(input);
    const body = init?.body ? JSON.parse(String(init.body)) : undefined;
    requests.push({ path, body });
    if (path.endsWith("/probe")) return json({ connected: true });
    if (path.endsWith("/sync")) {
      const id = body.connection.account.id;
      const events: unknown[] = [
        { kind: "folders", account: id, folders: ["INBOX", "Plans", "Archive"] },
      ];
      if (id === "work" && body.folder.toUpperCase() === "INBOX" && control.listed)
        events.push({
          kind: "message",
          mail: { summary, raw, text: "Plans" },
        });
      if (id === "work" && body.folder.toUpperCase() === "INBOX")
        events.push({
          kind: "reconcile",
          account: id,
          folder: "INBOX",
          live_ids: control.listed ? [summary.id] : [],
        });
      events.push({ kind: "done", folders: ["INBOX"] });
      return stream(events);
    }
    if (path.endsWith("/transfer")) {
      if (control.upload === "lost") throw new TypeError("Synthetic lost reply");
      if (control.upload === "refused")
        return json({ error: "The message was not moved.", refused: true }, 409);
      return json({
        committed: true,
        remote_id: control.upload === "no-uid" ? null : "9.3",
      });
    }
    if (path.endsWith("/transfer/finish"))
      return control.finish === "ok"
        ? json({ committed: true })
        : json({ error: "The original could not be removed yet." }, 502);
    if (path.endsWith("/resolve-move"))
      return json({
        mail: body.receipt.current ?? {
          ...summary,
          account_id: body.receipt.account,
          folder: body.receipt.folder,
          remote_id: "9.4",
          id: `${body.receipt.account}:${body.receipt.folder}:9.4`,
        },
      });
    throw new Error(`Unexpected fixture path ${path}`);
  };
  // The real IndexedDB store, without the SQLite query workers Node lacks.
  const local: LocalStore = Object.create(store, {
    profileId: { value: undefined },
  });
  const repo = new GatewayRepository(
    { ...session, user_id: profile },
    local,
    request,
    unlock,
  );
  for (const a of [work, personal, pop]) await repo.connect(a, "p", "p");
  await repo.refresh();
  const id = repo.cached.find((m) => m.subject === summary.subject)!.id;
  return { store, repo, requests, control, id };
}
const calls = (requests: { path: string; body: any }[], suffix: string) =>
  requests.filter((r) => r.path.endsWith(suffix));

it("uploads the cached original, saves its receipt, then removes exactly the original", async () => {
  const s = await setup();
  const lease = await s.repo.registerMutation(s.id, {
    folder: "Plans",
    accountId: "personal",
  });
  await s.repo.mutate(s.id, { folder: "Plans", accountId: "personal" }, lease);
  const [upload] = calls(s.requests, "/transfer");
  expect(upload.body.source.account.id).toBe("work");
  expect(upload.body.destination.account.id).toBe("personal");
  expect(upload.body.mail.remote_id).toBe("42.7");
  expect(upload.body.folder).toBe("Plans");
  expect(upload.body.raw).toBe(raw);
  const [finish] = calls(s.requests, "/transfer/finish");
  expect(finish.body.source.account.id).toBe("work");
  expect(finish.body.mail).toMatchObject({ remote_id: "42.7", folder: "INBOX" });
  expect(calls(s.requests, "/move")).toHaveLength(0);
  const saved = await s.store.get<any>("mail", s.id);
  expect(saved.core).toMatchObject({
    account_id: "personal",
    folder: "Plans",
    remote_id: "9.3",
    id: "personal:Plans:9.3",
  });
  expect(saved.pendingTransfer).toBeUndefined();
  const action = (await s.store.intents!.activity!.page(undefined, true)).rows[0];
  expect(action.status).toBe("Succeeded");
  expect(action.receipt).toMatchObject({
    before: { account: "work", folder: "INBOX", remoteId: "42.7" },
    after: { account: "personal", folder: "Plans", remoteId: "9.3" },
  });
  // Undo moves it back through the same owner, from the destination account.
  await s.repo.undoSavedAction(action);
  const back = calls(s.requests, "/transfer").at(-1)!;
  expect(back.body.source.account.id).toBe("personal");
  expect(back.body.destination.account.id).toBe("work");
  expect(back.body.folder).toBe("INBOX");
  expect((await s.store.get<any>("mail", s.id)).core.account_id).toBe("work");
  s.store.close();
});

it("a definite refusal keeps the original in place and releases it for another try", async () => {
  const s = await setup();
  s.control.upload = "refused";
  const lease = await s.repo.registerMutation(s.id, {
    folder: "Plans",
    accountId: "personal",
  });
  const error = await s.repo
    .mutate(s.id, { folder: "Plans", accountId: "personal" }, lease)
    .catch((e) => e);
  expect(error).toBeInstanceOf(Error);
  expect(error).not.toBeInstanceOf(MutationFailure);
  expect(calls(s.requests, "/transfer/finish")).toHaveLength(0);
  const saved = await s.store.get<any>("mail", s.id);
  expect(saved.core.account_id).toBe("work");
  expect(saved.pendingMove).toBeUndefined();
  expect(saved.pendingTransfer).toBeUndefined();
  s.store.close();
});

it("an unconfirmed upload is never sent again until its review is accepted", async () => {
  const s = await setup();
  s.control.upload = "lost";
  const lease = await s.repo.registerMutation(s.id, {
    folder: "Plans",
    accountId: "personal",
  });
  await expect(
    s.repo.mutate(s.id, { folder: "Plans", accountId: "personal" }, lease),
  ).rejects.toMatchObject({ committed: false });
  expect(calls(s.requests, "/transfer/finish")).toHaveLength(0);
  // The original still being listed does not prove the copy is absent.
  await s.repo.refresh();
  expect((await s.store.get<any>("mail", s.id)).pendingTransfer).toBe(
    "personal",
  );
  s.control.upload = "ok";
  const again = await s.repo.registerMutation(s.id, {
    folder: "Plans",
    accountId: "personal",
  });
  await expect(
    s.repo.mutate(s.id, { folder: "Plans", accountId: "personal" }, again),
  ).rejects.toThrow(/no saved acknowledgment/);
  expect(calls(s.requests, "/transfer")).toHaveLength(1);
  const pending = (await s.store.intents!.activity!.page()).rows.find(
    (row) => row.status === "Uncertain",
  )!;
  await s.repo.acceptActionReview(pending, true);
  expect((await s.store.get<any>("mail", s.id)).pendingTransfer).toBeUndefined();
  s.store.close();
});

it("a failed cleanup keeps the receipt and repair removes the original without another upload", async () => {
  const s = await setup();
  s.control.finish = "fail";
  const lease = await s.repo.registerMutation(s.id, {
    folder: "Plans",
    accountId: "personal",
  });
  await expect(
    s.repo.mutate(s.id, { folder: "Plans", accountId: "personal" }, lease),
  ).rejects.toMatchObject({
    committed: true,
    receipt: { after: { account: "personal", remoteId: "9.3" } },
  });
  expect((await s.store.get<any>("mail", s.id)).core.account_id).toBe("work");
  const action = (await s.store.intents!.activity!.page()).rows[0];
  expect(action.status).toBe("Repair");
  s.control.finish = "ok";
  await s.repo.repairAction(action.id);
  expect(calls(s.requests, "/transfer")).toHaveLength(1);
  expect(calls(s.requests, "/transfer/finish")).toHaveLength(2);
  expect((await s.store.get<any>("mail", s.id)).core).toMatchObject({
    account_id: "personal",
    remote_id: "9.3",
  });
  s.store.close();
});

it("a copy without an APPENDUID is located by its exact content in the other account", async () => {
  const s = await setup();
  s.control.upload = "no-uid";
  const lease = await s.repo.registerMutation(s.id, {
    folder: "Plans",
    accountId: "personal",
  });
  await s.repo.mutate(s.id, { folder: "Plans", accountId: "personal" }, lease);
  const [resolve] = calls(s.requests, "/resolve-move");
  expect(resolve.body.connection.account.id).toBe("personal");
  expect(resolve.body.receipt.account).toBe("personal");
  const saved = await s.store.get<any>("mail", s.id);
  expect(saved.core).toMatchObject({ account_id: "personal", remote_id: "9.4" });
  expect(saved.moved).toBe(false);
  s.store.close();
});

it("POP3 and missing destinations are refused before any request", async () => {
  const s = await setup();
  for (const destination of ["pop", "missing"]) {
    const before = s.requests.length;
    await expect(
      s.repo.mutate(s.id, { folder: "Inbox", accountId: destination }),
    ).rejects.toThrow();
    expect(s.requests.length).toBe(before);
  }
  expect((await s.store.get<any>("mail", s.id)).core.account_id).toBe("work");
  s.store.close();
});
