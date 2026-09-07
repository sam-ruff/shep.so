import type { Account, Outgoing, RecordMail } from "./provider";
import type { Change, LocalStore } from "./storage";

export interface MailAlias {
  lineage?: string;
  targetLineage?: string;
  alias: string;
  target: string;
}
export interface MailRoles {
  account: string;
  discovered?: string | null;
  acknowledged: string[];
}
export const localId = (mail: RecordMail) => mail.localId ?? mail.core.id;
export async function resolveMail(store: LocalStore, id: string) {
  const alias = await store.get<MailAlias>("mailAliases", id);
  return store.get<RecordMail>("mail", alias?.target ?? id);
}
/** Caller holds the account cache lock; aliases always point directly at a row. */
export async function aliasChanges(
  store: LocalStore,
  old: string,
  target: string,
): Promise<Change[]> {
  if (old === target) return [];
  return [
    ...(await store.all<MailAlias>("mailAliases"))
      .filter((a) => a.target === old)
      .map((a) => ({
        store: "mailAliases" as const,
        key: a.alias,
        value: { ...a, target },
      })),
    { store: "mailAliases", key: old, value: { alias: old, target } },
  ];
}
export async function removeMailChanges(
  store: LocalStore,
  ids: Set<string>,
): Promise<Change[]> {
  return [
    ...[...ids].flatMap((key) => [
      { store: "mail" as const, key },
      { store: "raw" as const, key },
    ]),
    ...(await store.all<MailAlias>("mailAliases"))
      .filter((a) => ids.has(a.target))
      .map((a) => ({ store: "mailAliases" as const, key: a.alias })),
  ];
}
export function roleFolders(account: Account, roles?: MailRoles): Set<string> {
  return new Set(
    [
      account.sent_folder,
      roles?.discovered,
      ...(roles?.acknowledged ?? []),
    ].filter((f): f is string => !!f),
  );
}
export async function acknowledgeRole(
  store: LocalStore,
  account: string,
  folder: string,
) {
  const value = (await store.get<MailRoles>("mailRoles", account)) ?? {
    account,
    acknowledged: [],
  };
  value.acknowledged = [...new Set([...value.acknowledged, folder])];
  return { store: "mailRoles" as const, key: account, value };
}
function untouched(local: RecordMail, original: RecordMail, id: string) {
  return (
    local.local &&
    !local.localEdited &&
    !local.moved &&
    !local.pendingMove &&
    !local.receipt &&
    local.core.id === id &&
    localId(local) === id &&
    local.core.account_id === original.core.account_id &&
    local.core.remote_id === original.core.remote_id &&
    local.core.folder === "Sent" &&
    local.core.unread === original.core.unread &&
    local.core.starred === original.core.starred
  );
}
/** Build one transaction for insertion and optional identity adoption. Never
 * alter the outgoing journal's raw MIME, or merge an edited/moved local copy. */
export async function insertSentCandidate(
  store: LocalStore,
  incoming: RecordMail,
  raw: string,
  fallback?: RecordMail,
): Promise<Change[]> {
  const key = localId(incoming);
  const ordinary: Change[] = [
    { store: "mail", key, value: incoming },
    { store: "raw", key, value: raw },
  ];
  if (
    incoming.local ||
    !incoming.sentMessageId ||
    incoming.moved ||
    incoming.pendingMove ||
    incoming.receipt
  )
    return ordinary;
  const attempt = /^<([^<>\s]+)@shep\.so>$/.exec(incoming.sentMessageId)?.[1];
  if (!attempt) return ordinary;
  const matches = (await store.submissions<Outgoing>(attempt)).filter(
    (r) =>
      r.account?.id === incoming.core.account_id &&
      r.account.protocol === "Imap" &&
      r.mail &&
      r.sent?.state === "saved" &&
      r.sent.receipt?.folder === incoming.core.folder &&
      `<${r.id}@shep.so>` === incoming.sentMessageId,
  );
  if (matches.length !== 1) return ordinary;
  const saved = matches[0],
    original = saved.mail!;
  const id = `${incoming.core.account_id}:Sent:local-sent-${saved.id}`;
  if (
    id === key ||
    original.core.id !== id ||
    original.core.remote_id !== `local-sent-${saved.id}` ||
    (saved.sent!.receipt!.remote_id !== null &&
      saved.sent!.receipt!.remote_id !== incoming.core.remote_id)
  )
    return ordinary;
  const local = (await store.get<RecordMail>("mail", id)) ?? fallback;
  if (!local || !untouched(local, original, id)) return ordinary;
  return [
    { store: "mail", key },
    { store: "raw", key },
    ...(await aliasChanges(store, key, id)),
    { store: "mail", key: id, value: { ...incoming, localId: id } },
    { store: "raw", key: id, value: raw },
  ];
}
