import { afterEach, expect, it, vi } from "vitest";
import { DraftSession } from "./draft_session";
import type { Draft } from "./model";
import {
  DraftConflict,
  observeDraft,
  type DraftObservation,
} from "./draft_revision";

const initial: Draft = {
  id: "draft",
  revision: 0,
  to: "",
  cc: "",
  bcc: "",
  subject: "Subject",
  body: "Saved text",
  forwardSource: "source",
  references: ["<original>"],
};
function deferred() {
  let resolve!: () => void, reject!: (error: Error) => void;
  const promise = new Promise<void>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
afterEach(() => vi.useRealTimers());

it("refreshes a clean reopened draft but preserves edits made during the read", () => {
  vi.useFakeTimers();
  const session = new DraftSession(initial, true, vi.fn());
  const expected = observeDraft(session.draft);
  const current = {
    ...initial,
    revision: 4,
    body: "Other editor",
    references: undefined,
  };
  expect(session.refreshSaved(current, expected)).toBe(true);
  expect(session.draft.references).toBeUndefined();
  const reopened = observeDraft(session.draft);
  session.draft.body = "Own newer text";
  session.edited();
  expect(session.refreshSaved({ ...current, revision: 6 }, reopened)).toBe(
    false,
  );
  expect(session.draft.body).toBe("Own newer text");
});

it("keeps conflicting text editable without blind retries until the saved revision is reviewed", async () => {
  const save = vi
    .fn<(draft: Draft, expected: DraftObservation | null) => Promise<void>>()
    .mockRejectedValueOnce(new DraftConflict())
    .mockResolvedValue(undefined);
  const session = new DraftSession(initial, true, save);
  session.draft.body = "My first edit";
  session.edited();
  expect(await session.flush()).toBe(false);
  session.draft.body = "My latest retained text";
  session.edited();
  expect(await session.flush(true)).toBe(false);
  expect(save).toHaveBeenCalledTimes(1);
  const current = { ...initial, body: "Other editor", revision: 5 };
  expect(session.acceptReview(current, 1, true)).toBe(false);
  expect(session.acceptReview(current, 2, true)).toBe(true);
  expect(session.draft.body).toBe("My latest retained text");
  expect(await session.flush()).toBe(true);
  expect(save.mock.calls[1]).toEqual([
    {
      ...current,
      body: "My latest retained text",
      revision: 6,
      attachments: [],
      forward: undefined,
      forwardSource: "source",
    },
    observeDraft(current),
  ]);
  expect(session.needsReview).toBe(false);
});

it("accepting reviewed saved text refreshes the base for the next edit", async () => {
  const save = vi
    .fn<(draft: Draft, expected: DraftObservation | null) => Promise<void>>()
    .mockResolvedValue(undefined);
  const session = new DraftSession(initial, true, save);
  session.recordConflict(new DraftConflict());
  expect(session.pending).toBe(true);
  const current = { ...initial, revision: 7, body: "Saved elsewhere" };
  expect(session.acceptReview(current, 0, false)).toBe(true);
  expect(session.pending).toBe(false);
  session.draft.body = "Continue from reviewed text";
  session.edited();
  expect(await session.flush()).toBe(true);
  expect(save.mock.calls[0][1]).toEqual(observeDraft(current));
  expect(session.draft.revision).toBe(8);
});

it("abandons a refused file request only after observing current saved attachments", async () => {
  const session = new DraftSession(initial, true, async () => {});
  const refused = vi.fn().mockRejectedValue(Error("Too many files"));
  expect(await session.changeFiles(refused)).toBe(false);
  const observed = [
    { id: "existing", name: "saved.txt", size: 3, media_type: "text/plain" },
  ];
  expect(
    await session.useSavedFiles(async () => {
      throw Error("Read failed");
    }),
  ).toBe(false);
  expect(session.filesPending).toBe(true);
  expect(await session.useSavedFiles(async () => observed)).toBe(true);
  expect(session.draft.attachments).toEqual(observed);
  expect(session.filesPending).toBe(false);
  expect(refused).toHaveBeenCalledTimes(1);
});

it("retiring a removed draft prevents late replies from republishing or saving newer queued text", async () => {
  const first = deferred();
  const save = vi
    .fn<(draft: Draft) => Promise<void>>()
    .mockReturnValue(first.promise);
  const changed = vi.fn();
  const session = new DraftSession(initial, true, save, changed);
  session.edited();
  const pending = session.flush();
  session.draft.body = "Later edit";
  session.edited();
  session.retire();
  const notifications = changed.mock.calls.length;
  first.resolve();
  expect(await pending).toBe(false);
  expect(await session.flush(true)).toBe(false);
  expect(save).toHaveBeenCalledTimes(1);
  expect(changed).toHaveBeenCalledTimes(notifications);
});

it("coalesces newer edits behind an owned save without replacing their text or metadata", async () => {
  const first = deferred(),
    second = deferred();
  const save = vi
    .fn<(draft: Draft) => Promise<void>>()
    .mockReturnValueOnce(first.promise)
    .mockReturnValueOnce(second.promise);
  const session = new DraftSession(initial, true, save);
  session.draft.body = "First";
  session.edited();
  const pending = session.flush();
  session.draft.body = "Newest";
  session.edited();
  first.resolve();
  await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(2));
  expect(session.draft.body).toBe("Newest");
  expect(session.status).toBe("Saving…");
  expect(save.mock.calls[1][0]).toMatchObject({
    body: "Newest",
    revision: 2,
    forwardSource: "source",
    references: ["<original>"],
  });
  second.resolve();
  expect(await pending).toBe(true);
  expect(session.status).toBe("Saved on this browser");
});

