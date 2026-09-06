import { describe, it, expect } from "vitest";
import {
  GatewayRepository,
  type Account,
  type Outgoing,
  type RecordMail,
} from "./provider";
import {
  Workspace,
  defaults,
  type Mail,
  type Fields,
  type Repository,
} from "./model";
import type { LocalStore, StoreName, Change } from "./storage";

class Memory implements LocalStore {
  data = new Map<string, unknown>();
  failAliases = false;
  async all<T>(name: StoreName) {
    return structuredClone(
      [...this.data]
        .filter(([k]) => k.startsWith(`${name}:`))
        .map(([, v]) => v),
    ) as T[];
  }
  async get<T>(name: StoreName, key: string) {
    return structuredClone(this.data.get(`${name}:${key}`)) as T | undefined;
  }
  async snapshot(names: readonly StoreName[]) {
    return Object.fromEntries(
      await Promise.all(
        names.map(async (name) => [name, await this.all(name)]),
      ),
    );
  }
  async submissions<T>(id: string) {
    return (await this.all<{ id: string }>("outgoing")).filter(
      (r) => r.id === id,
    ) as T[];
  }
  async commit(changes: Change[]) {
    if (this.failAliases && changes.some((c) => c.store === "mailAliases")) {
      this.failAliases = false;
      throw new Error("Synthetic transaction failure");
    }
    for (const c of changes) {
      if (c.value === undefined) this.data.delete(`${c.store}:${c.key}`);
      else this.data.set(`${c.store}:${c.key}`, structuredClone(c.value));
    }
  }
}
const account: Account = {
  id: "handover",
  name: "Handover fixture",
  email: "owner@example.test",
  protocol: "Imap",
  host: "mail.example.test",
  port: 993,
  username: "owner",
  incoming_security: "Tls",
  incoming_auth: "Password",
  smtp_host: "mail.example.test",
  smtp_port: 465,
  smtp_username: "owner",
  smtp_security: "Tls",
  smtp_auth: "Automatic",
  smtp_separate_password: false,
  sent_copy: "Automatic",
  sent_folder: "Sent Mail",
};
const attempt = "H".repeat(43),
  localId = `handover:Sent:local-sent-${attempt}`,
  remoteId = "handover:Sent Mail:91.4";
const draft = {
  id: "handover-draft",
  accountId: account.id,
  to: "peer@example.test",
  cc: "",
  bcc: "",
  subject: "Handover fixture",
  body: "Original Sent body",
};
const originalRaw = btoa(
    `Message-ID: <${attempt}@shep.so>\r\n\r\nOriginal Sent body`,
  ),
  serverRaw = btoa(
    `Received: synthetic\r\nMessage-ID: <${attempt}@shep.so>\r\n\r\nOriginal Sent body`,
  );
function local(): RecordMail {
  return {
    local: true,
    core: {
      id: localId,
      account_id: account.id,
      remote_id: `local-sent-${attempt}`,
      folder: "Sent",
      sender: account.email,
      recipient: draft.to,
      subject: draft.subject,
      preview: draft.body,
      timestamp: 1788696000,
      unread: false,
      starred: false,
      attachment_count: 0,
    },
    text: draft.body,
    reply: {
      reply_to: [{ email: draft.to, text: draft.to }],
      to: [],
      cc: [],
      message_id: `<${attempt}@shep.so>`,
      references: [],
    },
  };
}
function provider(): RecordMail {
  const m = local();
  delete m.local;
  m.core.id = remoteId;
  m.core.folder = "Sent Mail";
  m.core.remote_id = "91.4";
  m.sentMessageId = `<${attempt}@shep.so>`;
  m.text = "Provider cached body";
  return m;
}
function outgoing(): Outgoing {
  return {
    id: attempt,
    account: structuredClone(account),
    draft: structuredClone(draft),
    state: "delivered",
    mail: local(),
    wire: {
      raw: originalRaw,
      envelope: { from: account.email, to: [draft.to] },
    },
    sent: {
      state: "saved",
      folder: "Sent Mail",
      receipt: { folder: "Sent Mail", remote_id: "91.4" },
    },
  };
}
const json = (value: unknown) =>
  new Response(JSON.stringify(value), {
    headers: { "content-type": "application/json" },
  });
