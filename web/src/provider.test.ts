import { describe, it, expect } from "vitest";
import { GatewayRepository, type Account, type CoreMail } from "./provider";
import type { LocalStore, StoreName, Change } from "./storage";
import type { Draft, Fields } from "./model";
import type { IntentStore, IntentLease } from "./mail_intents";
import { MutationFailure } from "./model";
class Memory implements LocalStore {
  intents?: IntentStore;
  data = new Map<string, unknown>();
  fail = false;
  async all<T>(store: StoreName) {
    return structuredClone(
      [...this.data]
        .filter(([key]) => key.startsWith(`${store}:`))
        .map(([, value]) => value),
    ) as T[];
  }
  async get<T>(store: StoreName, key: string) {
    return structuredClone(this.data.get(`${store}:${key}`)) as T | undefined;
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
    if (this.fail) {
      this.fail = false;
      throw new Error("Synthetic disk full");
    }
    for (const c of changes) {
      if (c.value === undefined) this.data.delete(`${c.store}:${c.key}`);
      else this.data.set(`${c.store}:${c.key}`, structuredClone(c.value));
    }
  }
}

// Transaction/alias ordering is exercised in real Chromium. This adapter
// controls ownership at the wire boundary without replacing the provider path.
function controlledIntent(effective: Fields) {
  const lease: IntentLease = {
    id: summary.id,
    account: account.id,
    revision: 1,
    fields: { ...effective },
  };
  const outcomes: string[] = [];
  const intents: IntentStore = {
    reserve: async () => 1,
    register: async (_id, fields) => ({ ...lease, fields }),
    claim: async () => lease,
    effective: async () => effective,
    uncached: async (lease) => lease.fields,
    finish: async (_lease, status) => {
      outcomes.push(status);
    },
  };
  return { lease, intents, outcomes };
}
it("checks durable field ownership before dispatch and reports only accepted fields", async () => {
  const s = setup();
  await s.repo.connect(account, "p", "p");
  await s.repo.refresh();
  const intent = controlledIntent({ unread: false });
  s.db.intents = intent.intents;
  const applied = await s.repo.mutate(summary.id, {
    starred: true,
    unread: false,
  });
  expect(applied).toEqual({ unread: false });
  const writes = s.requests.filter((r) => r.path.endsWith("/flags"));
  expect(writes).toHaveLength(1);
  expect(writes[0].body.starred).toBeUndefined();
  expect(writes[0].body.unread).toBe(false);
  expect(intent.outcomes).toEqual(["applied"]);
  intent.intents.effective = async () => ({});
  expect(await s.repo.mutate(summary.id, { starred: true })).toEqual({});
  expect(s.requests.filter((r) => r.path.endsWith("/flags"))).toHaveLength(1);
});
it("acknowledgment survives intent finalization failure and returns a safe cached receipt", async () => {
  const s = setup();
  await s.repo.connect(account, "p", "p");
  await s.repo.refresh();
  const intent = controlledIntent({ starred: true });
  s.db.intents = intent.intents;
  intent.intents.finish = async () => {
    throw Error("Synthetic intent finalization failure");
  };
  await expect(
    s.repo.mutate(summary.id, { starred: true }),
  ).rejects.toMatchObject({
    committed: true,
    cacheApplied: true,
    applied: { starred: true },
    receipt: { after: { starred: true } },
  });
  expect((await s.db.get<any>("mail", summary.id)).core.starred).toBe(true);
  expect(s.requests.filter((r) => r.path.endsWith("/flags"))).toHaveLength(1);
});
it("unknown wire outcomes retain pending ownership while nondispatched failures retire it", async () => {
  const s = setup();
  await s.repo.connect(account, "p", "p");
  await s.repo.refresh();
  const intent = controlledIntent({ folder: "Archive" });
  s.db.intents = intent.intents;
  s.db.fail = true;
  await expect(
    s.repo.mutate(summary.id, { folder: "Archive" }),
  ).rejects.toThrow("disk full");
  expect(intent.outcomes).toEqual(["failed"]);
  expect(s.requests.filter((r) => r.path.endsWith("/move"))).toHaveLength(0);
  intent.outcomes.length = 0;
  s.moveFailure("lost");
  await expect(
    s.repo.mutate(summary.id, { folder: "Archive" }),
  ).rejects.toMatchObject({ committed: false });
  expect(intent.outcomes).toEqual([]);
  expect(s.requests.filter((r) => r.path.endsWith("/move"))).toHaveLength(1);
});