it("keeps an old failure visible until its newer owned revision succeeds", async () => {
  const first = deferred(),
    second = deferred();
  const save = vi
    .fn<(draft: Draft) => Promise<void>>()
    .mockReturnValueOnce(first.promise)
    .mockReturnValueOnce(second.promise);
  const session = new DraftSession(initial, true, save);
  session.edited();
  const pending = session.flush();
  session.draft.body = "Latest after switch";
  session.edited();
  first.reject(Error("Storage unavailable"));
  await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(2));
  expect(session.error).toBe("Storage unavailable");
  expect(session.draft.body).toBe("Latest after switch");
  second.resolve();
  await pending;
  expect(session.error).toBeUndefined();
});

it("retains a parked failure without automatic retry and saves exact latest text on explicit Retry", async () => {
  vi.useFakeTimers();
  const save = vi
    .fn<(draft: Draft) => Promise<void>>()
    .mockRejectedValueOnce(Error("Disk full"))
    .mockResolvedValue(undefined);
  const session = new DraftSession(initial, true, save);
  session.draft.body = "Retained";
  session.edited();
  expect(await session.flush()).toBe(false);
  await vi.advanceTimersByTimeAsync(5000);
  expect(await session.flush()).toBe(false);
  const other = new DraftSession({ ...initial, id: "other" }, true, save);
  expect(other.status).toBe("Saved on this browser");
  expect(session.status).toBe("Not saved");
  expect(save).toHaveBeenCalledTimes(1);
  expect(await session.flush(true)).toBe(true);
  expect(save.mock.calls[1][0].body).toBe("Retained");
});

it("new edits retry text while a failed attachment request remains owned until explicit recovery", async () => {
  const save = vi
    .fn<(draft: Draft) => Promise<void>>()
    .mockResolvedValue(undefined);
  const files = vi
    .fn()
    .mockRejectedValueOnce(Error("File storage full"))
    .mockResolvedValue([
      {
        id: "file",
        name: "exact.bin",
        size: 3,
        media_type: "application/octet-stream",
      },
    ]);
  const session = new DraftSession(initial, true, save);
  expect(await session.changeFiles(files)).toBe(false);
  session.draft.body = "Newer with file error";
  session.edited();
  expect(await session.flush()).toBe(false);
  expect(save.mock.calls[0][0].body).toBe("Newer with file error");
  expect(session.error).toBe("File storage full");
  expect(files).toHaveBeenCalledTimes(1);
  expect(await session.flush(true)).toBe(true);
  expect(files).toHaveBeenCalledTimes(2);
  expect(session.draft.attachments?.[0].id).toBe("file");
  expect(session.draft.forwardSource).toBe("source");
});

it("does not strand edits whose timer fired during a failed file write", async () => {
  vi.useFakeTimers();
  const file = deferred();
  const save = vi
    .fn<(draft: Draft) => Promise<void>>()
    .mockResolvedValue(undefined);
  const session = new DraftSession(initial, true, save);
  const pending = session.changeFiles(async () => {
    await file.promise;
    return [];
  });
  session.draft.body = "Edited while attaching";
  session.edited();
  await vi.advanceTimersByTimeAsync(501);
  file.reject(Error("Files failed"));
  expect(await pending).toBe(false);
  expect(save.mock.calls[0][0].body).toBe("Edited while attaching");
  expect(session.error).toBe("Files failed");
});