async function fixture(withLocal = true, withProvider = false) {
  const store = new Memory();
  await store.commit([
    { store: "accounts", key: account.id, value: account },
    { store: "outgoing", key: draft.id, value: outgoing() },
    ...(withLocal
      ? [
          { store: "mail" as const, key: localId, value: local() },
          { store: "raw" as const, key: localId, value: originalRaw },
        ]
      : []),
    ...(withProvider
      ? [
          { store: "mail" as const, key: remoteId, value: provider() },
          { store: "raw" as const, key: remoteId, value: serverRaw },
        ]
      : []),
  ]);
  let gate: Promise<void> | undefined,
    release: () => void = () => {},
    entered: () => void = () => {},
    listing = true;
  let observed = new Promise<void>((r) => (entered = r));
  let queued = new Promise<void>(() => {}),
    queuedNotify = () => {},
    accountJobs = 0;
  const calls: { path: string; body: any }[] = [];
  let serial = 5;
  const fetcher: typeof fetch = async (input, init) => {
    const path = String(input),
      body = init?.body ? JSON.parse(String(init.body)) : undefined;
    calls.push({ path, body });
    if (path.endsWith("/probe")) return json({ connected: true });
    if (path.endsWith("/sync")) {
      entered();
      if (gate) await gate;
      const m = provider();
      const events = [
        { kind: "sent_folder", account: account.id, folder: "Sent Mail" },
        ...(listing
          ? [
              {
                kind: "message",
                mail: { summary: m.core, text: m.text, raw: serverRaw },
                reply: m.reply,
                sent_message_id: m.sentMessageId,
              },
            ]
          : []),
        {
          kind: "reconcile",
          account: account.id,
          folder: "Sent Mail",
          live_ids: listing ? [remoteId] : [],
        },
        { kind: "done" },
      ];
      return new Response(
        events.map((e) => JSON.stringify(e)).join("\n") + "\n",
        { headers: { "content-type": "application/x-ndjson" } },
      );
    }
    if (path.endsWith("/flags")) return json({ committed: true });
    if (path.endsWith("/resolve-move"))
      return json({ mail: body.receipt.current });
    if (path.endsWith("/move"))
      return json({ committed: true, remote_id: `92.${serial++}` });
    throw new Error(`Unexpected request ${path}`);
  };
  const jobs = new Map<string, Promise<unknown>>();
  const lock = async <T>(name: string, fn: () => Promise<T>) => {
    if (gate && name.endsWith(`account.${account.id}`) && ++accountJobs === 2)
      queuedNotify();
    const job = (jobs.get(name) ?? Promise.resolve()).catch(() => {}).then(fn);
    jobs.set(name, job);
    return job;
  };
  const create = () =>
    new GatewayRepository(
      { email: account.email, user_id: "U".repeat(43), csrf: "C".repeat(43) },
      store,
      fetcher,
      lock,
    );
  const repo = create();
  await repo.load();
  await repo.connect(account, "synthetic", "synthetic");
  return {
    store,
    repo,
    create,
    calls,
    hold: () => {
      accountJobs = 0;
      queued = new Promise<void>((r) => (queuedNotify = r));
      gate = new Promise<void>((r) => (release = r));
      observed = new Promise<void>((r) => (entered = r));
      return observed;
    },
    queued: () => queued,
    release: () => release(),
    listing: (value: boolean) => (listing = value),
  };
}
describe("native-equivalent browser Sent handover", () => {
  it("adopts the provider identity, keeps both IDs through reopen/reply and preserves submitted MIME", async () => {
    const s = await fixture();
    await s.repo.refresh("Sent");
    expect(s.repo.cached).toHaveLength(1);
    expect(s.repo.cached[0].id).toBe(localId);
    expect(s.repo.cached[0].folder).toBe("Sent Mail");
    expect(s.repo.aliases.get(remoteId)).toBe(localId);
    const reopened = s.create();
    await reopened.load();
    expect((await reopened.reply(remoteId, false)).body).toContain(
      "Provider cached body",
    );
    expect(await reopened.reply(localId, false)).toMatchObject({
      to: draft.to,
    });
    expect((await s.store.get<Outgoing>("outgoing", draft.id))?.wire?.raw).toBe(
      originalRaw,
    );
    expect(await s.store.get("raw", localId)).toBe(serverRaw);
    expect(await s.store.get("mail", remoteId)).toBeUndefined();
  });
  it("rolls handover back as one transaction and repairs a cached provider copy after receipt recovery", async () => {
    const s = await fixture(false, true);
    s.store.failAliases = true;
    await expect(s.repo.recoverSent(attempt, "check")).rejects.toThrow();
    expect(await s.store.get("mail", localId)).toBeUndefined();
    expect(await s.store.get("raw", remoteId)).toBe(serverRaw);
    expect(await s.store.all("mailAliases")).toEqual([]);
    await s.repo.recoverSent(attempt, "check");
    expect(s.repo.cached.map((m) => m.id)).toEqual([localId]);
    expect((await s.store.get<Outgoing>("outgoing", draft.id))?.wire?.raw).toBe(
      originalRaw,
    );
    await s.repo.recoverOutgoing(attempt, "check");
    expect(
      (await s.store.get<RecordMail>("mail", localId))?.core.remote_id,
    ).toBe("91.4");
  });
  it("persists a local edit while network sync is held and retains that local copy separately", async () => {
    const s = await fixture();
    const entered = s.hold();
    const refresh = s.repo.refresh("Sent");
    await entered;
    const changed = s.repo.mutate(localId, { starred: true });
    await Promise.race([
      changed,
      new Promise((_, reject) =>
        setTimeout(
          () => reject(new Error("Local edit blocked behind provider sync")),
          2000,
        ),
      ),
    ]);
    expect((await s.store.get<RecordMail>("mail", localId))?.localEdited).toBe(
      true,
    );
    s.release();
    await refresh;
    expect(s.repo.cached).toHaveLength(2);
    expect(s.repo.aliases.size).toBe(0);
    expect(s.repo.cached.find((m) => m.id === localId)?.starred).toBe(true);
    expect(s.calls.some((c) => c.path.endsWith("/flags"))).toBe(false);
  });
  it("resolves a queued provider action after handover and preserves physical move/Undo destinations", async () => {
    const s = await fixture(true, true);
    const entered = s.hold();
    const refresh = s.repo.refresh("Sent");
    await entered;
    const action = s.repo.mutate(remoteId, { starred: true });
    await s.queued();
    s.release();
    await Promise.all([refresh, action]);
    const flagged = s.calls.find((c) => c.path.endsWith("/flags"))!;
    expect(flagged.body.mail).toMatchObject({
      remote_id: "91.4",
      folder: "Sent Mail",
    });
    expect((await s.store.get<RecordMail>("mail", localId))?.core.starred).toBe(
      true,
    );
    await s.repo.mutate(remoteId, { folder: "Archive" });
    await s.repo.mutate(remoteId, { folder: "Sent Mail" });
    const moves = s.calls.filter((c) => c.path.endsWith("/move"));
    expect(moves.map((c) => c.body.folder)).toEqual(["Archive", "Sent Mail"]);
    expect(moves[1].body.mail.remote_id).toBe("92.5");
  });
  it("refuses ambiguous submissions, wrong acknowledged UIDs and edited legacy local copies", async () => {
    for (const kind of ["duplicate", "epoch", "legacy-edit"]) {
      const s = await fixture();
      if (kind === "duplicate")
        await s.store.commit([
          { store: "outgoing", key: "duplicate-draft", value: outgoing() },
        ]);
      else if (kind === "epoch") {
        const r = outgoing();
        r.sent!.receipt!.remote_id = "90.4";
        await s.store.commit([{ store: "outgoing", key: draft.id, value: r }]);
      } else {
        const m = local();
        m.core.starred = true;
        await s.store.commit([{ store: "mail", key: localId, value: m }]);
      }
      await s.repo.refresh("Sent");
      expect(s.repo.cached).toHaveLength(2);
      expect(s.repo.aliases.size).toBe(0);
    }
  });
  it("removes aliases with their reconciled message and retains the open reader snapshot", async () => {
    const s = await fixture();
    await s.repo.refresh("Sent");
    const w = new Workspace(s.repo, {
      read: () => structuredClone(defaults),
      write: () => {},
    });
    w.navigate("Sent");
    w.selected = localId;
    expect(w.visible).toHaveLength(1);
    s.listing(false);
    await w.refresh();
    expect(w.visible).toHaveLength(0);
    expect(w.readerMessage?.body).toBe("Provider cached body");
    expect(await s.store.all("mailAliases")).toEqual([]);
  });
});