describe("acknowledged mutation receipts", () => {
  it("keeps remote and local commits acknowledged when only list refresh fails", async () => {
    for (const protocol of ["Imap", "Pop3"] as const) {
      for (const fields of [{ unread: false }, { folder: "Archive" }]) {
        const s = setup();
        await s.repo.connect({ ...account, protocol }, "p", "p");
        await s.repo.refresh();
        s.db.snapshot = async () => {
          throw Error("Synthetic display read failure");
        };
        const error = await s.repo.mutate(summary.id, fields).catch((e) => e);
        expect(error).toBeInstanceOf(MutationFailure);
        expect(error).toMatchObject({
          committed: true,
          cacheApplied: true,
          receipt: { before: { folder: "INBOX", remoteId: "42.7" } },
        });
        const saved = await s.db.get<any>("mail", summary.id);
        expect(saved.core).toMatchObject(fields);
        expect(error.receipt.after.folder).toBe(saved.core.folder);
        expect(error.message).toContain("message list could not refresh");
        expect(
          s.requests.filter((r) => /\/move$|\/flags$/.test(r.path)),
        ).toHaveLength(protocol === "Imap" ? 1 : 0);
      }
    }
  });
  it("delivers the physical receipt before a fallible cache update and preserves it on failure", async () => {
    const s = setup();
    await s.repo.connect(account, "p", "p");
    await s.repo.refresh();
    let acknowledged: any;
    const failure = await s.repo
      .mutateWithReceipt(summary.id, { folder: "Archive" }, async (result) => {
        acknowledged = structuredClone(result);
        expect((await s.db.get<any>("mail", summary.id)).core.folder).toBe(
          "INBOX",
        );
        expect((await s.db.get<any>("mail", summary.id)).pendingMove).toBe(
          "Archive",
        );
        s.db.fail = true;
      })
      .catch((e) => e);
    expect(failure).toMatchObject({
      committed: true,
      cacheApplied: false,
      receipt: acknowledged.receipt,
    });
    expect(acknowledged).toMatchObject({
      cacheApplied: false,
      receipt: {
        before: { id: summary.id, remoteId: "42.7" },
        after: { id: summary.id, folder: "Archive", remoteId: "91.8" },
      },
    });
    expect(acknowledged.receipt.recovery.sha256).toHaveLength(32);
    const reopened = s.reopen();
    await reopened.load();
    await reopened.connect(account, "p", "p");
    await expect(
      reopened.mutate(summary.id, { folder: "Archive" }),
    ).rejects.toThrow("not moved again");
    expect(s.requests.filter((r) => r.path.endsWith("/move"))).toHaveLength(1);
  });
  it("retains unknown-destination proof and returns the recovered identity without repeating MOVE", async () => {
    const s = setup();
    await s.repo.connect(account, "p", "p");
    await s.repo.refresh();
    s.moveRemote(null);
    const receipts: any[] = [];
    const result = await s.repo.mutateWithReceipt(
      summary.id,
      { folder: "Archive" },
      async (receipt) => {
        receipts.push(receipt);
      },
    );
    expect(receipts).toHaveLength(1);
    expect(receipts[0].receipt.after).toMatchObject({
      folder: "Archive",
      remoteId: "",
    });
    expect(receipts[0].receipt.recovery).toMatchObject({ bytes: 9 });
    expect(result).toMatchObject({
      cacheApplied: true,
      receipt: { after: { folder: "Archive", remoteId: "91.8" } },
    });
    expect(s.requests.filter((r) => r.path.endsWith("/move"))).toHaveLength(1);
  });
  it("rejects changed physical sources before flags or MOVE and keeps receipt-save failures acknowledged", async () => {
    const s = setup();
    await s.repo.connect(account, "p", "p");
    await s.repo.refresh();
    const original = {
      id: summary.id,
      account: account.id,
      folder: "INBOX",
      remoteId: "obsolete",
      unread: true,
      starred: false,
    };
    await expect(
      s.repo.mutateWithReceipt(
        summary.id,
        { unread: false },
        undefined,
        original,
      ),
    ).rejects.toThrow("changed since the group review");
    expect(s.requests.some((r) => r.path.endsWith("/flags"))).toBe(false);
    const failure = await s.repo
      .mutateWithReceipt(summary.id, { unread: false }, async () => {
        throw new MutationFailure("Synthetic receipt writer failure");
      })
      .catch((e) => e);
    expect(failure).toMatchObject({
      committed: true,
      cacheApplied: false,
      receipt: { after: { unread: false } },
    });
    expect((await s.db.get<any>("mail", summary.id)).core.unread).toBe(true);
    expect(s.requests.filter((r) => r.path.endsWith("/flags"))).toHaveLength(1);
  });
});
const account: Account = {
  id: "fixture",
  name: "Fixture",
  email: "owner@example.test",
  protocol: "Imap",
  host: "mail.example.test",
  port: 993,
  username: "owner",
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
};
const draft: Draft = {
  id: "draft",
  accountId: account.id,
  to: "recipient@example.test",
  cc: "",
  bcc: "hidden@example.test",
  subject: "Delivery fixture",
  body: "Synthetic body",
};
const summary: CoreMail = {
  id: "fixture:INBOX:42.7",
  account_id: "fixture",
  remote_id: "42.7",
  folder: "INBOX",
  sender: "Renée <renee@example.test>",
  recipient: account.email,
  subject: "Streaming fixture",
  preview: "Résumé",
  timestamp: 1788692400,
  unread: true,
  starred: false,
  attachment_count: 0,
};
const message = {
  kind: "message",
  mail: { summary, raw: "c3ludGhldGlj", text: "Résumé\nOne body" },
};
const done = { kind: "done", folders: ["INBOX"] };
const session = {
  email: "owner@example.test",
  csrf: "C".repeat(43),
  user_id: "U".repeat(43),
};
const unlock = async <T>(_name: string, run: () => Promise<T>) => run();
function json(value: unknown, status = 200) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });
}
function stream(events: unknown[], terminated = true) {
  const bytes = new TextEncoder().encode(
    events.map((e) => JSON.stringify(e)).join("\n") + (terminated ? "\n" : ""),
  );
  let offset = 0;
  return new Response(
    new ReadableStream({
      pull(c) {
        if (offset === bytes.length) c.close();
        else
          c.enqueue(
            bytes.slice(offset, (offset += Math.min(7, bytes.length - offset))),
          );
      },
    }),
    { headers: { "content-type": "application/x-ndjson" } },
  );
}
function setup() {
  const db = new Memory();
  let sends = 0,
    checks = 0;
  let mode = "delivered";
  let events: unknown[] = [message, done];
  let terminated = true;
  let moveRemote: string | null = "91.8";
  let recoveryFails = false;
  let moveFailure = "";
  const requests: { path: string; body: any }[] = [];
  const request: typeof fetch = async (input, init) => {
    const path = String(input);
    const body = init?.body ? JSON.parse(String(init.body)) : undefined;
    requests.push({ path, body });
    expect(init?.credentials).toBe("same-origin");
    expect(init?.redirect).toBe("error");
    if (body)
      expect((init!.headers as Record<string, string>)["x-shep-csrf"]).toBe(
        session.csrf,
      );
    if (path.endsWith("probe")) return json({ connected: true });
    if (path.endsWith("sync")) return stream(events, terminated);
    if (path.endsWith("resolve-move")) {
      if (recoveryFails)
        return json({ error: "Several copies match this destination." }, 502);
      return json({
        mail: {
          ...summary,
          folder: body.receipt.folder,
          remote_id: "91.8",
          id: `${account.id}:${body.receipt.folder}:91.8`,
        },
      });
    }
    if (path.endsWith("flags")) return json({ committed: true });
    if (path.endsWith("move")) {
      if (moveFailure === "lost")
        throw new Error("Synthetic lost acknowledgment");
      if (moveFailure === "cache") db.fail = true;
      return json({ committed: true, remote_id: moveRemote });
    }
    if (path.endsWith("reserve"))
      return json({ id: "R".repeat(43), state: "reserved" }, 201);
    if (path.endsWith("prepare")) {
      const core = {
        ...summary,
        id: `${account.id}:Sent:local-sent-${"R".repeat(43)}`,
        remote_id: `local-sent-${"R".repeat(43)}`,
        folder: "Sent",
        sender: account.email,
        subject: body.draft.subject,
        unread: false,
      };
      if (mode === "prepare-fail")
        throw new Error("Synthetic lost preparation");
      if (mode === "wire-save-fail") db.fail = true;
      return json({
        id: "R".repeat(43),
        state: "reserved",
        wire: {
          envelope: {
            from: account.email,
            to: ["recipient@example.test", "hidden@example.test"],
          },
          raw: "UHJlcGFyZWQgc3ludGhldGljIE1JTUU=",
        },
        mail: { core, text: body.draft.body, local: true },
      });
    }
    if (path.endsWith("send")) {
      sends++;
      expect(await db.get("outgoing", draft.id)).toMatchObject({
        state: "submitting",
        wire: body.wire,
      });
      if (mode === "lost") throw new TypeError("Synthetic connection reset");
      return json(
        { id: "R".repeat(43), state: mode },
        mode === "uncertain" ? 409 : 200,
      );
    }
    if (path.includes("/outgoing/")) {
      checks++;
      if (path.endsWith("/cancel") && mode === "reserved") mode = "cancelled";
      return mode === "unknown"
        ? json({ error: "Unknown" }, 404)
        : json({
            id: "R".repeat(43),
            state: mode === "lost" ? "delivered" : mode,
          });
    }
    throw new Error(`Unexpected fixture path ${path}`);
  };
  const repo = new GatewayRepository(session, db, request, unlock);
  return {
    db,
    repo,
    requests,
    reopen: () => new GatewayRepository(session, db, request, unlock),
    moveFailure: (v: string) => {
      moveFailure = v;
    },
    moveRemote: (v: string | null) => {
      moveRemote = v;
    },
    recoveryFails: (v: boolean) => {
      recoveryFails = v;
    },
    sends: () => sends,
    checks: () => checks,
    mode: (v: string) => {
      mode = v;
    },
    events: (v: unknown[], complete = true) => {
      events = v;
      terminated = complete;
    },
  };
}
describe("real browser provider/cache contract", () => {
  it("stores no passwords; a reopened cache remains readable and asks to reconnect", async () => {
    const s = setup();
    await s.repo.load();
    await s.repo.connect(account, "private-incoming", "private-smtp");
    const mail = await s.repo.refresh();
    expect(mail[0].sender).toBe("Renée");
    expect(mail[0].body).toBe("Résumé\nOne body");
    expect(JSON.stringify([...s.db.data])).not.toContain("private-");
    const reopened = s.reopen();
    await reopened.load();
    expect(reopened.cached).toEqual(mail);
    await reopened.refresh();
    expect(reopened.warning).toContain("Reconnect");
    expect(s.requests.filter((r) => r.path.endsWith("sync"))).toHaveLength(1);
  });
  it("keeps partial messages and existing cache when reconciliation is followed by failure", async () => {
    const s = setup();
    await s.repo.connect(account, "password", "password");
    await s.repo.refresh();
    s.events([
      { kind: "reconcile", account: account.id, folder: "INBOX", live_ids: [] },
      { kind: "error", error: "Synthetic partial failure" },
    ]);
    const mail = await s.repo.refresh();
    expect(mail).toHaveLength(1);
    expect(s.repo.warning).toContain("partial failure");
  });
  it("rejects truncated streams, cross-account identities and missing completion", async () => {
    const s = setup();
    await s.repo.connect(account, "password", "password");
    s.events([message, done], false);
    await s.repo.refresh();
    expect(s.repo.warning).toContain("incomplete");
    s.events([
      {
        ...message,
        mail: {
          ...message.mail,
          summary: { ...summary, account_id: "another" },
        },
      },
      done,
    ]);
    await s.repo.refresh();
    expect(s.repo.warning).toContain("identity");
    s.events([message]);
    await s.repo.refresh();
    expect(s.repo.warning).toContain("interrupted");
  });
  it("reconciles only a completely listed folder and keeps POP3 local edits", async () => {
    const s = setup();
    await s.repo.connect(account, "password", "password");
    await s.repo.refresh();
    s.events([
      { kind: "reconcile", account: account.id, folder: "INBOX", live_ids: [] },
      done,
    ]);
    expect(await s.repo.refresh()).toHaveLength(0);
    expect(await s.db.get("raw", summary.id)).toBeUndefined();
    const pop = { ...account, protocol: "Pop3" as const };
    const p = setup();
    await p.repo.connect(pop, "p", "p");
    await p.repo.refresh();
    await p.repo.mutate(summary.id, {
      folder: "Archive",
      unread: false,
      starred: true,
    });
    await p.repo.refresh();
    expect(p.repo.cached[0]).toMatchObject({
      folder: "Archive",
      unread: false,
      starred: true,
    });
    expect(p.requests.some((r) => /flags$|move$/.test(r.path))).toBe(false);
  });
  it("keeps a stable local id through MOVE, sync, restart and verified Undo", async () => {
    const s = setup();
    await s.repo.connect(account, "p", "p");
    await s.repo.refresh();
    await s.repo.mutate(summary.id, { folder: "Archive" });
    expect(s.repo.cached[0]).toMatchObject({
      id: summary.id,
      folder: "Archive",
    });
    const destination = {
      ...summary,
      folder: "Archive",
      remote_id: "91.8",
      id: "fixture:Archive:91.8",
    };
    s.events([
      { ...message, mail: { ...message.mail, summary: destination } },
      { kind: "flags", flags: [[destination.id, false, true]] },
      {
        kind: "reconcile",
        account: account.id,
        folder: "Archive",
        live_ids: [destination.id],
      },
      done,
    ]);
    await s.repo.refresh("Archive");
    expect(s.repo.cached).toHaveLength(1);
    expect(s.repo.cached[0]).toMatchObject({
      id: summary.id,
      unread: false,
      starred: true,
    });
    expect(await s.db.get("raw", summary.id)).toBe(message.mail.raw);
    const reopened = s.reopen();
    await reopened.load();
    await reopened.connect(account, "p", "p");
    s.moveRemote("42.18");
    await reopened.mutate(summary.id, { folder: "Inbox" });
    const moves = s.requests.filter((r) => r.path.endsWith("/move"));
    expect(moves[1].body.mail).toMatchObject({
      remote_id: "91.8",
      folder: "Archive",
    });
    expect(moves[1].body.folder).toBe("INBOX");
    expect(reopened.cached[0]).toMatchObject({
      id: summary.id,
      folder: "Inbox",
    });
    expect(s.requests.some((r) => r.path.endsWith("resolve-move"))).toBe(true);
  });
  it("recovers missing receipts and refuses ambiguous copies without issuing another MOVE", async () => {
    for (const fail of [false, true]) {
      const s = setup();
      await s.repo.connect(account, "p", "p");
      await s.repo.refresh();
      s.moveRemote(null);
      s.recoveryFails(fail);
      if (fail) {
        await expect(
          s.repo.mutate(summary.id, { folder: "Archive" }),
        ).rejects.toMatchObject({ committed: true });
        await expect(
          s.repo.mutate(summary.id, { folder: "Inbox" }),
        ).rejects.toThrow("copies");
        expect(s.requests.filter((r) => r.path.endsWith("/move"))).toHaveLength(
          1,
        );
        expect(
          (await s.db.get<any>("mail", summary.id)).receipt.current,
        ).toBeNull();
      } else {
        await s.repo.mutate(summary.id, { folder: "Archive" });
        expect(s.repo.cached[0]).toMatchObject({
          id: summary.id,
          folder: "Archive",
        });
        s.moveRemote("42.18");
        await s.repo.mutate(summary.id, { folder: "Inbox" });
        const moves = s.requests.filter((r) => r.path.endsWith("/move"));
        expect(moves[1].body.mail.remote_id).toBe("91.8");
      }
    }
  });
  it("persists unresolved MOVE intent before transmission and never retries it after lost acknowledgment or cache failure", async () => {
    for (const failure of ["lost", "cache"]) {
      const s = setup();
      await s.repo.connect(account, "p", "p");
      await s.repo.refresh();
      s.moveFailure(failure);
      await expect(
        s.repo.mutate(summary.id, { folder: "Archive" }),
      ).rejects.toThrow();
      const reopened = s.reopen();
      await reopened.load();
      await reopened.connect(account, "p", "p");
      await expect(
        reopened.mutate(summary.id, { folder: "Inbox" }),
      ).rejects.toThrow("not moved again");
      expect(s.requests.filter((r) => r.path.endsWith("/move"))).toHaveLength(
        1,
      );
      s.events([
        message,
        {
          kind: "reconcile",
          account: account.id,
          folder: "INBOX",
          live_ids: [summary.id],
        },
        done,
      ]);
      await reopened.refresh();
      s.moveFailure("");
      await reopened.mutate(summary.id, { folder: "Archive" });
      expect(s.requests.filter((r) => r.path.endsWith("/move"))).toHaveLength(
        2,
      );
    }
    const s = setup();
    await s.repo.connect(account, "p", "p");
    await s.repo.refresh();
    s.db.fail = true;
    await expect(
      s.repo.mutate(summary.id, { folder: "Archive" }),
    ).rejects.toThrow("disk full");
    expect(s.requests.filter((r) => r.path.endsWith("/move"))).toHaveLength(0);
  });
  it("persists the outgoing identity before sending and survives lost acknowledgment/reload", async () => {
    const s = setup();
    await s.repo.connect(account, "incoming", "smtp");
    s.mode("lost");
    await expect(s.repo.send(draft)).rejects.toThrow("not confirmed");
    expect(s.sends()).toBe(1);
    const reopened = s.reopen();
    await reopened.load();
    expect(reopened.drafts).toEqual([{ ...draft, attachments: [] }]);
    await reopened.send(draft);
    expect(s.checks()).toBe(1);
    expect(s.sends()).toBe(1);
    expect(await s.db.get("drafts", draft.id)).toBeUndefined();
    await expect(reopened.saveDraft(draft)).rejects.toThrow(
      "already delivered",
    );
    await reopened.send(draft);
    expect(s.sends()).toBe(1);
    expect(JSON.stringify([...s.db.data])).not.toContain('"smtp"');
  });
  it("does not submit if durable storage fails", async () => {
    const s = setup();
    await s.repo.connect(account, "p", "p");
    s.db.fail = true;
    await expect(s.repo.send(draft)).rejects.toThrow("disk full");
    expect(s.sends()).toBe(0);
  });
  it("never resends ambiguous, rejected, unknown or edited submissions", async () => {
    for (const mode of ["uncertain", "rejected"]) {
      const s = setup();
      await s.repo.connect(account, "p", "p");
      s.mode(mode);
      await expect(s.repo.send(draft)).rejects.toThrow();
      await expect(s.repo.send(draft)).rejects.toThrow();
      expect(s.sends()).toBe(1);
      await expect(s.repo.send({ ...draft, body: "changed" })).rejects.toThrow(
        "delivery record",
      );
      s.mode("unknown");
      await expect(s.repo.send(draft)).rejects.toThrow("not resent");
      expect(s.sends()).toBe(1);
    }
  });
});

