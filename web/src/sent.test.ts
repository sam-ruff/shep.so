import { describe, it, expect } from "vitest";
import { GatewayRepository, type Account, type Outgoing } from "./provider";
import type { Change, LocalStore, StoreName } from "./storage";

class Memory implements LocalStore {
  data = new Map<string, unknown>();
  fail: ((changes: Change[]) => boolean) | undefined;
  async all<T>(store: StoreName) {
    return structuredClone(
      [...this.data]
        .filter(([k]) => k.startsWith(`${store}:`))
        .map(([, v]) => v),
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
    if (this.fail?.(changes)) {
      this.fail = undefined;
      throw new Error("Synthetic disk full");
    }
    for (const c of changes) {
      if (c.value === undefined) this.data.delete(`${c.store}:${c.key}`);
      else this.data.set(`${c.store}:${c.key}`, structuredClone(c.value));
    }
  }
}
const account: Account = {
  id: "sent-account",
  email: "owner@example.test",
  name: "Sent fixture",
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
  sent_folder: "",
};
const id = "S".repeat(43),
  draftId = "sent-draft",
  mailId = `${account.id}:Sent:local-sent-${id}`;
const sentReceipt = { folder: "Sent Mail", remote_id: "91.4" };
function record(): Outgoing {
  return {
    id,
    account: structuredClone(account),
    draft: {
      id: draftId,
      accountId: account.id,
      to: "peer@example.test",
      cc: "",
      bcc: "",
      subject: "Sent fixture",
      body: "Original bytes",
    },
    state: "delivered",
    sent: { state: "pending" },
    wire: {
      envelope: { from: account.email, to: ["peer@example.test"] },
      raw: btoa("Exact immutable MIME fixture"),
    },
    mail: {
      local: true,
      core: {
        id: mailId,
        account_id: account.id,
        folder: "Sent",
        remote_id: `local-sent-${id}`,
        sender: account.email,
        recipient: "peer@example.test",
        subject: "Sent fixture",
        preview: "Original bytes",
        timestamp: 1788696000,
        unread: false,
        starred: false,
        attachment_count: 0,
      },
      text: "Original bytes",
    },
  };
}
function response(value: unknown, status = 200) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });
}
async function fixture() {
  const store = new Memory();
  const initial = record();
  await store.commit([
    { store: "accounts", key: account.id, value: account },
    { store: "outgoing", key: draftId, value: initial },
  ]);
  const calls: { path: string; body: any }[] = [];
  let next = 0,
    uploads = 0,
    phase = "reserved",
    copyId = "",
    found = false,
    mode = "saved";
  const fetcher: typeof fetch = async (input, init) => {
    const path = String(input),
      body = init?.body ? JSON.parse(String(init.body)) : undefined;
    calls.push({ path, body });
    if (path.endsWith("/probe")) return response({ connected: true });
    if (path.endsWith("/sent/check"))
      return response({
        folder: "Sent Mail",
        receipt: found ? sentReceipt : null,
      });
    if (path.endsWith("/sent/reserve")) {
      if (
        !copyId ||
        (body.reviewed_retry &&
          ["unknown", "uncertain", "failed"].includes(phase))
      ) {
        copyId = String(++next).padStart(43, "C");
        phase = "reserved";
      }
      return response({ id: copyId, state: phase });
    }
    if (path.endsWith("/copy")) {
      const committed = await store.get<Outgoing>("outgoing", draftId);
      expect(committed?.sent?.state).toBe("copying");
      expect(committed?.sent?.copyId).toBe(copyId);
      expect(body.wire).toEqual(initial.wire);
      expect(body.connection.account.sent_folder).toBe("Sent Mail");
      expect(committed?.sent?.copyAccount).toEqual(body.connection.account);
      if (phase === "reserved") uploads++;
      phase = mode === "uncertain" ? "uncertain" : "saved";
      if (mode === "lost") throw new Error("Synthetic lost HTTP reply");
      return response({ id: copyId, state: phase, receipt: sentReceipt });
    }
    if (path === `/api/mail/sent/${copyId}`)
      return phase === "unknown"
        ? response({}, 404)
        : response({ id: copyId, state: phase, receipt: sentReceipt });
    throw new Error(`Unexpected transport ${path}`);
  };
  const locks = new Map<string, Promise<unknown>>();
  const lock = async <T>(name: string, fn: () => Promise<T>): Promise<T> => {
    const result = (locks.get(name) ?? Promise.resolve())
      .catch(() => {})
      .then(fn);
    locks.set(name, result);
    return result;
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
  await repo.connect(account, "synthetic-password", "synthetic-password");
  return {
    repo,
    store,
    calls,
    create,
    uploads: () => uploads,
    mode: (v: string) => (mode = v),
    phase: (v: string) => (phase = v),
    found: (v: boolean) => (found = v),
    read: () => store.get<Outgoing>("outgoing", draftId),
  };
}
describe("client-owned provider Sent recovery", () => {
  it("commits exact content and destination before copy, preserves local edits, and never resends SMTP", async () => {
    const s = await fixture();
    const local = record().mail!;
    local.core.starred = true;
    local.core.folder = "Archive";
    await s.store.commit([{ store: "mail", key: mailId, value: local }]);
    await s.repo.recoverSent(id, "copy");
    await s.repo.recoverSent(id, "copy");
    expect((await s.read())?.sent?.receipt).toEqual(sentReceipt);
    expect(s.uploads()).toBe(1);
    expect(await s.store.get("mail", mailId)).toEqual(local);
    expect(s.calls.some((c) => c.path.includes("/send"))).toBe(false);
  });
  it("recovers a lost upload response after reopening without credentials or another upload", async () => {
    const s = await fixture();
    s.mode("lost");
    await expect(s.repo.recoverSent(id, "copy")).rejects.toThrow(
      "not confirmed",
    );
    expect((await s.read())?.sent?.state).toBe("copying");
    const reopened = s.create();
    await reopened.load();
    await reopened.recoverSent(id, "check");
    expect((await s.read())?.sent?.state).toBe("saved");
    expect(s.uploads()).toBe(1);
  });
  it("retains an acknowledged receipt in memory through a failed browser commit", async () => {
    const s = await fixture();
    s.store.fail = (changes) =>
      changes.some(
        (c) =>
          c.store === "outgoing" &&
          (c.value as Outgoing)?.sent?.state === "saved",
      );
    await expect(s.repo.recoverSent(id, "copy")).rejects.toThrow(
      "server saved",
    );
    s.phase("unknown");
    s.repo.forgetPasswords();
    const count = s.calls.length;
    await s.repo.recoverSent(id, "check");
    expect(s.calls).toHaveLength(count);
    expect((await s.read())?.sent?.state).toBe("saved");
    expect(s.uploads()).toBe(1);
  });
  it("repairs a failed local cache write from the stored acknowledgment after reopening", async () => {
    const s = await fixture();
    s.store.fail = (changes) => changes.some((c) => c.store === "mail");
    await expect(s.repo.recoverSent(id, "copy")).rejects.toThrow();
    expect((await s.read())?.sent?.state).toBe("saved");
    const reopened = s.create();
    await reopened.load();
    const count = s.calls.length;
    await reopened.recoverSent(id, "check");
    expect(s.calls).toHaveLength(count);
    expect(await s.store.get("raw", mailId)).toBe(record().wire!.raw);
  });
  it("never starts APPEND before the browser commits its reserved identity and copying marker", async () => {
    for (const failAt of ["reserved", "copying"]) {
      const s = await fixture();
      s.store.fail = (changes) =>
        changes.some(
          (c) =>
            c.store === "outgoing" &&
            (c.value as Outgoing)?.sent?.state === failAt,
        );
      await expect(s.repo.recoverSent(id, "copy")).rejects.toThrow("disk full");
      expect(s.uploads()).toBe(0);
      await s.repo.recoverSent(id, "copy");
      expect(s.uploads()).toBe(1);
    }
  });
  it("requires explicit review after an uncertain or expired receipt, and lookup alone never uploads", async () => {
    for (const phase of ["uncertain", "unknown"]) {
      const s = await fixture();
      s.mode("uncertain");
      await expect(s.repo.recoverSent(id, "copy")).rejects.toThrow(
        "not confirmed",
      );
      s.phase(phase);
      await s.repo.recoverSent(id, "check");
      expect(s.uploads()).toBe(1);
      await expect(s.repo.recoverSent(id, "copy")).rejects.toThrow("confirm");
      expect(s.uploads()).toBe(1);
      s.mode("saved");
      await s.repo.recoverSent(id, "copy", true);
      expect(s.uploads()).toBe(2);
      expect((await s.read())?.sent?.state).toBe("saved");
    }
  });
  it("a matching Sent copy resolves uncertain delivery without changing its SMTP history", async () => {
    const s = await fixture();
    const original = record();
    original.state = "uncertain";
    await s.store.commit([
      { store: "outgoing", key: draftId, value: original },
    ]);
    await expect(s.repo.recoverSent(id, "copy", true)).rejects.toThrow(
      "Review delivery",
    );
    s.found(true);
    await s.repo.recoverSent(id, "check");
    expect(s.uploads()).toBe(0);
    expect((await s.read())?.state).toBe("uncertain");
    expect((await s.read())?.sent?.state).toBe("saved");
    await expect(s.repo.recoverOutgoing(id, "return", true)).rejects.toThrow(
      "acknowledged",
    );
    await s.repo.recoverOutgoing(id, "local");
    expect(await s.repo.outgoing()).toEqual([]);
  });
  it("does not use a changed incoming connection and keeps server-managed copies lookup-only", async () => {
    const s = await fixture();
    await s.repo.saveSentPreferences(account.id, "ServerManaged", "");
    await s.repo.recoverSent(id, "copy", false, true);
    expect(s.uploads()).toBe(0);
    expect((await s.read())?.sent?.state).toBe("missing");
    await s.store.commit([
      {
        store: "accounts",
        key: account.id,
        value: { ...account, host: "changed.example.test" },
      },
    ]);
    await expect(s.repo.recoverSent(id, "copy")).rejects.toThrow(
      "account changed",
    );
    expect(s.uploads()).toBe(0);
  });
  it("keeps POP3 and local-only automatic copies usable without passwords", async () => {
    for (const kind of ["Pop3", "LocalOnly"]) {
      const s = await fixture();
      const original = record();
      if (kind === "Pop3") original.account!.protocol = "Pop3";
      else original.account!.sent_copy = "LocalOnly";
      await s.store.commit([
        { store: "outgoing", key: draftId, value: original },
      ]);
      s.repo.forgetPasswords();
      const count = s.calls.length;
      await s.repo.recoverSent(id, "copy", false, true);
      expect((await s.read())?.sent?.state).toBe("local");
      expect(s.calls).toHaveLength(count);
    }
  });
  it("reconnect preserves newer Sent preferences and independent tabs share copy coordination", async () => {
    const s = await fixture();
    await s.repo.saveSentPreferences(account.id, "Automatic", "Sent Mail");
    await s.repo.connect(account, "new-password", "new-password");
    expect(s.repo.accounts[0].sent_folder).toBe("Sent Mail");
    const other = s.create();
    await other.load();
    await other.connect(s.repo.accounts[0], "password", "password");
    await Promise.all([
      s.repo.recoverSent(id, "copy"),
      other.recoverSent(id, "copy"),
    ]);
    expect(s.uploads()).toBe(1);
  });
});
