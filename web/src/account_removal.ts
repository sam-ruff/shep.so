import type { Account, Outgoing, RecordMail } from "./provider";
import type { Draft } from "./model";
import type { Change, StoreName } from "./storage";
import { localId, type MailAlias } from "./sent_cache";
export interface RemovalReview {
  id: string;
  token: string;
  email: string;
  messages: number;
  drafts: number;
  files: number;
  outgoing: number;
  unresolved: number;
  moves: number;
  fingerprint: string;
  draftIds: string[];
}
export type RemovalSnapshot = Partial<Record<StoreName, any[]>>;
export const reviewStores = [
  "accounts",
  "mail",
  "drafts",
  "draftFiles",
  "outgoing",
  "mailAliases",
  "mailRoles",
  "removedAccounts",
] as const;
const ordered = (items: any[], key: (v: any) => string) =>
  [...items].sort((a, b) => key(a).localeCompare(key(b)));
export function removalPreview(s: RemovalSnapshot, id: string): RemovalReview {
  const account = (s.accounts as Account[]).find((a) => a.id === id);
  if (!account)
    throw new Error("This account was removed. Reopen Preferences.");
  const mail = ordered(
    (s.mail as RecordMail[]).filter((m) => m.core.account_id === id),
    localId,
  );
  const drafts = ordered(
    (s.drafts as Draft[]).filter((d) => d.accountId === id),
    (d) => d.id,
  );
  const outgoing = ordered(
    (s.outgoing as Outgoing[]).filter(
      (o) => (o.account?.id ?? o.draft.accountId) === id,
    ),
    (o) => o.draft.id,
  );
  const draftIds = [
    ...new Set([
      ...drafts.map((d) => d.id),
      ...outgoing.map((o) => o.draft.id),
    ]),
  ].sort();
  const files = ordered(
    (s.draftFiles ?? []).filter((f) => draftIds.includes(f.draftId)),
    (f) => f.info.id,
  );
  return {
    id,
    token: crypto.randomUUID(),
    email: account.email,
    messages: mail.length,
    drafts: drafts.length,
    files: files.length,
    outgoing: outgoing.length,
    unresolved: outgoing.filter(
      (o) =>
        (!o.recovery &&
          !["delivered", "rejected", "cancelled"].includes(o.state)) ||
        (o.sent &&
          !["saved", "local"].includes(o.sent.state) &&
          (!o.recovery || o.recovery.action === "marked")),
    ).length,
    moves: mail.filter((m) => m.moved || m.pendingMove).length,
    draftIds,
    fingerprint: JSON.stringify([
      account,
      mail.map((m) => [localId(m), m.core, m.moved, m.pendingMove]),
      drafts,
      files.map((f) => [f.draftId, f.info, f.order]),
      outgoing.map((o) => [o.id, o.draft, o.state, o.recovery, o.sent]),
    ]),
  };
}
export function removalChanges(
  s: RemovalSnapshot,
  expected: RemovalReview,
  discard: boolean,
): Change[] {
  const previous = s.removedAccounts?.find((r) => r.id === expected.id);
  if (previous) {
    if (previous.token !== expected.token)
      throw new Error("This account removal changed. Reopen Preferences.");
    return [];
  }
  const current = removalPreview(s, expected.id);
  if (current.fingerprint !== expected.fingerprint)
    throw new Error(
      "Local data changed while this review was open. Reload the counts before removing this account.",
    );
  if (!discard && (current.unresolved || current.moves))
    throw new Error(
      "Confirm discarding unfinished delivery and move records first. Removal cannot undo a server operation.",
    );
  const mailIds = new Set(
    (s.mail as RecordMail[])
      .filter((m) => m.core.account_id === expected.id)
      .map(localId),
  );
  return [
    ...[...mailIds].flatMap((key) => [
      { store: "mail" as const, key },
      { store: "raw" as const, key },
    ]),
    ...(s.mailAliases as MailAlias[])
      .filter((a) => mailIds.has(a.target))
      .map((a) => ({ store: "mailAliases" as const, key: a.alias })),
    ...current.draftIds.flatMap((key) => [
      { store: "drafts" as const, key },
      { store: "outgoing" as const, key },
    ]),
    ...(s.draftFiles ?? [])
      .filter((f) => current.draftIds.includes(f.draftId))
      .map((f) => ({ store: "draftFiles" as const, key: f.info.id })),
    { store: "mailRoles", key: expected.id },
    { store: "accounts", key: expected.id },
    {
      store: "removedAccounts",
      key: expected.id,
      value: {
        id: expected.id,
        token: expected.token,
        draftIds: current.draftIds,
        mailIds: [...mailIds],
      },
    },
  ];
}
/** Invoked in the write transaction, so late tab/editor results cannot resurrect removal. */
export function checkRemovedWrites(changes: Change[], removed: any[]) {
  if (!removed.length) return;
  const ids = new Set(removed.map((r) => r.id));
  const drafts = new Set(removed.flatMap((r) => r.draftIds ?? []));
  const mail = new Set(removed.flatMap((r) => r.mailIds ?? []));
  for (const c of changes) {
    if (c.value === undefined) continue;
    const v: any = c.value;
    const owner =
      c.store === "accounts"
        ? c.key
        : c.store === "mail"
          ? v.core?.account_id
          : c.store === "drafts"
            ? v.accountId
            : c.store === "outgoing"
              ? (v.account?.id ?? v.draft?.accountId)
              : c.store === "mailRoles"
                ? v.account
                : undefined;
    if (
      ids.has(owner) ||
      (c.store === "raw" && mail.has(c.key)) ||
      (c.store === "mailAliases" && mail.has(v.target)) ||
      ((c.store === "drafts" || c.store === "outgoing") && drafts.has(c.key)) ||
      (c.store === "draftFiles" && drafts.has(v.draftId))
    )
      throw new Error(
        "This account was removed. Reopen Preferences or create a new draft with a connected account.",
      );
  }
}