describe("persistent draft files and cached replies", () => {
  it("keeps blobs across reopening, excludes removed files despite late text saves and sends exact metadata/bytes/headers", async () => {
    const s = setup();
    await s.repo.connect(account, "incoming", "smtp");
    const initial = {
      ...draft,
      revision: 1,
      inReplyTo: "<original@example.test>",
      references: ["<root@example.test>", "<original@example.test>"],
    };
    await s.repo.saveDraft(initial);
    const files = await s.repo.addFiles(draft.id, [
      new File(["Remove me"], "first.txt", { type: "text/plain" }),
      new File([new Uint8Array([0, 255, 1, 13, 10])], "binary.bin", {
        type: "application/octet-stream",
      }),
    ]);
    await s.repo.saveDraft({ ...initial, revision: 2, attachments: [] });
    const reopened = s.reopen();
    await reopened.load();
    expect(reopened.drafts[0].attachments).toEqual(files);
    const remaining = await reopened.removeFile(draft.id, files[0].id);
    await reopened.saveDraft({ ...initial, revision: 3, attachments: files });
    expect(await reopened.attachments(draft.id)).toEqual([files[1]]);
    expect((await s.db.all<any>("draftFiles"))[0].blob.size).toBe(5);
    await reopened.connect(account, "incoming", "smtp");
    s.mode("lost");
    const sending = { ...initial, revision: 3, attachments: remaining };
    await expect(reopened.send(sending)).rejects.toThrow("not confirmed");
    const request = s.requests.find((r) => r.path.endsWith("/prepare"))!.body;
    expect(request.draft).toMatchObject({
      in_reply_to: initial.inReplyTo,
      references: initial.references,
      bcc: draft.bcc,
    });
    expect(request.files).toEqual([
      {
        id: files[1].id,
        name: "binary.bin",
        media_type: "application/octet-stream",
        data: "AP8BDQo=",
      },
    ]);
    await expect(reopened.removeFile(draft.id, files[1].id)).rejects.toThrow(
      "delivery record",
    );
    await expect(
      reopened.addFiles(draft.id, [new File(["late"], "late.txt")]),
    ).rejects.toThrow("delivery record");
    await reopened.send(sending);
    expect(s.sends()).toBe(1);
    expect(await reopened.attachments(draft.id)).toEqual(remaining);
  });
  it("rejects stale file and text snapshots before reserving SMTP", async () => {
    const s = setup();
    await s.repo.connect(account, "p", "p");
    await s.repo.saveDraft({ ...draft, revision: 2 });
    const files = await s.repo.addFiles(draft.id, [
      new File(["fixture"], "fixture.txt"),
    ]);
    await expect(
      s.repo.send({ ...draft, revision: 1, attachments: files }),
    ).rejects.toThrow("another editor");
    await expect(
      s.repo.send({ ...draft, revision: 2, attachments: [] }),
    ).rejects.toThrow("attachments changed");
    await expect(s.repo.saveDraft({ ...draft, revision: 1 })).rejects.toThrow(
      "newer text",
    );
    expect(s.requests.some((r) => r.path.endsWith("reserve"))).toBe(false);
    expect(s.sends()).toBe(0);
  });
  it("fails an import atomically on size, name, count and storage failures, and scopes removal to one draft", async () => {
    const s = setup();
    await s.repo.saveDraft(draft);
    const first = await s.repo.addFiles(draft.id, [
      new File(["one"], "one.txt"),
    ]);
    await s.repo.saveDraft({ ...draft, id: "another" });
    expect(await s.repo.removeFile("another", first[0].id)).toEqual([]);
    expect(await s.repo.attachments(draft.id)).toEqual(first);
    for (const batch of [
      [
        new File(["small"], "small.txt"),
        new File([new Uint8Array(18 * 1024 * 1024)], "too-large.bin"),
      ],
      [new File(["small"], "small.txt"), new File(["bad"], "bad\nname")],
      Array.from({ length: 32 }, (_, n) => new File([""], `file-${n}`)),
    ])
      await expect(s.repo.addFiles(draft.id, batch)).rejects.toThrow();
    s.db.fail = true;
    await expect(
      s.repo.addFiles(draft.id, [new File(["later"], "later.txt")]),
    ).rejects.toThrow("disk full");
    expect(await s.repo.attachments(draft.id)).toEqual(first);
    await expect(
      s.repo.addFiles("missing", [new File(["x"], "x.txt")]),
    ).rejects.toThrow("Save the draft");
  });
  it("prepares Reply all from cached Rust envelopes without reconnecting and excludes all saved accounts", async () => {
    const s = setup();
    await s.repo.connect(account, "p", "p");
    await s.repo.connect(
      { ...account, id: "second", email: "alias@example.test" },
      "p",
      "p",
    );
    const addr = (email: string) => ({ email, text: email });
    s.events([
      {
        ...message,
        reply: {
          reply_to: [addr("support@example.test")],
          to: [
            addr(account.email),
            addr("ALIAS@example.test"),
            addr("peer@example.test"),
          ],
          cc: [addr("peer@example.test"), addr("copy@example.test")],
          message_id: "<original@example.test>",
          references: [],
        },
      },
      done,
    ]);
    await s.repo.refresh();
    const repo = s.reopen();
    await repo.load();
    const requests = s.requests.length;
    const reply = await repo.reply(summary.id, true);
    expect(reply).toMatchObject({
      to: "support@example.test, peer@example.test",
      cc: "copy@example.test",
      bcc: "",
      inReplyTo: "<original@example.test>",
      references: ["<original@example.test>"],
    });
    expect(s.requests).toHaveLength(requests);
  });
});

