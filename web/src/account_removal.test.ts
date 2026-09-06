import { it, expect } from "vitest";
import {
  removalPreview,
  removalChanges,
  checkRemovedWrites,
  type RemovalSnapshot,
} from "./account_removal";
const snapshot = (): RemovalSnapshot => ({
  accounts: [
    { id: "a", email: "a@example.test" },
    { id: "b", email: "b@example.test" },
  ],
  mail: [
    { core: { id: "m", account_id: "a" }, pendingMove: { folder: "Archive" } },
  ],
  drafts: [{ id: "d", accountId: "a", body: "PRIVATE BODY", revision: 1 }],
  draftFiles: [
    {
      draftId: "d",
      info: { id: "f", size: 2 },
      blob: new Blob(["PRIVATE BINARY"]),
    },
  ],
  outgoing: [
    { id: "out", draft: { id: "d", accountId: "a" }, state: "uncertain" },
  ],
  mailAliases: [{ alias: "old", target: "m" }],
  mailRoles: [],
  removedAccounts: [],
});
it("reviews unfinished operations explicitly and retains only identities in the removal tombstone", () => {
  const s = snapshot(),
    review = removalPreview(s, "a");
  expect(review.unresolved).toBe(1);
  expect(review.moves).toBe(1);
  expect(review.files).toBe(1);
  expect(() => removalChanges(s, review, false)).toThrow("Confirm");
  const changes = removalChanges(s, review, true);
  expect(changes.some((c) => c.key === "b")).toBe(false);
  const removed = changes.find((c) => c.store === "removedAccounts")!
    .value as any;
  expect(JSON.stringify(removed)).not.toContain("PRIVATE");
  expect(removed.token).toBe(review.token);
  for (const c of [
    { store: "accounts" as const, key: "a", value: { id: "a" } },
    { store: "drafts" as const, key: "d", value: { accountId: "b" } },
    { store: "drafts" as const, key: "fresh", value: { accountId: "a" } },
    { store: "draftFiles" as const, key: "f", value: { draftId: "d" } },
    { store: "raw" as const, key: "m", value: "stale" },
    { store: "mailAliases" as const, key: "late", value: { target: "m" } },
  ])
    expect(() => checkRemovedWrites([c], [removed])).toThrow("removed");
  expect(() =>
    checkRemovedWrites(
      [{ store: "drafts", key: "fresh-other", value: { accountId: "b" } }],
      [removed],
    ),
  ).not.toThrow();
});
it("new mail, a changed draft or changed attachment ownership invalidates the open review", () => {
  for (const change of [
    (s: RemovalSnapshot) =>
      s.mail!.push({ core: { id: "new", account_id: "a" } }),
    (s: RemovalSnapshot) => s.drafts![0].revision++,
    (s: RemovalSnapshot) => s.draftFiles![0].info.size++,
  ]) {
    const s = snapshot(),
      review = removalPreview(s, "a");
    change(s);
    expect(() => removalChanges(s, review, true)).toThrow("Local data changed");
  }
});
