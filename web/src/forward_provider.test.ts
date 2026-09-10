import { beforeAll, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { initSync } from "./wasm/shep_mail_content";
import { prepareForwardContent } from "./forward_content";
import { GatewayRepository, type Account, type Outgoing } from "./provider";
import { checkRemovedWrites } from "./account_removal";
import type { Draft } from "./model";
import type { Change, LocalStore, StoreName } from "./storage";
import cases from "../../shared/forward-fixtures.json";

beforeAll(() =>
  initSync({
    module: new WebAssembly.Module(
      readFileSync(
        new URL("./wasm/shep_mail_content_bg.wasm", import.meta.url),
      ),
    ),
  }),
);
class Memory implements LocalStore {
  data = new Map<string, unknown>();
  fail = false;
  lostAck = false;
  transactions: Change[][] = [];
  async all<T>(store: StoreName): Promise<T[]> {
    return structuredClone(
      [...this.data]
        .filter(([k]) => k.startsWith(store + ":"))
        .map(([, v]) => v),
    ) as T[];
  }
  async get<T>(store: StoreName, key: string): Promise<T | undefined> {
    return structuredClone(this.data.get(`${store}:${key}`)) as T | undefined;
  }
  async snapshot(names: readonly StoreName[]) {
    return Object.fromEntries(
      await Promise.all(names.map(async (n) => [n, await this.all(n)])),
    );
  }
  async submissions<T>(id: string): Promise<T[]> {
    return (await this.all<{ id: string }>("outgoing")).filter(
      (v) => v.id === id,
    ) as T[];
  }
  async commit(changes: Change[]) {
    if (this.fail) {
      this.fail = false;
      throw new Error("Synthetic disk full");
    }
    checkRemovedWrites(changes, await this.all("removedAccounts"));
    this.transactions.push(changes);
    for (const c of changes) {
      if (c.value === undefined) this.data.delete(`${c.store}:${c.key}`);
      else this.data.set(`${c.store}:${c.key}`, structuredClone(c.value));
    }
    if (this.lostAck) {
      this.lostAck = false;
      throw new Error("Synthetic lost commit acknowledgment");
    }
  }
}
const account: Account = {
  id: "fixture",
  name: "Forward fixture",
  email: "sender@example.test",
  protocol: "Pop3",
  host: "mail.example.test",
  port: 995,
  username: "fixture",
  incoming_security: "Tls",
  incoming_auth: "Password",
  smtp_host: "mail.example.test",
  smtp_port: 465,
  smtp_username: "fixture",
  smtp_security: "Tls",
  smtp_auth: "Automatic",
  smtp_separate_password: false,
  sent_copy: "LocalOnly",
  sent_folder: "Sent",
};
const session = {
  email: account.email,
  csrf: "C".repeat(43),
  user_id: "F".repeat(43),
};
const unlock = async <T>(_name: string, run: () => Promise<T>) => run();
async function setup() {
  const store = new Memory();
  await store.commit([{ store: "accounts", key: account.id, value: account }]);
  const raw = Buffer.from(cases[0].raw).toString("base64");
  const prepare = async () => ({
    ...(await prepareForwardContent(raw)),
    accountId: account.id,
  });
  const requests: { path: string; body: any }[] = [];
  const fetcher: typeof fetch = async (input, init) => {
    const path = String(input),
      body = init?.body ? JSON.parse(String(init.body)) : undefined;
    requests.push({ path, body });
    if (path.endsWith("/probe")) return Response.json({ connected: true });
    if (path.endsWith("/reserve"))
      return Response.json({ id: "R".repeat(43), state: "reserved" });
    throw new Error("Synthetic prepare refusal");
  };
  const repo = new GatewayRepository(session, store, fetcher, unlock, prepare);
  await repo.load();
  return { store, repo, prepare, requests, fetcher };
}
it("commits complete binary files atomically and retries a lost acknowledgment without replacing edits", async () => {
  const s = await setup();
  s.store.fail = true;
  await expect(s.repo.forward("source", "draft")).rejects.toThrow("disk full");
  expect(await s.store.all("drafts")).toEqual([]);
  expect(await s.store.all("draftFiles")).toEqual([]);
  s.store.lostAck = true;
  await expect(s.repo.forward("source", "draft")).rejects.toThrow(
    "lost commit",
  );
  expect(s.store.transactions.at(-1)?.map((c) => c.store)).toEqual([
    "drafts",
    "draftFiles",
    "draftFiles",
    "draftFiles",
  ]);
  const draft = (await s.store.get<Draft>("drafts", "draft"))!;
  expect(draft).toMatchObject({
    to: "",
    cc: "",
    bcc: "",
    subject: "Fwd: Café project",
  });
  expect(draft.inReplyTo).toBeUndefined();
  expect(draft.references).toBeUndefined();
  const bytes = await s.store.all<{ blob: Blob; info: { media_type: string } }>(
    "draftFiles",
  );
  expect([...new Uint8Array(await bytes[0].blob.arrayBuffer())]).toEqual([
    0, 255, 1, 13, 10,
  ]);
  expect(bytes[1].info.media_type).toBe("application/x-second");
  await s.repo.saveDraft({
    ...draft,
    body: "Newer edit",
    revision: 2,
    forward: null,
    attachments: [],
  });
  await s.repo.removeFile(draft.id, draft.attachments![0].id);
  const retry = await s.repo.forward("source", "draft");
  expect(retry.body).toBe("Newer edit");
  expect(retry.forward).toEqual(draft.forward);
  expect(retry.attachments).toHaveLength(2);
  expect(s.requests).toHaveLength(0);
  await expect(s.repo.forward("different-source", "draft")).rejects.toThrow(
    "another message",
  );
});
it("account removal after preparation prevents partial draft files from appearing", async () => {
  const s = await setup();
  let release!: () => void, started!: () => void;
  const waiting = new Promise<void>((r) => (started = r)),
    held = new Promise<void>((r) => (release = r));
  const repo = new GatewayRepository(
    session,
    s.store,
    s.fetcher,
    unlock,
    async () => {
      const value = await s.prepare();
      started();
      await held;
      return value;
    },
  );
  const work = repo.forward("source", "removed-draft");
  const refusal = expect(work).rejects.toThrow("removed");
  await waiting;
  await s.store.commit([
    { store: "accounts", key: account.id },
    {
      store: "removedAccounts",
      key: account.id,
      value: { id: account.id, draftIds: [], mailIds: [] },
    },
  ]);
  release();
  await refusal;
  expect(await s.store.all("drafts")).toEqual([]);
  expect(await s.store.all("draftFiles")).toEqual([]);
});
it("send carries retained HTML and inline identities; reviewed recovery clones their ownership", async () => {
  const s = await setup();
  const draft = await s.repo.forward("source", "draft");
  draft.to = "recipient@example.test";
  await s.repo.saveDraft(draft);
  await s.repo.connect(account, "synthetic", "synthetic");
  await expect(s.repo.send(draft)).rejects.toThrow();
  const prepared = s.requests.find((r) => r.path.endsWith("/prepare"))!.body;
  expect(prepared.draft.forward).toEqual(draft.forward);
  expect(prepared.draft.in_reply_to).toBeNull();
  expect(prepared.draft.references).toEqual([]);
  expect(prepared.files[2].content_id).toBe(draft.attachments![2].content_id);
  expect(Buffer.from(prepared.files[2].data, "base64").toString()).toBe(
    "inline fixture",
  );
  expect(s.requests.some((r) => r.path.endsWith("/send"))).toBe(false);
  const record = (await s.store.get<Outgoing>("outgoing", draft.id))!;
  record.state = "rejected";
  await s.store.commit([{ store: "outgoing", key: draft.id, value: record }]);
  const recovered = (await s.repo.recoverOutgoing(record.id, "return"))!;
  expect(recovered.id).not.toBe(draft.id);
  expect(recovered.forward).toEqual(draft.forward);
  expect(recovered.attachments![2].content_id).toBe(
    draft.attachments![2].content_id,
  );
  expect(recovered.attachments![2].id).not.toBe(draft.attachments![2].id);
  expect(await s.repo.recoverOutgoing(record.id, "return")).toEqual(recovered);
  await expect(s.repo.forward("source", draft.id)).rejects.toThrow(
    "delivery record",
  );
});