describe("prepared SMTP and local Sent", () => {
  it("never starts SMTP after losing preparation or failing to save exact MIME", async () => {
    for (const failure of ["prepare-fail", "wire-save-fail"]) {
      const s = setup();
      await s.repo.connect(account, "p", "p");
      s.mode(failure);
      await expect(s.repo.send(draft)).rejects.toThrow("not confirmed");
      expect(s.sends()).toBe(0);
      expect(await s.db.get("outgoing", draft.id)).toMatchObject({
        state: "preparing",
      });
      const reopened = s.reopen();
      await reopened.load();
      s.mode("reserved");
      await expect(reopened.send(draft)).rejects.toThrow("not resent");
      expect(
        s.requests.filter((r) => r.path.endsWith("/prepare")),
      ).toHaveLength(1);
      expect(s.sends()).toBe(0);
    }
  });
  it("keeps exact Sent bytes after reload and reconciliation, with offline local flags and moves", async () => {
    const s = setup();
    await s.repo.connect(account, "p", "p");
    await s.repo.send(draft);
    const record = await s.db.get<any>("outgoing", draft.id);
    const sent = s.repo.cached.find((m) => m.folder === "Sent")!;
    expect(sent.body).toBe(draft.body);
    expect(await s.db.get("raw", sent.id)).toBe(record.wire.raw);
    const reopened = s.reopen();
    await reopened.load();
    await reopened.mutate(sent.id, { starred: true });
    await reopened.mutate(sent.id, { folder: "Archive" });
    expect(reopened.cached[0]).toMatchObject({
      folder: "Archive",
      starred: true,
    });
    await reopened.connect(account, "p", "p");
    s.events([
      {
        kind: "reconcile",
        account: account.id,
        folder: "Archive",
        live_ids: [],
      },
      done,
    ]);
    await reopened.refresh("Archive");
    expect(reopened.cached[0]).toMatchObject({
      id: sent.id,
      folder: "Archive",
      starred: true,
    });
    expect(
      s.requests.some(
        (r) => r.path.endsWith("/flags") || r.path.endsWith("/move"),
      ),
    ).toBe(false);
    expect(s.sends()).toBe(1);
  });
});