class HandoverRepository implements Repository {
  preview = true;
  events = [];
  cached: Mail[] = [
    {
      id: remoteId,
      sender: "Peer",
      address: draft.to,
      subject: draft.subject,
      preview: "Before",
      body: "Before",
      date: "2026-09-06T10:00:00Z",
      account: account.email,
      accountId: account.id,
      folder: "Sent Mail",
      unread: false,
      starred: false,
      attachments: [],
    },
  ];
  aliases = new Map<string, string>();
  folderRoles = new Map([[account.id, new Set(["Sent Mail"])]]);
  resolve: () => void = () => {};
  jobs: {
    id: string;
    fields: Fields;
    resolve: () => void;
    reject: () => void;
  }[] = [];
  async refresh() {
    await new Promise<void>((r) => (this.resolve = r));
    this.aliases.set(remoteId, localId);
    this.cached = this.cached.map((m) => ({
      ...m,
      id: localId,
      body: "After handover",
    }));
    return structuredClone(this.cached);
  }
  async mutate(id: string, fields: Fields) {
    await new Promise<void>((resolve, reject) =>
      this.jobs.push({
        id,
        fields,
        resolve,
        reject: () => reject(new Error("Synthetic refusal")),
      }),
    );
    this.cached = this.cached.map((m) =>
      m.id === (this.aliases.get(id) ?? id) ? { ...m, ...fields } : m,
    );
  }
  async saveDraft() {}
  async send() {}
  async saveEvent() {}
}
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
describe("reader and intent through identity adoption", () => {
  it("keeps selection, reader and queued newer flags through an older rejection", async () => {
    const repo = new HandoverRepository(),
      w = new Workspace(repo, {
        read: () => structuredClone(defaults),
        write: () => {},
      });
    w.navigate("Sent");
    w.selected = remoteId;
    w.selection.add(remoteId);
    const refresh = w.refresh();
    const first = w.action(remoteId, "star");
    await tick();
    repo.resolve();
    await refresh;
    expect(w.selected).toBe(localId);
    expect(w.readerMessage?.body).toBe("After handover");
    expect(w.readerMessage?.starred).toBe(true);
    const second = w.action(localId, "star");
    repo.jobs[0].reject();
    await first;
    await tick();
    expect(w.readerMessage?.starred).toBe(false);
    expect(w.error).toBeNull();
    repo.jobs[1].resolve();
    await second;
    expect(w.pending).toBe(0);
  });
  it("retains an Undo created before handover and groups physical Sent folders", async () => {
    const repo = new HandoverRepository(),
      w = new Workspace(repo, {
        read: () => structuredClone(defaults),
        write: () => {},
      });
    w.navigate("Sent");
    w.selected = remoteId;
    const change = w.action(remoteId, "star");
    await tick();
    repo.jobs[0].resolve();
    await change;
    const refresh = w.refresh();
    repo.resolve();
    await refresh;
    expect(w.visible.map((m) => m.id)).toEqual([localId]);
    w.undo!();
    await tick();
    expect(repo.jobs[1].id).toBe(localId);
    repo.jobs[1].resolve();
    await tick();
    const archive = w.action(localId, "archive");
    await tick();
    repo.jobs[2].resolve();
    await archive;
    w.undo!();
    await tick();
    expect(repo.jobs[3].fields.folder).toBe("Sent Mail");
    repo.jobs[3].resolve();
    await tick();
    expect(w.visible).toHaveLength(1);
  });
});
