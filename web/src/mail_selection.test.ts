import { afterEach, describe, expect, it } from "vitest";
import { MailSelection } from "./mail_selection";
import { PreviewSelection } from "./preview_selection";
import type { Mail } from "./model";
import type { SelectionCommand, SelectionScope } from "./selection_types";
const mail = (count = 125): Mail[] =>
  Array.from({ length: count }, (_, i) => ({
    id: `m${i.toString().padStart(3, "0")}`,
    sender: "Sender",
    address: "sender@example.test",
    subject: `Message ${i}`,
    preview: "Selection fixture",
    body: "Body",
    account: "Work",
    accountId: "work",
    folder: "Inbox",
    date: new Date(1000000 - i * 1000).toISOString(),
    unread: i % 2 === 0,
    starred: false,
    attachments: [],
  }));
function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((r) => (resolve = r));
  return { promise, resolve };
}
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
async function until(check: () => boolean) {
  for (let i = 0; !check(); i++) {
    if (i > 1000) throw Error("Selection did not settle");
    await tick();
  }
  await tick();
}
class Controlled {
  calls: SelectionCommand[] = [];
  held?: SelectionCommand["kind"];
  responseHeld?: SelectionCommand["kind"];
  fail?: SelectionCommand["kind"];
  lost?: SelectionCommand["kind"];
  gate = deferred();
  responseGate = deferred();
  active = 0;
  maxActive = 0;
  constructor(readonly memory: PreviewSelection) {}
  async selection(command: SelectionCommand, observed: string[] = []) {
    this.calls.push(command);
    this.maxActive = Math.max(this.maxActive, ++this.active);
    try {
      if (this.held === command.kind) {
        this.held = undefined;
        await this.gate.promise;
      }
      if (this.fail === command.kind) {
        this.fail = undefined;
        throw Error("Synthetic refusal");
      }
      const result = await this.memory.selection(command, observed);
      if (this.responseHeld === command.kind) {
        this.responseHeld = undefined;
        await this.responseGate.promise;
      }
      if (this.lost === command.kind) {
        this.lost = undefined;
        throw Error("Synthetic lost acknowledgment");
      }
      return result;
    } finally {
      this.active--;
    }
  }
}
const models: MailSelection[] = [];
afterEach(async () => {
  for (const m of models.splice(0)) m.dispose();
  await tick();
});
function setup(count = 125) {
  const rows = mail(count),
    memory = new PreviewSelection(() => rows),
    repo = new Controlled(memory);
  let scope: SelectionScope = { folder: "Inbox" };
  const model = new MailSelection(
    repo,
    () => scope,
    () => rows.filter((m) => m.folder === scope.folder).length,
    () => {},
  );
  model.setObserved(rows.slice(0, 50).map((m) => m.id));
  models.push(model);
  return {
    rows,
    memory,
    repo,
    model,
    scope: (next: SelectionScope) => {
      scope = next;
      model.reconcileScope();
    },
  };
}
const settled = (m: MailSelection) => until(() => !m.pending && !m.observing);
describe("captured selection controller", () => {
  it("acknowledges a range target before its newly rendered rank arrives", async () => {
    const { model: m, repo } = setup();
    m.toggle("m000");
    await settled(m);
    repo.held = "observe";
    m.setObserved(["m100"]);
    await until(() => !repo.held);
    m.range("m100");
    expect(m.selected("m100")).toBe(true);
    expect(m.pending).toBe(true);
    repo.gate.resolve();
    await settled(m);
    expect(m.count).toBe(101);
  });

  it("captures the whole query, observes pages, ranges and clears without leaving mode", async () => {
    const { model: m, repo, memory } = setup();
    m.toggle("m000");
    expect(m.selected("m000")).toBe(true);
    await settled(m);
    expect(m.count).toBe(1);
    m.all();
    expect(m.count).toBe(125);
    await settled(m);
    m.setObserved(["m100", "m124"]);
    await settled(m);
    m.range("m100");
    expect(m.count).toBe(101);
    await settled(m);
    expect(m.selected("m100")).toBe(true);
    expect(m.selected("m124")).toBe(false);
    m.clear();
    expect(m.count).toBe(0);
    expect(m.mode).toBe(true);
    await settled(m);
    m.done();
    await until(() => memory.captures.size === 0);
    expect(repo.maxActive).toBe(1);
  });
  it("retains offscreen deselection queued behind Select all", async () => {
    const { model: m, repo } = setup();
    m.start();
    await settled(m);
    repo.held = "capture";
    m.all();
    m.toggle("m000");
    expect(m.count).toBe(124);
    m.setObserved(["m100"]);
    expect(m.count).toBe(124);
    repo.gate.resolve();
    await settled(m);
    expect(m.count).toBe(124);
    m.setObserved(["m000", "m100"]);
    await settled(m);
    expect(m.selected("m000")).toBe(false);
    expect(m.selected("m100")).toBe(true);
  });
  it("keeps passive arrivals out until an explicit choice or recapture", async () => {
    const { model: m, rows } = setup();
    m.all();
    await settled(m);
    rows.push({ ...rows[0], id: "arrival" });
    m.setObserved(["m000", "arrival"]);
    m.refresh();
    await settled(m);
    expect(m.count).toBe(125);
    expect(m.selected("arrival")).toBe(false);
    m.toggle("arrival");
    expect(m.count).toBe(126);
    await settled(m);
    rows.push({ ...rows[0], id: "next" });
    m.all();
    await settled(m);
    expect(m.count).toBe(127);
  });
  it("rolls back a rejected gesture while preserving later queued input and Retry", async () => {
    const { model: m, repo } = setup();
    m.all();
    await settled(m);
    repo.held = "change";
    repo.fail = "change";
    m.toggle("m000");
    m.toggle("m001");
    expect(m.count).toBe(123);
    repo.gate.resolve();
    await until(() => !!m.error);
    expect(m.count).toBe(124);
    expect(m.selected("m000")).toBe(true);
    expect(m.selected("m001")).toBe(false);
    m.retry();
    expect(m.count).toBe(123);
    await settled(m);
    expect(m.count).toBe(123);
    expect(m.error).toBeUndefined();
  });
  it("recovers exactly committed revisions without repeating a lost change", async () => {
    const { model: m, repo } = setup();
    repo.lost = "capture";
    m.all();
    await settled(m);
    expect(m.count).toBe(125);
    repo.lost = "change";
    m.toggle("m000");
    await settled(m);
    expect(m.count).toBe(124);
    expect(repo.calls.filter((c) => c.kind === "change")).toHaveLength(1);
  });
  it("discards scope immediately and releases an in-flight capture before replacement", async () => {
    const { model: m, repo, memory, scope } = setup();
    repo.held = "capture";
    m.all();
    scope({ folder: "Archive" });
    expect(m.mode).toBe(false);
    expect(m.count).toBe(0);
    m.start();
    repo.gate.resolve();
    await settled(m);
    expect(m.snapshot?.total).toBe(0);
    expect(memory.captures.size).toBe(1);
    const kinds = repo.calls.map((c) => c.kind);
    expect(kinds.indexOf("release")).toBeLessThan(kinds.lastIndexOf("capture"));
  });
  it("caps pending gestures and preserves the anchor when a click overflows", async () => {
    const { model: m, repo } = setup();
    m.start();
    await settled(m);
    repo.held = "change";
    for (let i = 0; i < 32; i++) m.toggle(`m${i.toString().padStart(3, "0")}`);
    m.toggle("m040");
    expect(m.warning).toContain("catching up");
    expect(m.anchor).toBe("m031");
    repo.gate.resolve();
    await settled(m);
    expect(m.count).toBe(32);
    expect(repo.calls.filter((c) => c.kind === "change")).toHaveLength(32);
    expect(repo.maxActive).toBe(1);
  });
  it("re-observes refreshes arriving behind an older result, even with no rendered rows", async () => {
    const { model: m, repo, rows } = setup(2);
    m.all();
    await settled(m);
    repo.responseHeld = "observe";
    m.refresh();
    await until(() => !repo.responseHeld);
    rows.splice(0);
    m.setObserved([]);
    m.refresh();
    repo.responseGate.resolve();
    await settled(m);
    expect(m.count).toBe(2);
    expect(m.snapshot?.available).toBe(0);
    expect(repo.calls.filter((c) => c.kind === "observe")).toHaveLength(2);
  });
  it("retries abandoned capture release and preserves aliased choices", async () => {
    const { model: m, repo, rows, memory } = setup(2);
    m.toggle("m000");
    await settled(m);
    memory.aliases.set("m000", "adopted");
    rows[0] = { ...rows[0], id: "adopted" };
    m.refresh();
    await settled(m);
    expect(m.selected("adopted")).toBe(true);
    expect(m.count).toBe(1);
    repo.fail = "release";
    m.done();
    m.start();
    await until(() => !!m.error);
    m.retry();
    await settled(m);
    expect(m.count).toBe(0);
    expect(memory.captures.size).toBe(1);
  });
  it("rejects oversized observations and never lets a disposed result reopen mode", async () => {
    const { model: m, repo, memory } = setup();
    expect(() => m.setObserved(Array(51).fill("m000"))).toThrow(
      "one mail page",
    );
    repo.held = "capture";
    m.all();
    m.dispose();
    repo.gate.resolve();
    await until(() => memory.captures.size === 0);
    expect(m.mode).toBe(false);
    expect(m.snapshot).toBeUndefined();
  });
});