describe("Outbox review and recovery", () => {
  const id = "R".repeat(43);
  it("atomically cancels an unused reservation before returning its immutable content", async () => {
    const s = setup();
    await s.repo.connect(account, "p", "p");
    s.mode("prepare-fail");
    await expect(s.repo.send(draft)).rejects.toThrow("not confirmed");
    s.mode("reserved");
    const reopened = s.reopen();
    await reopened.load();
    const recovered = await reopened.recoverOutgoing(id, "return");
    expect(s.requests.at(-1)?.path).toBe(`/api/mail/outgoing/${id}/cancel`);
    expect(recovered).toMatchObject({
      subject: draft.subject,
      body: draft.body,
      revision: 0,
    });
    expect(recovered?.id).not.toBe(draft.id);
    expect(await s.db.get("drafts", draft.id)).toBeUndefined();
    expect(await reopened.outgoing()).toEqual([]);
    expect(await reopened.recoverOutgoing(id, "return")).toEqual(recovered);
    await expect(reopened.saveDraft(draft)).rejects.toThrow("already reviewed");
    await expect(reopened.send(draft)).rejects.toThrow("not resent");
    expect(s.sends()).toBe(0);
  });
  it("cannot release an active SMTP operation even after a confirmed review", async () => {
    const s = setup();
    await s.repo.connect(account, "p", "p");
    s.mode("submitting");
    await expect(s.repo.send(draft)).rejects.toThrow("unconfirmed");
    for (const action of ["return", "mark", "local"] as const)
      await expect(s.repo.recoverOutgoing(id, action, true)).rejects.toThrow(
        "SMTP is still running",
      );
    expect(await s.repo.recoverOutgoing(id, "check")).toBeUndefined();
    expect(await s.db.get("drafts", draft.id)).toEqual(draft);
    expect(await s.repo.outgoing()).toHaveLength(1);
    expect(s.sends()).toBe(1);
  });
  it("requires explicit review for unknown or uncertain delivery and clones attachment ownership once", async () => {
    for (const state of ["uncertain", "unknown"]) {
      const s = setup();
      await s.repo.connect(account, "p", "p");
      await s.repo.saveDraft(draft);
      const attachments = await s.repo.addFiles(draft.id, [
        new File([new Uint8Array([0, 255, 1])], "original.bin"),
      ]);
      const original = { ...draft, attachments };
      s.mode("uncertain");
      await expect(s.repo.send(original)).rejects.toThrow("unconfirmed");
      s.mode(state);
      await expect(s.repo.recoverOutgoing(id, "return")).rejects.toThrow(
        "another send could create a duplicate",
      );
      await expect(s.repo.recoverOutgoing(id, "mark")).rejects.toThrow(
        "Confirm your delivery review",
      );
      const restored = (await s.repo.recoverOutgoing(id, "return", true))!;
      expect(restored.attachments).toHaveLength(1);
      expect(restored.attachments![0].id).not.toBe(attachments[0].id);
      const files = await s.db.all<any>("draftFiles");
      const recoveredFile = files.find((f) => f.draftId === restored.id);
      expect([
        ...new Uint8Array(await recoveredFile.blob.arrayBuffer()),
      ]).toEqual([0, 255, 1]);
      await s.repo.removeFile(restored.id, restored.attachments![0].id);
      expect(await s.repo.attachments(draft.id)).toEqual(attachments);
      await expect(s.repo.saveDraft(original)).rejects.toThrow(
        "already reviewed",
      );
      await expect(s.repo.send(original)).rejects.toThrow("already reviewed");
      const reopened = s.reopen();
      await reopened.load();
      expect(await reopened.recoverOutgoing(id, "return", true)).toMatchObject({
        id: restored.id,
        attachments: [],
      });
      expect(reopened.drafts).toHaveLength(1);
      expect(s.sends()).toBe(1);
    }
  });
  it("returns a rejection without implying delivery, and preserves the original on a failed recovery commit", async () => {
    const s = setup();
    await s.repo.connect(account, "p", "p");
    s.mode("rejected");
    await expect(s.repo.send(draft)).rejects.toThrow("not sent");
    const commit = s.db.commit.bind(s.db);
    let failRecovery = true;
    s.db.commit = async (changes) => {
      if (failRecovery && changes.some((c) => c.store === "drafts")) {
        failRecovery = false;
        throw Error("Synthetic recovery disk full");
      }
      await commit(changes);
    };
    await expect(s.repo.recoverOutgoing(id, "return")).rejects.toThrow(
      "disk full",
    );
    expect(await s.db.get("drafts", draft.id)).toEqual(draft);
    expect(await s.db.all("drafts")).toHaveLength(1);
    expect(await s.repo.outgoing()).toHaveLength(1);
    const recovered = await s.repo.recoverOutgoing(id, "return");
    expect(recovered?.body).toBe(draft.body);
    expect(await s.db.all("drafts")).toHaveLength(1);
    expect(s.sends()).toBe(1);
  });
  it("records a reviewed uncertain delivery without inventing a server acknowledgement or sending again", async () => {
    const s = setup();
    await s.repo.connect(account, "p", "p");
    s.mode("uncertain");
    await expect(s.repo.send(draft)).rejects.toThrow("unconfirmed");
    await s.repo.recoverOutgoing(id, "mark", true);
    const outgoing = await s.db.get<any>("outgoing", draft.id);
    expect(outgoing.state).toBe("uncertain");
    expect(outgoing.recovery).toEqual({ action: "marked" });
    expect(s.repo.cached[0].body).toBe(draft.body);
    expect(await s.db.get("raw", s.repo.cached[0].id)).toBe(outgoing.wire.raw);
    expect(await s.db.get("drafts", draft.id)).toBeUndefined();
    await expect(s.repo.send(draft)).rejects.toThrow("already reviewed");
    await expect(s.repo.saveDraft(draft)).rejects.toThrow("already reviewed");
    // Manual delivery review remains available for a separate provider Sent copy.
    expect(await s.repo.outgoing()).toHaveLength(1);
    await s.repo.recoverOutgoing(id, "local");
    expect(await s.repo.outgoing()).toEqual([]);
    expect(s.sends()).toBe(1);
  });
  it("repairs a lost delivery result and preserves newer Sent edits when choosing a local copy", async () => {
    const s = setup();
    await s.repo.connect(account, "p", "p");
    s.mode("lost");
    await expect(s.repo.send(draft)).rejects.toThrow("not confirmed");
    const reopened = s.reopen();
    await reopened.load();
    await reopened.recoverOutgoing(id, "check");
    expect(await s.db.get("drafts", draft.id)).toBeUndefined();
    const sent = reopened.cached[0];
    await reopened.mutate(sent.id, { starred: true });
    await reopened.mutate(sent.id, { folder: "Archive" });
    await reopened.recoverOutgoing(id, "local");
    expect(reopened.cached[0]).toMatchObject({
      id: sent.id,
      starred: true,
      folder: "Archive",
    });
    expect(await reopened.outgoing()).toEqual([]);
    expect(s.sends()).toBe(1);
  });
  it("retains older submissions without exact saved MIME instead of falsely recording a Sent copy", async () => {
    const s = setup();
    await s.db.commit([
      { store: "drafts", key: draft.id, value: draft },
      {
        store: "outgoing",
        key: draft.id,
        value: { id, draft, state: "uncertain" },
      },
    ]);
    s.mode("uncertain");
    await expect(s.repo.recoverOutgoing(id, "mark", true)).rejects.toThrow(
      "no exact saved MIME",
    );
    expect(await s.db.get("drafts", draft.id)).toEqual(draft);
    expect(await s.repo.outgoing()).toHaveLength(1);
    expect(s.repo.cached).toHaveLength(0);
    expect(s.sends()).toBe(0);
  });
});

describe("durable terminal delivery receipts", () => {
  it("keeps acknowledged delivery and rejection authoritative after server receipts expire", async () => {
    for (const state of ["delivered", "rejected"]) {
      const s = setup();
      await s.repo.connect(account, "p", "p");
      s.mode(state);
      if (state === "delivered") await s.repo.send(draft);
      else await expect(s.repo.send(draft)).rejects.toThrow("not sent");
      s.mode("unknown");
      const reopened = s.reopen();
      await reopened.load();
      const requests = s.requests.length;
      await reopened.recoverOutgoing("R".repeat(43), "check");
      expect((await reopened.outgoing())[0].state).toBe(state);
      if (state === "delivered") {
        await expect(
          reopened.recoverOutgoing("R".repeat(43), "return", true),
        ).rejects.toThrow("was delivered");
        expect(reopened.cached.some((m) => m.folder === "Sent")).toBe(true);
      } else {
        expect(
          await reopened.recoverOutgoing("R".repeat(43), "return"),
        ).toMatchObject({ body: draft.body });
      }
      expect(s.requests).toHaveLength(requests);
      expect(s.sends()).toBe(1);
    }
  });
});

it("a closed Gateway cannot recreate mailbox workers through a delayed caller", async () => {
  const db = new Memory();
  Object.assign(db, { profileId: session.user_id });
  const repo = new GatewayRepository(session, db);
  const mailbox = repo.mailbox!;
  repo.stopMailbox();
  await expect(
    mailbox.page({ scope: { folder: "Inbox" }, offset: 0 }),
  ).rejects.toThrow("closed");
  await expect(mailbox.detail("old")).rejects.toThrow("closed");
  await expect(mailbox.metadata("old")).rejects.toThrow("closed");
  await expect(mailbox.prefetch!("old")).rejects.toThrow("closed");
  await mailbox.close();
  // Node has no Worker: accidentally recreating one would fail this assertion.
  await expect(repo.mailbox!.detail("old")).rejects.toThrow("closed");
});
